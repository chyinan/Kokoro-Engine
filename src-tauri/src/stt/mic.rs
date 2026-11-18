// pattern: Imperative Shell
use crate::ai::context::AIOrchestrator;
use crate::ipc::events::SttMicAutoStop;
use crate::lifecycle::TaskScope;
use crate::stt::native_worker::{self, NativeWorker};
use crate::stt::stream::{AudioBuffer, SAMPLE_RATE};
use crate::stt::SttService;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use serde::Serialize;
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TrySendError};
#[cfg(test)]
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const VOLUME_EVENT_INTERVAL: Duration = Duration::from_millis(50);
const RESAMPLER_CHUNK_SIZE: usize = 256;
const SILERO_VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const SILERO_VAD_MODEL_NAME: &str = "silero_vad.onnx";

enum WorkerCommand {
    Start {
        auto_stop_on_silence: bool,
        only_when_chat_idle: bool,
        response: oneshot::Sender<Result<bool, String>>,
    },
    Stop {
        response: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Clone, Copy, Serialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "stt:mic-volume")]
pub(crate) struct MicVolumeEvent {
    volume: f32,
    rms: f32,
}

pub struct NativeMicState {
    worker: NativeWorker<WorkerCommand>,
}

impl NativeMicState {
    pub(crate) fn new(work: TaskScope, service: TaskScope) -> Self {
        Self {
            worker: NativeWorker::new("native microphone", work, service),
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.worker.shutdown().await
    }
}

struct NativeFrameProcessorHandle {
    cancel: CancellationToken,
    join: Option<std::thread::JoinHandle<()>>,
}

impl NativeFrameProcessorHandle {
    fn stop(mut self) -> Result<(), String> {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| "Native microphone frame processor panicked".to_string())?;
        }
        Ok(())
    }
}

impl Drop for NativeFrameProcessorHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
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
                .map_err(|err| format!("Failed to create resampler: {err}"))?,
            )
        };

        Ok(Self {
            channels,
            resampler,
            pending_mono: Vec::with_capacity(RESAMPLER_CHUNK_SIZE * 2),
        })
    }

    fn process_input<T>(&mut self, input: &[T]) -> Option<NativeAudioFrame>
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let mono = interleaved_to_mono(input, self.channels);
        if mono.is_empty() {
            return None;
        }

        let rms =
            (mono.iter().map(|sample| sample * sample).sum::<f32>() / mono.len() as f32).sqrt();
        let db = if rms > 0.0 { 20.0 * rms.log10() } else { -60.0 };
        let volume = ((db + 60.0) * 2.0).clamp(0.0, 100.0);

        let output = match self.resampler.as_mut() {
            Some(resampler) => resample_mono_chunk(&mut self.pending_mono, resampler, mono),
            None => mono,
        };

        if output.is_empty() {
            return None;
        }

        Some(NativeAudioFrame {
            samples: output,
            volume,
            rms,
        })
    }
}

struct NativeAudioFrame {
    samples: Vec<f32>,
    volume: f32,
    rms: f32,
}

struct NativeFrameProcessor {
    app: AppHandle,
    vad: Option<VoiceActivityDetector>,
    auto_stop_emitted: bool,
    last_volume_emit: Instant,
    vad_detected_logged: bool,
    vad_frame_counter: usize,
}

impl NativeFrameProcessor {
    fn new(
        app: AppHandle,
        auto_stop_on_silence: bool,
        stop: &CancellationToken,
    ) -> Result<Self, String> {
        let vad = if auto_stop_on_silence {
            Some(create_voice_activity_detector(stop)?)
        } else {
            None
        };

        Ok(Self {
            app,
            vad,
            auto_stop_emitted: false,
            last_volume_emit: Instant::now() - VOLUME_EVENT_INTERVAL,
            vad_detected_logged: false,
            vad_frame_counter: 0,
        })
    }

    fn process_frame(&mut self, frame: NativeAudioFrame) {
        if self.last_volume_emit.elapsed() >= VOLUME_EVENT_INTERVAL {
            let _ = MicVolumeEvent {
                volume: frame.volume,
                rms: frame.rms,
            }
            .emit(&self.app);
            self.last_volume_emit = Instant::now();
        }

        if let Some(vad) = self.vad.as_ref() {
            self.vad_frame_counter += 1;
            vad.accept_waveform(&frame.samples);
            if !self.vad_detected_logged && vad.detected() {
                self.vad_detected_logged = true;
            }
            if !self.auto_stop_emitted && !vad.is_empty() {
                self.auto_stop_emitted = true;
                let _ = SttMicAutoStop.emit(&self.app);
            }
        }

        let audio_buffer = self.app.state::<AudioBuffer>();
        if let Err(err) = audio_buffer.append_samples(frame.samples) {
            tracing::error!(target: "stt", "[STT] Native mic append failed: {err}");
        }
    }
}

pub(crate) async fn start_native_mic_with_options(
    app: &AppHandle,
    mic_state: &NativeMicState,
    auto_stop_on_silence: bool,
    only_when_chat_idle: bool,
) -> Result<bool, String> {
    let _lease = mic_state.worker.admit("start")?;
    let app = app.clone();
    let tx = mic_state
        .worker
        .commands(move |rx, stop| run_mic_worker(rx, app, stop))?;
    let (response, result) = oneshot::channel();
    tx.send(WorkerCommand::Start {
        auto_stop_on_silence,
        only_when_chat_idle,
        response,
    })
    .map_err(|_| "Native microphone worker is unavailable".to_string())?;
    native_worker::response(result, "Native microphone").await
}

pub(crate) async fn stop_native_mic(mic_state: &NativeMicState) -> Result<(), String> {
    let Ok(_lease) = mic_state.worker.admit("stop") else {
        return mic_state.worker.confirm_app_stop().await;
    };
    let Some(tx) = mic_state.worker.running_commands() else {
        return Ok(());
    };
    let (response, result) = oneshot::channel();
    tx.send(WorkerCommand::Stop { response })
        .map_err(|_| "Native microphone worker is unavailable".to_string())?;
    native_worker::response(result, "Native microphone").await
}

fn start_interrupt(
    state: &AIOrchestrator,
    stt: &SttService,
    start: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    stt.with_interrupt_enabled(|| state.start_if_chat_idle(|| start().map(|()| true)))
}

fn run_mic_worker(
    rx: Receiver<WorkerCommand>,
    app: AppHandle,
    stop: CancellationToken,
) -> Result<(), String> {
    let mut stream: Option<(Stream, NativeFrameProcessorHandle)> = None;

    while let Some(command) = native_worker::receive(&rx, &stop) {
        match command {
            WorkerCommand::Start {
                auto_stop_on_silence,
                only_when_chat_idle,
                response,
            } => {
                if response.is_closed() {
                    continue;
                }
                if stream.is_some() {
                    let _ = response.send(Ok(!only_when_chat_idle));
                    continue;
                }
                let result = (|| {
                    let state = only_when_chat_idle.then(|| app.state::<AIOrchestrator>());
                    if state.as_ref().is_some_and(|state| state.is_chat_busy()) {
                        return Ok(false);
                    }
                    let stt = only_when_chat_idle.then(|| app.state::<SttService>());
                    if let Some(stt) = &stt {
                        if !stt.with_interrupt_enabled(|| Ok(true))? {
                            return Ok(false);
                        }
                    }
                    let (new_stream, processor) =
                        build_native_input_stream(&app, auto_stop_on_silence, stop.child_token())?;
                    if stop.is_cancelled() || response.is_closed() {
                        return Ok(false);
                    }
                    let play = || {
                        if stop.is_cancelled() || response.is_closed() {
                            return Err("Native microphone start was cancelled".into());
                        }
                        new_stream
                            .play()
                            .map_err(|error| format!("Failed to start microphone stream: {error}"))
                    };
                    let started = match (state, stt) {
                        (Some(state), Some(stt)) => start_interrupt(&state, &stt, || {
                            app.state::<AudioBuffer>().discard()?;
                            play()
                        })?,
                        _ => {
                            play()?;
                            true
                        }
                    };
                    if started {
                        stream = Some((new_stream, processor));
                    }
                    Ok(started)
                })();
                if let Err(Ok(true)) = response.send(result) {
                    // The caller timed out while the device was initializing.
                    drop_mic_stream(&mut stream)?;
                }
            }
            WorkerCommand::Stop { response } => {
                let stopped = drop_mic_stream(&mut stream);
                let _ = MicVolumeEvent {
                    volume: 0.0,
                    rms: 0.0,
                }
                .emit(&app);
                let _ = response.send(stopped);
            }
        }
    }
    drop_mic_stream(&mut stream)
}

fn drop_mic_stream(
    stream: &mut Option<(Stream, NativeFrameProcessorHandle)>,
) -> Result<(), String> {
    if let Some((input, frames)) = stream.take() {
        frames.cancel.cancel();
        drop(input);
        frames.stop()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::SttConfig;
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn interrupt_cannot_start_while_an_unannounced_chat_owns_execution() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let stt = SttService::init_from_config(&SttConfig {
            auto_send: true,
            ..SttConfig::default()
        })
        .await;
        let pet_turn = state.try_acquire_chat_turn("pet-before-ack").await.unwrap();
        let started = tokio::task::spawn_blocking(move || {
            start_interrupt(&state, &stt, || panic!("microphone must not start"))
        })
        .await
        .unwrap()
        .unwrap();
        assert!(!started);
        drop(pet_turn);
    }

    #[tokio::test]
    async fn interrupt_rechecks_stt_settings_after_device_preparation() {
        let state = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stt_config.json");
        let enabled = SttConfig {
            auto_send: true,
            ..SttConfig::default()
        };
        let stt = SttService::init_from_config(&enabled).await;
        let mut providers_off = enabled.clone();
        for provider in &mut providers_off.providers {
            provider.enabled = false;
        }

        for disabled in [
            SttConfig {
                auto_send: false,
                ..enabled.clone()
            },
            providers_off,
        ] {
            stt.save_and_reload(&path, enabled.clone()).await.unwrap();
            let (preparing, preparation_started) = oneshot::channel();
            let (finish_setup, prepared) = mpsc::sync_channel(1);
            let mic_state = state.clone();
            let mic_stt = stt.clone();
            let mic = tokio::task::spawn_blocking(move || {
                assert!(mic_stt.with_interrupt_enabled(|| Ok(true)).unwrap());
                preparing.send(()).unwrap();
                prepared.recv_timeout(Duration::from_secs(5)).unwrap();
                start_interrupt(&mic_state, &mic_stt, || {
                    panic!("settings now forbid capture")
                })
            });
            preparation_started.await.unwrap();
            stt.save_and_reload(&path, disabled).await.unwrap();
            finish_setup.send(()).unwrap();
            assert!(!mic.await.unwrap().unwrap());
        }
    }

    #[tokio::test]
    async fn pet_chat_arriving_during_mic_start_waits_and_is_admitted() {
        let state = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let stt = SttService::init_from_config(&SttConfig {
            auto_send: true,
            ..SttConfig::default()
        })
        .await;
        let (playing, play_started) = oneshot::channel();
        let (finish_play, played) = mpsc::sync_channel(1);
        let mic_state = state.clone();
        let mic = tokio::task::spawn_blocking(move || {
            start_interrupt(&mic_state, &stt, || {
                playing.send(()).unwrap();
                played.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
        });
        play_started.await.unwrap();
        assert!(!state.is_chat_busy());
        let mut pet_turn = std::pin::pin!(state.try_acquire_chat_turn("pet-during-mic-start"));
        poll_fn(|cx| {
            assert!(pet_turn.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        finish_play.send(()).unwrap();
        assert!(mic.await.unwrap().unwrap());
        let guard = pet_turn.await.unwrap();
        assert_eq!(
            guard.context().unwrap().client_request_id,
            "pet-during-mic-start"
        );
    }
}

fn build_native_input_stream(
    app: &AppHandle,
    auto_stop_on_silence: bool,
    stop: CancellationToken,
) -> Result<(Stream, NativeFrameProcessorHandle), String> {
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

    let result = match config.sample_format() {
        SampleFormat::I8 => build_input_stream::<i8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::I64 => build_input_stream::<i64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::U32 => build_input_stream::<u32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::U64 => build_input_stream::<u64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        SampleFormat::F64 => build_input_stream::<f64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
            stop,
        ),
        sample_format => Err(format!(
            "Unsupported microphone sample format: {sample_format}"
        )),
    };
    result
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: u32,
    app: AppHandle,
    auto_stop_on_silence: bool,
    stop: CancellationToken,
) -> Result<(Stream, NativeFrameProcessorHandle), String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let mut processor = NativeInputProcessor::new(sample_rate, channels)?;
    let err_app = app.clone();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<NativeAudioFrame>(8);
    let error_stop = stop.clone();
    let callback_stop = stop.clone();

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
                            tracing::error!(target: "stt", "[STT] Native mic frame dropped: processor is lagging");
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            tracing::error!(target: "stt", "[STT] Native mic frame processor disconnected");
                        }
                    }
                }
            },
            move |err| {
                if error_stop.is_cancelled() {
                    return;
                }
                tracing::error!(target: "stt", "[STT] Native microphone stream error: {err}");
                let _ = MicVolumeEvent { volume: 0.0, rms: 0.0 }.emit(&err_app);
            },
            None,
        )
        .map_err(|err| format!("Failed to build microphone input stream: {err}"))?;
    let frame_processor = spawn_frame_processor(app, frame_rx, auto_stop_on_silence, stop)?;
    Ok((stream, frame_processor))
}

fn spawn_frame_processor(
    app: AppHandle,
    frame_rx: Receiver<NativeAudioFrame>,
    auto_stop_on_silence: bool,
    cancel: CancellationToken,
) -> Result<NativeFrameProcessorHandle, String> {
    let (initialized, ready) = mpsc::sync_channel(1);
    let cancel_worker = cancel.clone();
    let frames = std::thread::Builder::new()
        .name("microphone-frames".into())
        .spawn(move || {
            let mut processor =
                match NativeFrameProcessor::new(app, auto_stop_on_silence, &cancel_worker) {
                    Ok(processor) => {
                        let _ = initialized.send(Ok(()));
                        processor
                    }
                    Err(error) => {
                        let _ = initialized.send(Err(error));
                        return;
                    }
                };
            while let Some(frame) = native_worker::receive(&frame_rx, &cancel_worker) {
                processor.process_frame(frame);
            }
        })
        .map_err(|error| format!("Could not start native microphone frame processor: {error}"))?;
    match ready.recv() {
        Ok(Ok(())) => Ok(NativeFrameProcessorHandle {
            cancel,
            join: Some(frames),
        }),
        result => {
            let _ = frames.join();
            Err(result
                .ok()
                .and_then(Result::err)
                .unwrap_or_else(|| "Native microphone frame processor exited during setup".into()))
        }
    }
}

pub(crate) fn create_voice_activity_detector(
    stop: &CancellationToken,
) -> Result<VoiceActivityDetector, String> {
    static MODEL_SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _setup = MODEL_SETUP.lock().unwrap();
    let model_path = ensure_silero_vad_model(stop)?;
    match create_voice_activity_detector_from_path(&model_path) {
        Ok(detector) => Ok(detector),
        Err(first_error) => {
            tracing::warn!(
                target: "stt",
                "[STT] Cached Silero VAD model failed validation; retrying download: {first_error}"
            );
            let _ = fs::remove_file(&model_path);
            let refreshed_path = ensure_silero_vad_model(stop)?;
            create_voice_activity_detector_from_path(&refreshed_path)
                .map_err(|second_error| format!("{first_error}; retry failed: {second_error}"))
        }
    }
}

fn create_voice_activity_detector_from_path(
    model_path: &Path,
) -> Result<VoiceActivityDetector, String> {
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(model_path.to_string_lossy().into_owned()),
            threshold: 0.5,
            min_silence_duration: 0.8,
            min_speech_duration: 0.25,
            window_size: 512,
            max_speech_duration: 20.0,
        },
        sample_rate: SAMPLE_RATE as i32,
        num_threads: 1,
        provider: None,
        debug: false,
        ..VadModelConfig::default()
    };

    VoiceActivityDetector::create(&config, 30.0)
        .ok_or_else(|| "failed to create sherpa-onnx voice activity detector".to_string())
}

fn ensure_silero_vad_model(stop: &CancellationToken) -> Result<std::path::PathBuf, String> {
    if stop.is_cancelled() {
        return Err("Silero VAD model setup was cancelled".into());
    }
    let app_data_dir = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("stt")
        .join("vad");
    fs::create_dir_all(&app_data_dir).map_err(|err| err.to_string())?;
    let model_path = app_data_dir.join(SILERO_VAD_MODEL_NAME);
    if model_path.is_file() {
        return Ok(model_path);
    }

    let download = async {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|err| format!("Failed to build Silero VAD HTTP client: {err}"))?
            .get(SILERO_VAD_MODEL_URL)
            .send()
            .await
            .map_err(|err| format!("Failed to download silero VAD model: {err}"))?
            .error_for_status()
            .map_err(|err| format!("Silero VAD model request failed: {err}"))?
            .bytes()
            .await
            .map_err(|err| format!("Failed to read silero VAD model bytes: {err}"))
    };
    let bytes = tauri::async_runtime::block_on(async {
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err("Silero VAD model download was cancelled".into()),
            result = download => result,
        }
    })?;

    let temporary = model_path.with_file_name(format!(
        ".{}.{}.download",
        SILERO_VAD_MODEL_NAME,
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|err| format!("failed to create temporary Silero VAD model: {err}"))?;
        std::io::Write::write_all(&mut file, &bytes)
            .map_err(|err| format!("failed to write Silero VAD model: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("failed to sync Silero VAD model: {err}"))?;
        crate::config::atomic_replace_file(&temporary, &model_path)
            .map_err(|err| format!("failed to install Silero VAD model: {err}"))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(model_path)
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
                tracing::error!(target: "stt", "[STT] Native microphone resampling failed: {err}");
                break;
            }
        }
        pending_mono.drain(..needed);
    }

    output
}
