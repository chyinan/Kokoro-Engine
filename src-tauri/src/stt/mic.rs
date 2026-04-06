use crate::stt::stream::{AudioBuffer, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use serde::Serialize;
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::fs;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const VOLUME_EVENT_INTERVAL: Duration = Duration::from_millis(50);
const RESAMPLER_CHUNK_SIZE: usize = 256;
const SILERO_VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const SILERO_VAD_MODEL_NAME: &str = "silero_vad.onnx";

enum WorkerCommand {
    Start {
        auto_stop_on_silence: bool,
        response: SyncSender<Result<(), String>>,
    },
    Stop {
        response: SyncSender<Result<(), String>>,
    },
}

#[derive(Clone, Copy, Serialize)]
struct MicVolumeEvent {
    volume: f32,
    rms: f32,
}

#[derive(Default)]
pub struct NativeMicState {
    control_tx: Mutex<Option<Sender<WorkerCommand>>>,
}

impl NativeMicState {
    pub fn new() -> Self {
        Self {
            control_tx: Mutex::new(None),
        }
    }

    fn ensure_worker(&self, app: &AppHandle) -> Result<Sender<WorkerCommand>, String> {
        let mut guard = self
            .control_tx
            .lock()
            .map_err(|_| "Failed to lock native microphone state".to_string())?;

        if let Some(tx) = guard.as_ref() {
            return Ok(tx.clone());
        }

        let (tx, rx) = mpsc::channel();
        spawn_mic_worker(rx, app.clone());
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
    fn new(app: AppHandle, auto_stop_on_silence: bool) -> Result<Self, String> {
        let vad = if auto_stop_on_silence {
            Some(create_voice_activity_detector()?)
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
            let _ = self.app.emit(
                "stt:mic-volume",
                MicVolumeEvent {
                    volume: frame.volume,
                    rms: frame.rms,
                },
            );
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
                let _ = self.app.emit("stt:mic-auto-stop", ());
            }
        }

        let audio_buffer = self.app.state::<AudioBuffer>();
        if let Err(err) = audio_buffer.append_samples(frame.samples) {
            tracing::error!(target: "stt", "[STT] Native mic append failed: {err}");
        }
    }
}

pub fn start_native_mic(app: &AppHandle, mic_state: &NativeMicState) -> Result<(), String> {
    let tx = mic_state.ensure_worker(app)?;
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(WorkerCommand::Start {
        auto_stop_on_silence: false,
        response: response_tx,
    })
    .map_err(|_| "Native microphone worker is unavailable".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "Native microphone worker did not respond".to_string())?
}

pub fn start_native_mic_with_options(
    app: &AppHandle,
    mic_state: &NativeMicState,
    auto_stop_on_silence: bool,
) -> Result<(), String> {
    let tx = mic_state.ensure_worker(app)?;
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(WorkerCommand::Start {
        auto_stop_on_silence,
        response: response_tx,
    })
    .map_err(|_| "Native microphone worker is unavailable".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "Native microphone worker did not respond".to_string())?
}

pub fn stop_native_mic(app: &AppHandle, mic_state: &NativeMicState) -> Result<(), String> {
    let tx = mic_state.ensure_worker(app)?;
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(WorkerCommand::Stop {
        response: response_tx,
    })
    .map_err(|_| "Native microphone worker is unavailable".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "Native microphone worker did not respond".to_string())?
}

fn spawn_mic_worker(rx: Receiver<WorkerCommand>, app: AppHandle) {
    std::thread::spawn(move || {
        let mut stream: Option<Stream> = None;

        while let Ok(command) = rx.recv() {
            match command {
                WorkerCommand::Start {
                    auto_stop_on_silence,
                    response,
                } => {
                    let result = if stream.is_some() {
                        Ok(())
                    } else {
                        match build_native_input_stream(&app, auto_stop_on_silence) {
                            Ok(new_stream) => {
                                if let Err(err) = new_stream.play() {
                                    Err(format!("Failed to start microphone stream: {err}"))
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
                    let _ = app.emit(
                        "stt:mic-volume",
                        MicVolumeEvent {
                            volume: 0.0,
                            rms: 0.0,
                        },
                    );
                    let _ = response.send(Ok(()));
                }
            }
        }
    });
}

fn build_native_input_stream(
    app: &AppHandle,
    auto_stop_on_silence: bool,
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
            auto_stop_on_silence,
        ),
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::I64 => build_input_stream::<i64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::U32 => build_input_stream::<u32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::U64 => build_input_stream::<u64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        SampleFormat::F64 => build_input_stream::<f64>(
            &device,
            &stream_config,
            channels,
            sample_rate,
            app_handle,
            auto_stop_on_silence,
        ),
        sample_format => Err(format!(
            "Unsupported microphone sample format: {sample_format}"
        )),
    }
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: u32,
    app: AppHandle,
    auto_stop_on_silence: bool,
) -> Result<Stream, String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let mut processor = NativeInputProcessor::new(sample_rate, channels)?;
    let err_app = app.clone();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<NativeAudioFrame>(8);
    spawn_frame_processor(app.clone(), frame_rx, auto_stop_on_silence)?;

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
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
                tracing::error!(target: "stt", "[STT] Native microphone stream error: {err}");
                let _ = err_app.emit(
                    "stt:mic-volume",
                    MicVolumeEvent {
                        volume: 0.0,
                        rms: 0.0,
                    },
                );
            },
            None,
        )
        .map_err(|err| format!("Failed to build microphone input stream: {err}"))
}

fn spawn_frame_processor(
    app: AppHandle,
    frame_rx: Receiver<NativeAudioFrame>,
    auto_stop_on_silence: bool,
) -> Result<(), String> {
    if auto_stop_on_silence {
        let _ = create_voice_activity_detector()?;
    }
    std::thread::spawn(move || {
        let mut processor = match NativeFrameProcessor::new(app, auto_stop_on_silence) {
            Ok(processor) => processor,
            Err(err) => {
                tracing::error!(target: "stt", "[STT] Native mic frame processor init failed: {err}");
                return;
            }
        };
        while let Ok(frame) = frame_rx.recv() {
            processor.process_frame(frame);
        }
    });
    Ok(())
}

pub(crate) fn create_voice_activity_detector() -> Result<VoiceActivityDetector, String> {
    let model_path = ensure_silero_vad_model()?;
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

    let detector = VoiceActivityDetector::create(&config, 30.0)
        .ok_or_else(|| "Failed to create sherpa-onnx voice activity detector".to_string())?;
    Ok(detector)
}

fn ensure_silero_vad_model() -> Result<std::path::PathBuf, String> {
    let app_data_dir = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("stt")
        .join("vad");
    fs::create_dir_all(&app_data_dir).map_err(|err| err.to_string())?;
    let model_path = app_data_dir.join(SILERO_VAD_MODEL_NAME);
    if model_path.is_file() {
        return Ok(model_path);
    }

    let bytes = tauri::async_runtime::block_on(async {
        reqwest::get(SILERO_VAD_MODEL_URL)
            .await
            .map_err(|err| format!("Failed to download silero VAD model: {err}"))?
            .bytes()
            .await
            .map_err(|err| format!("Failed to read silero VAD model bytes: {err}"))
    })?;

    fs::write(&model_path, &bytes).map_err(|err| err.to_string())?;
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
