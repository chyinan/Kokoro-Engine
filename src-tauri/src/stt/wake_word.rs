use crate::stt::mic::create_voice_activity_detector;
use crate::stt::stream::SAMPLE_RATE;
use crate::stt::{AudioChunk, AudioSource, SttService};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc as tokio_mpsc;

const DETECTION_COOLDOWN: Duration = Duration::from_secs(2);
const RESAMPLER_CHUNK_SIZE: usize = 256;

enum WorkerCommand {
    Start {
        wake_word: String,
        trigger_on_speech: bool,
        response: SyncSender<Result<(), String>>,
    },
    Stop {
        response: SyncSender<Result<(), String>>,
    },
}

#[derive(Default)]
pub struct NativeWakeWordState {
    control_tx: Mutex<Option<Sender<WorkerCommand>>>,
}

impl NativeWakeWordState {
    pub fn new() -> Self {
        Self {
            control_tx: Mutex::new(None),
        }
    }

    fn ensure_worker(&self, app: &AppHandle) -> Result<Sender<WorkerCommand>, String> {
        let mut guard = self
            .control_tx
            .lock()
            .map_err(|_| "Failed to lock wake word state".to_string())?;

        if let Some(tx) = guard.as_ref() {
            return Ok(tx.clone());
        }

        let (tx, rx) = mpsc::channel();
        spawn_wake_word_worker(rx, app.clone());
        *guard = Some(tx.clone());
        Ok(tx)
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
    fn new(segment_tx: tokio_mpsc::Sender<Vec<f32>>) -> Result<Self, String> {
        Ok(Self {
            segment_tx,
            vad_logged: false,
            frame_counter: 0,
            detector: create_voice_activity_detector()?,
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
                    eprintln!("[WakeWord][native] segment dropped: transcription is busy");
                }
                Err(tokio_mpsc::error::TrySendError::Closed(_)) => {
                    eprintln!("[WakeWord][native] transcription worker disconnected");
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
}

impl WakeWordTranscriber {
    fn new(app: AppHandle, wake_word: String, trigger_on_speech: bool) -> Self {
        Self {
            app,
            wake_word_normalized: normalize_text(&wake_word),
            trigger_on_speech,
            last_detection_at: None,
        }
    }

    async fn check_segment(&mut self, samples: Vec<f32>) -> Result<(), String> {
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

        if self.trigger_on_speech {
            let text = result.text.trim().to_string();
            if !text.is_empty() {
                self.last_detection_at = Some(Instant::now());
                let _ = self.app.emit("stt:wake-word-detected", text);
            }
            return Ok(());
        }

        if self.wake_word_normalized.is_empty() {
            return Ok(());
        }

        let normalized = normalize_text(&result.text);
        if normalized.contains(&self.wake_word_normalized) {
            self.last_detection_at = Some(Instant::now());
            let _ = self.app.emit("stt:wake-word-detected", result.text);
        }

        Ok(())
    }
}

pub fn start_native_wake_word(
    app: &AppHandle,
    wake_word_state: &NativeWakeWordState,
    wake_word: String,
    trigger_on_speech: bool,
) -> Result<(), String> {
    let tx = wake_word_state.ensure_worker(app)?;
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(WorkerCommand::Start {
        wake_word,
        trigger_on_speech,
        response: response_tx,
    })
    .map_err(|_| "Native wake word worker is unavailable".to_string())?;

    response_rx
        .recv()
        .map_err(|_| "Native wake word worker did not respond".to_string())?
}

pub fn stop_native_wake_word(
    app: &AppHandle,
    wake_word_state: &NativeWakeWordState,
) -> Result<(), String> {
    let tx = wake_word_state.ensure_worker(app)?;
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(WorkerCommand::Stop {
        response: response_tx,
    })
    .map_err(|_| "Native wake word worker is unavailable".to_string())?;

    response_rx
        .recv()
        .map_err(|_| "Native wake word worker did not respond".to_string())?
}

fn spawn_wake_word_worker(rx: Receiver<WorkerCommand>, app: AppHandle) {
    std::thread::spawn(move || {
        let mut stream: Option<Stream> = None;

        while let Ok(command) = rx.recv() {
            match command {
                WorkerCommand::Start {
                    wake_word,
                    trigger_on_speech,
                    response,
                } => {
                    let result = if stream.is_some() {
                        Ok(())
                    } else {
                        match build_native_wake_word_stream(&app, wake_word, trigger_on_speech) {
                            Ok(new_stream) => {
                                if let Err(err) = new_stream.play() {
                                    Err(format!("Failed to start wake word microphone: {err}"))
                                } else {
                                    stream = Some(new_stream);
                                    Ok(())
                                }
                            }
                            Err(err) => Err(err),
                        }
                    };
                    let _ = response.send(result);
                }
                WorkerCommand::Stop { response } => {
                    stream.take();
                    let _ = response.send(Ok(()));
                }
            }
        }
    });
}

fn build_native_wake_word_stream(
    app: &AppHandle,
    wake_word: String,
    trigger_on_speech: bool,
) -> Result<Stream, String> {
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
        ),
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::I64 => build_input_stream::<i64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::U32 => build_input_stream::<u32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::U64 => build_input_stream::<u64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        SampleFormat::F64 => build_input_stream::<f64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            wake_word,
            trigger_on_speech,
        ),
        sample_format => Err(format!(
            "Unsupported wake word microphone sample format: {sample_format}"
        )),
    }
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: u32,
    app: AppHandle,
    wake_word: String,
    trigger_on_speech: bool,
) -> Result<Stream, String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let mut processor = NativeInputProcessor::new(sample_rate, channels)?;
    let err_app = app.clone();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<Vec<f32>>(32);
    spawn_frame_processor(app, frame_rx, wake_word, trigger_on_speech)?;

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if let Some(frame) = processor.process_input(data) {
                    match frame_tx.try_send(frame) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            eprintln!("[WakeWord][native] frame dropped: processor is lagging");
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            eprintln!("[WakeWord][native] frame processor disconnected");
                        }
                    }
                }
            },
            move |err| {
                eprintln!("[WakeWord][native] microphone stream error: {err}");
                let _ = err_app.emit(
                    "stt:wake-word-error",
                    format!("Native wake word stream error: {err}"),
                );
            },
            None,
        )
        .map_err(|err| format!("Failed to build wake word microphone stream: {err}"))
}

fn spawn_frame_processor(
    app: AppHandle,
    frame_rx: Receiver<Vec<f32>>,
    wake_word: String,
    trigger_on_speech: bool,
) -> Result<(), String> {
    let _ = create_voice_activity_detector()?;
    let (segment_tx, segment_rx) = tokio_mpsc::channel::<Vec<f32>>(1);
    spawn_transcription_worker(app.clone(), segment_rx, wake_word, trigger_on_speech);

    std::thread::spawn(move || {
        let mut processor = match WakeWordFrameProcessor::new(segment_tx) {
            Ok(processor) => processor,
            Err(err) => {
                eprintln!("[WakeWord][native] frame processor init failed: {err}");
                return;
            }
        };

        while let Ok(frame) = frame_rx.recv() {
            processor.process_frame(frame);
        }
    });

    Ok(())
}

fn spawn_transcription_worker(
    app: AppHandle,
    mut segment_rx: tokio_mpsc::Receiver<Vec<f32>>,
    wake_word: String,
    trigger_on_speech: bool,
) {
    tauri::async_runtime::spawn(async move {
        let mut transcriber = WakeWordTranscriber::new(app, wake_word, trigger_on_speech);
        while let Some(segment) = segment_rx.recv().await {
            if let Err(err) = transcriber.check_segment(segment).await {
                eprintln!("[WakeWord][native] Segment transcription failed: {err}");
            }
        }
    });
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
                eprintln!("[WakeWord][native] resampling failed: {err}");
                break;
            }
        }
        pending_mono.drain(..needed);
    }

    output
}
