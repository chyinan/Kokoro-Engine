use crate::ipc::events::{SttWakeWordDetected, SttWakeWordError};
use crate::lifecycle::TaskScope;
use crate::stt::mic::create_voice_activity_detector;
use crate::stt::native_worker::{self, NativeWorker};
use crate::stt::stream::SAMPLE_RATE;
use crate::stt::{AudioChunk, AudioSource, SttService};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use std::sync::mpsc::{self, Receiver, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::{
    runtime::Handle,
    sync::{mpsc as tokio_mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

const DETECTION_COOLDOWN: Duration = Duration::from_secs(2);
const RESAMPLER_CHUNK_SIZE: usize = 256;

enum WorkerCommand {
    Start {
        wake_word: String,
        trigger_on_speech: bool,
        owner: Option<u64>,
        response: oneshot::Sender<Result<(), String>>,
    },
    Stop {
        owner: Option<u64>,
        response: oneshot::Sender<Result<(), String>>,
    },
}

pub struct NativeWakeWordState {
    worker: NativeWorker<WorkerCommand>,
}

impl NativeWakeWordState {
    pub(crate) fn new(work: TaskScope, service: TaskScope) -> Self {
        Self {
            worker: NativeWorker::new("native wake word microphone and recognition", work, service),
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.worker.shutdown().await
    }
}

struct NativeInputProcessor {
    channels: usize,
    resampler: Option<FastFixedIn<f32>>,
    pending_mono: Vec<f32>,
}

impl NativeInputProcessor {
    fn new(sample_rate: u32, channels: usize) -> Result<Self, String> {
        let resampler = if sample_rate == SAMPLE_RATE {
            None
        } else {
            Some(
                FastFixedIn::<f32>::new(
                    SAMPLE_RATE as f64 / sample_rate as f64,
                    1.0,
                    PolynomialDegree::Cubic,
                    RESAMPLER_CHUNK_SIZE,
                    1,
                )
                .map_err(|err| format!("Failed to create wake word resampler: {err}"))?,
            )
        };

        Ok(Self {
            channels,
            resampler,
            pending_mono: Vec::with_capacity(RESAMPLER_CHUNK_SIZE * 2),
        })
    }

    fn process_input<T>(&mut self, input: &[T]) -> Option<Vec<f32>>
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let mono = interleaved_to_mono(input, self.channels);
        if mono.is_empty() {
            return None;
        }

        let output = match self.resampler.as_mut() {
            Some(resampler) => resample_mono_chunk(&mut self.pending_mono, resampler, mono),
            None => mono,
        };

        if output.is_empty() {
            return None;
        }

        Some(output)
    }
}

struct WakeWordFrameProcessor {
    segment_tx: tokio_mpsc::Sender<Vec<f32>>,
    vad_logged: bool,
    frame_counter: usize,
    detector: sherpa_onnx::VoiceActivityDetector,
}

impl WakeWordFrameProcessor {
    fn new(
        segment_tx: tokio_mpsc::Sender<Vec<f32>>,
        stop: &CancellationToken,
    ) -> Result<Self, String> {
        Ok(Self {
            segment_tx,
            vad_logged: false,
            frame_counter: 0,
            detector: create_voice_activity_detector(stop)?,
        })
    }

    fn process_frame(&mut self, samples: Vec<f32>) {
        self.frame_counter += 1;
        self.detector.accept_waveform(&samples);

        if !self.vad_logged && self.detector.detected() {
            self.vad_logged = true;
        }

        while let Some(segment) = self.detector.front() {
            let clip = segment.samples().to_vec();
            self.detector.pop();
            self.vad_logged = false;
            self.frame_counter = 0;

            if clip.is_empty() {
                continue;
            }

            match self.segment_tx.try_send(clip) {
                Ok(()) => {}
                Err(tokio_mpsc::error::TrySendError::Full(_)) => {
                    tracing::error!(target: "stt", "[WakeWord][native] segment dropped: transcription is busy");
                }
                Err(tokio_mpsc::error::TrySendError::Closed(_)) => {
                    tracing::error!(target: "stt", "[WakeWord][native] transcription worker disconnected");
                }
            }
        }
    }
}

struct WakeWordTranscriber {
    app: AppHandle,
    wake_word_normalized: String,
    trigger_on_speech: bool,
    last_detection_at: Option<Instant>,
    stop: CancellationToken,
}

impl WakeWordTranscriber {
    fn new(
        app: AppHandle,
        wake_word: String,
        trigger_on_speech: bool,
        stop: CancellationToken,
    ) -> Self {
        Self {
            app,
            wake_word_normalized: normalize_text(&wake_word),
            trigger_on_speech,
            last_detection_at: None,
            stop,
        }
    }

    async fn check_segment(&mut self, samples: Vec<f32>) -> Result<(), String> {
        if self.stop.is_cancelled() {
            return Ok(());
        }
        if self
            .last_detection_at
            .is_some_and(|last| last.elapsed() < DETECTION_COOLDOWN)
        {
            return Ok(());
        }

        let stt_service = self.app.state::<SttService>().inner().clone();
        let chunk = AudioChunk {
            samples: Arc::new(samples),
            sample_rate: SAMPLE_RATE,
        };

        let result = stt_service
            .transcribe(&AudioSource::Chunk(chunk), None)
            .await
            .map_err(|err| err.to_string())?;

        // A stop or replacement may have happened while transcription was in
        // flight; never emit a detection for an obsolete native owner.
        if self.stop.is_cancelled() {
            return Ok(());
        }

        if self.trigger_on_speech {
            let text = result.text.trim().to_string();
            if !text.is_empty() {
                self.last_detection_at = Some(Instant::now());
                let _ = SttWakeWordDetected(text).emit(&self.app);
            }
            return Ok(());
        }

        if self.wake_word_normalized.is_empty() {
            return Ok(());
        }

        let normalized = normalize_text(&result.text);
        if normalized.contains(&self.wake_word_normalized) {
            self.last_detection_at = Some(Instant::now());
            let _ = SttWakeWordDetected(result.text).emit(&self.app);
        }

        Ok(())
    }
}

pub(crate) async fn start_native_wake_word(
    app: &AppHandle,
    state: &NativeWakeWordState,
    wake_word: String,
    trigger_on_speech: bool,
    owner: Option<u64>,
) -> Result<(), String> {
    let _lease = state.worker.admit("start")?;
    let app = app.clone();
    let tx = state
        .worker
        .commands(move |rx, stop| run_wake_word_worker(rx, app, stop))?;
    let (response, result) = oneshot::channel();
    tx.send(WorkerCommand::Start {
        wake_word,
        trigger_on_speech,
        owner,
        response,
    })
    .map_err(|_| "Native wake word worker is unavailable".to_string())?;
    native_worker::response(result, "Native wake word").await
}

pub(crate) async fn stop_native_wake_word(
    state: &NativeWakeWordState,
    owner: Option<u64>,
) -> Result<(), String> {
    let Ok(_lease) = state.worker.admit("stop") else {
        return state.worker.confirm_app_stop().await;
    };
    let Some(tx) = state.worker.running_commands() else {
        return Ok(());
    };
    let (response, result) = oneshot::channel();
    tx.send(WorkerCommand::Stop { owner, response })
        .map_err(|_| "Native wake word worker is unavailable".to_string())?;
    native_worker::response(result, "Native wake word").await
}

fn run_wake_word_worker(
    rx: Receiver<WorkerCommand>,
    app: AppHandle,
    stop: CancellationToken,
) -> Result<(), String> {
    let mut capture: Option<(Stream, WakeWordTasks, u64)> = None;
    while let Some(command) = native_worker::receive(&rx, &stop) {
        match command {
            WorkerCommand::Start {
                wake_word,
                trigger_on_speech,
                owner,
                response,
            } => {
                if response.is_closed() {
                    continue;
                }
                let owner = owner.unwrap_or(0);
                if capture
                    .as_ref()
                    .is_some_and(|(_, _, current)| *current == owner)
                {
                    let _ = response.send(Ok(()));
                    continue;
                }
                let result = (|| {
                    close_capture(&mut capture)?;
                    if stop.is_cancelled() || response.is_closed() {
                        return Err("Native wake word start was cancelled".into());
                    }
                    let (stream, tasks) = build_native_wake_word_stream(
                        &app,
                        wake_word,
                        trigger_on_speech,
                        stop.child_token(),
                    )?;
                    if stop.is_cancelled() || response.is_closed() {
                        drop(stream);
                        tasks.stop()?;
                        return Err("Native wake word start was cancelled".into());
                    }
                    stream.play().map_err(|error| {
                        format!("Failed to start wake word microphone: {error}")
                    })?;
                    capture = Some((stream, tasks, owner));
                    Ok(())
                })();
                if let Err(Ok(())) = response.send(result) {
                    close_capture(&mut capture)?;
                }
            }
            WorkerCommand::Stop { owner, response } => {
                let result = if owner.is_none()
                    || capture
                        .as_ref()
                        .is_some_and(|(_, _, current)| owner == Some(*current))
                {
                    close_capture(&mut capture)
                } else {
                    Ok(())
                };
                let _ = response.send(result);
            }
        }
    }
    close_capture(&mut capture)
}

fn close_capture(capture: &mut Option<(Stream, WakeWordTasks, u64)>) -> Result<(), String> {
    if let Some((stream, tasks, _)) = capture.take() {
        tasks.cancel.cancel();
        drop(stream);
        tasks.stop()?;
    }
    Ok(())
}

fn build_native_wake_word_stream(
    app: &AppHandle,
    wake_word: String,
    trigger_on_speech: bool,
    stop: CancellationToken,
) -> Result<(Stream, WakeWordTasks), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No input microphone device is available".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|err| format!("Failed to query default microphone config: {err}"))?;

    let stream_config: StreamConfig = config.clone().into();
    let channels = stream_config.channels as usize;
    let sample_rate = stream_config.sample_rate.0;
    let app_handle = app.clone();

    match config.sample_format() {
        SampleFormat::I8 => build_input_stream::<i8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::I64 => build_input_stream::<i64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::U32 => build_input_stream::<u32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::U64 => build_input_stream::<u64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        SampleFormat::F64 => build_input_stream::<f64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
            stop,
        ),
        sample_format => Err(format!(
            "Unsupported wake word microphone sample format: {sample_format}"
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: u32,
    app: AppHandle,
    wake_word: String,
    trigger_on_speech: bool,
    stop: CancellationToken,
) -> Result<(Stream, WakeWordTasks), String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let mut processor = NativeInputProcessor::new(sample_rate, channels)?;
    let err_app = app.clone();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<Vec<f32>>(32);
    let callback_stop = stop.clone();
    let error_stop = stop.clone();
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if callback_stop.is_cancelled() {
                    return;
                }
                if let Some(frame) = processor.process_input(data) {
                    match frame_tx.try_send(frame) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            tracing::error!(target: "stt", "[WakeWord][native] frame dropped: processor is lagging");
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            tracing::error!(target: "stt", "[WakeWord][native] frame processor disconnected");
                        }
                    }
                }
            },
            move |err| {
                if error_stop.is_cancelled() {
                    return;
                }
                tracing::error!(target: "stt", "[WakeWord][native] microphone stream error: {err}");
                let _ = SttWakeWordError(format!("Native wake word stream error: {err}")).emit(&err_app);
            },
            None,
        )
        .map_err(|err| format!("Failed to build wake word microphone stream: {err}"))?;
    let tasks = spawn_frame_processor(app, frame_rx, wake_word, trigger_on_speech, stop)?;
    Ok((stream, tasks))
}

struct WakeWordTasks {
    cancel: CancellationToken,
    frames: Option<std::thread::JoinHandle<()>>,
    transcription: Option<tokio::task::JoinHandle<()>>,
    runtime: Handle,
}

impl WakeWordTasks {
    fn stop(mut self) -> Result<(), String> {
        self.join()
    }

    fn join(&mut self) -> Result<(), String> {
        self.cancel.cancel();
        let frames = self.frames.take().map(|thread| thread.join());
        let transcription = self
            .transcription
            .take()
            .map(|task| self.runtime.block_on(task));
        if frames.is_some_and(|outcome| outcome.is_err()) {
            return Err("Native wake word frame processor panicked".into());
        }
        if let Some(Err(error)) = transcription {
            return Err(format!(
                "Native wake word transcription worker failed: {error}"
            ));
        }
        Ok(())
    }
}

impl Drop for WakeWordTasks {
    fn drop(&mut self) {
        if let Err(error) = self.join() {
            tracing::error!(target: "stt", %error, "Could not confirm native wake word worker completion");
        }
    }
}

fn spawn_frame_processor(
    app: AppHandle,
    frame_rx: Receiver<Vec<f32>>,
    wake_word: String,
    trigger_on_speech: bool,
    stop: CancellationToken,
) -> Result<WakeWordTasks, String> {
    let (segment_tx, mut segment_rx) = tokio_mpsc::channel::<Vec<f32>>(1);
    let (initialized, ready) = mpsc::sync_channel(1);
    let frame_stop = stop.clone();
    let frames = std::thread::Builder::new()
        .name("wake-word-frames".into())
        .spawn(move || {
            let mut processor = match WakeWordFrameProcessor::new(segment_tx, &frame_stop) {
                Ok(processor) => {
                    let _ = initialized.send(Ok(()));
                    processor
                }
                Err(error) => {
                    let _ = initialized.send(Err(error));
                    return;
                }
            };
            while let Some(frame) = native_worker::receive(&frame_rx, &frame_stop) {
                processor.process_frame(frame);
            }
        })
        .map_err(|error| format!("Could not start native wake word frame processor: {error}"))?;

    match ready.recv() {
        Ok(Ok(())) => {}
        result => {
            let _ = frames.join();
            return Err(result
                .ok()
                .and_then(Result::err)
                .unwrap_or_else(|| "Native wake word frame processor exited during setup".into()));
        }
    }
    let runtime = Handle::current();
    let transcriber_stop = stop.clone();
    let transcription = runtime.spawn(async move {
        let mut transcriber =
            WakeWordTranscriber::new(app, wake_word, trigger_on_speech, transcriber_stop.clone());
        while let Some(segment) = tokio::select! {
            biased;
            _ = transcriber_stop.cancelled() => None,
            next = segment_rx.recv() => next,
        } {
            if let Err(error) = transcriber.check_segment(segment).await {
                tracing::error!(target: "stt", %error, "Native wake word transcription failed");
            }
        }
    });
    Ok(WakeWordTasks {
        cancel: stop,
        frames: Some(frames),
        transcription: Some(transcription),
        runtime,
    })
}

fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn interleaved_to_mono<T>(input: &[T], channels: usize) -> Vec<f32>
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels <= 1 {
        return input
            .iter()
            .map(|&sample| f32::from_sample(sample))
            .collect();
    }

    input
        .chunks_exact(channels)
        .map(|frame| {
            let sum = frame
                .iter()
                .map(|&sample| f32::from_sample(sample))
                .sum::<f32>();
            sum / channels as f32
        })
        .collect()
}

fn resample_mono_chunk(
    pending_mono: &mut Vec<f32>,
    resampler: &mut FastFixedIn<f32>,
    mono: Vec<f32>,
) -> Vec<f32> {
    pending_mono.extend_from_slice(&mono);

    let mut output = Vec::new();
    loop {
        let needed = resampler.input_frames_next();
        if pending_mono.len() < needed {
            break;
        }

        let input_chunk = vec![pending_mono[..needed].to_vec()];
        match resampler.process(&input_chunk, None) {
            Ok(mut processed) => {
                if let Some(channel) = processed.pop() {
                    output.extend(channel);
                }
            }
            Err(err) => {
                tracing::error!(target: "stt", "[WakeWord][native] resampling failed: {err}");
                break;
            }
        }
        pending_mono.drain(..needed);
    }

    output
}
