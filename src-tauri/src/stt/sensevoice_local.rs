use super::config::SttProviderConfig;
use super::interface::{
    AudioSource, SttEngine, SttError, TranscriptionResult, TranscriptionSegment,
};
use async_trait::async_trait;
use bzip2::read::BzDecoder;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::PathBuf;
use tar::Archive;

const APP_DIR_NAME: &str = "com.chyin.kokoro";
const RECOMMENDED_MODEL_ID: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
const RECOMMENDED_ARCHIVE_NAME: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2";
const RECOMMENDED_DOWNLOAD_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenseVoiceLocalModelStatus {
    pub installed: bool,
    pub download_instructions_url: String,
    pub recommended_model_id: String,
    pub download_url: String,
    pub install_dir: String,
    pub model_path: String,
    pub tokens_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenseVoiceLocalDownloadProgress {
    pub stage: String,
    pub message: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

pub fn app_data_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
}

fn recommended_root_dir() -> PathBuf {
    app_data_dir()
        .join("stt")
        .join("sensevoice_local")
        .join(RECOMMENDED_MODEL_ID)
}

fn recommended_extract_dir() -> PathBuf {
    recommended_root_dir().join(RECOMMENDED_MODEL_ID)
}

fn recommended_archive_path() -> PathBuf {
    recommended_root_dir().join(RECOMMENDED_ARCHIVE_NAME)
}

fn recommended_model_path() -> PathBuf {
    recommended_extract_dir().join("model.int8.onnx")
}

fn recommended_tokens_path() -> PathBuf {
    recommended_extract_dir().join("tokens.txt")
}

pub fn recommended_model_status() -> SenseVoiceLocalModelStatus {
    let model_path = recommended_model_path();
    let tokens_path = recommended_tokens_path();

    SenseVoiceLocalModelStatus {
        installed: model_path.is_file() && tokens_path.is_file(),
        download_instructions_url: "https://k2-fsa.github.io/sherpa/onnx/sense-voice/export.html"
            .to_string(),
        recommended_model_id: RECOMMENDED_MODEL_ID.to_string(),
        download_url: RECOMMENDED_DOWNLOAD_URL.to_string(),
        install_dir: recommended_extract_dir().to_string_lossy().into_owned(),
        model_path: model_path.to_string_lossy().into_owned(),
        tokens_path: tokens_path.to_string_lossy().into_owned(),
    }
}

pub async fn download_recommended_model<F>(
    mut emit_progress: F,
) -> Result<SenseVoiceLocalModelStatus, String>
where
    F: FnMut(SenseVoiceLocalDownloadProgress) -> Result<(), String>,
{
    let status = recommended_model_status();
    if status.installed {
        emit_progress(SenseVoiceLocalDownloadProgress {
            stage: "ready".to_string(),
            message: "Recommended SenseVoice model is already installed".to_string(),
            downloaded_bytes: 0,
            total_bytes: None,
        })?;
        return Ok(status);
    }

    let root_dir = recommended_root_dir();
    fs::create_dir_all(&root_dir).map_err(|e| e.to_string())?;

    let archive_path = recommended_archive_path();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;

    emit_progress(SenseVoiceLocalDownloadProgress {
        stage: "downloading".to_string(),
        message: "Downloading recommended SenseVoice model".to_string(),
        downloaded_bytes: 0,
        total_bytes: None,
    })?;

    let response = client
        .get(RECOMMENDED_DOWNLOAD_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to start download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let total_bytes = response.content_length();
    let mut file = File::create(&archive_path).map_err(|e| e.to_string())?;
    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();

    use futures::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded_bytes += chunk.len() as u64;

        emit_progress(SenseVoiceLocalDownloadProgress {
            stage: "downloading".to_string(),
            message: "Downloading recommended SenseVoice model".to_string(),
            downloaded_bytes,
            total_bytes,
        })?;
    }

    file.flush().map_err(|e| e.to_string())?;

    emit_progress(SenseVoiceLocalDownloadProgress {
        stage: "extracting".to_string(),
        message: "Extracting recommended SenseVoice model".to_string(),
        downloaded_bytes,
        total_bytes,
    })?;

    let extract_parent = root_dir.clone();
    let archive_path_for_extract = archive_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let tar_file = File::open(&archive_path_for_extract).map_err(|e| e.to_string())?;
        let decoder = BzDecoder::new(BufReader::new(tar_file));
        let mut archive = Archive::new(decoder);
        archive.unpack(&extract_parent).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = fs::remove_file(&archive_path);

    let final_status = recommended_model_status();
    if !final_status.installed {
        return Err("Model archive extracted, but required files were not found".to_string());
    }

    emit_progress(SenseVoiceLocalDownloadProgress {
        stage: "complete".to_string(),
        message: "Recommended SenseVoice model installed".to_string(),
        downloaded_bytes,
        total_bytes,
    })?;

    Ok(final_status)
}

pub struct SenseVoiceLocalProvider {
    provider_id: String,
    model_path: PathBuf,
    tokens_path: PathBuf,
    language: Option<String>,
    num_threads: i32,
    use_itn: bool,
}

impl SenseVoiceLocalProvider {
    pub fn new(config: &SttProviderConfig, language: Option<String>) -> Self {
        let status = recommended_model_status();
        let model_path = config
            .model_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from(&status.model_path));
        let tokens_path = config
            .tokens_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from(&status.tokens_path));

        Self {
            provider_id: config.id.clone(),
            model_path,
            tokens_path,
            language,
            num_threads: config.num_threads.unwrap_or(2).max(1),
            use_itn: config.use_itn.unwrap_or(true),
        }
    }

    fn create_recognizer(&self, language: Option<&str>) -> Result<OfflineRecognizer, SttError> {
        if !self.model_path.is_file() {
            return Err(SttError::ModelNotLoaded);
        }
        if !self.tokens_path.is_file() {
            return Err(SttError::ModelNotLoaded);
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(self.model_path.to_string_lossy().into_owned()),
            language: language
                .map(|value| value.to_string())
                .or_else(|| self.language.clone())
                .or_else(|| Some("auto".to_string())),
            use_itn: self.use_itn,
        };
        config.model_config.tokens = Some(self.tokens_path.to_string_lossy().into_owned());
        config.model_config.num_threads = self.num_threads;
        config.model_config.model_type = Some("sense_voice".to_string());

        OfflineRecognizer::create(&config).ok_or_else(|| {
            SttError::EngineUnavailable(
                "Failed to create sherpa-onnx offline SenseVoice recognizer".to_string(),
            )
        })
    }

    fn decode_source(&self, audio: &AudioSource) -> Result<(i32, Vec<f32>), SttError> {
        match audio {
            AudioSource::Chunk(chunk) => {
                Ok((chunk.sample_rate as i32, chunk.samples.as_ref().clone()))
            }
            AudioSource::Encoded { data, format } => {
                if format.to_ascii_lowercase() != "wav" {
                    return Err(SttError::AudioFormatInvalid(
                        "sensevoice_local currently supports WAV input for encoded audio"
                            .to_string(),
                    ));
                }
                decode_wav_bytes(data)
            }
        }
    }
}

fn decode_wav_bytes(data: &[u8]) -> Result<(i32, Vec<f32>), SttError> {
    let cursor = std::io::Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor)
        .map_err(|e| SttError::AudioFormatInvalid(format!("Failed to read WAV: {}", e)))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        return Err(SttError::AudioFormatInvalid(
            "sensevoice_local expects mono WAV input".to_string(),
        ));
    }

    let sample_rate = spec.sample_rate as i32;
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                SttError::AudioFormatInvalid(format!("Invalid float WAV samples: {}", e))
            })?,
        hound::SampleFormat::Int => {
            let max_amplitude =
                ((1i64 << (spec.bits_per_sample.saturating_sub(1) as u32)) - 1).max(1) as f32;
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / max_amplitude)
                        .map_err(|e| {
                            SttError::AudioFormatInvalid(format!("Invalid PCM WAV samples: {}", e))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    Ok((sample_rate, samples))
}

#[async_trait]
impl SttEngine for SenseVoiceLocalProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    async fn is_available(&self) -> bool {
        self.model_path.is_file() && self.tokens_path.is_file()
    }

    async fn transcribe(
        &self,
        audio: &AudioSource,
        language: Option<&str>,
    ) -> Result<TranscriptionResult, SttError> {
        let start_time = std::time::Instant::now();
        let recognizer = self.create_recognizer(language)?;

        let (sample_rate, samples) = self.decode_source(audio)?;
        let stream = recognizer.create_stream();
        stream.accept_waveform(sample_rate, &samples);
        recognizer.decode(&stream);

        let result = stream
            .get_result()
            .ok_or_else(|| SttError::ChunkFailed("No recognition result returned".to_string()))?;

        let segments = match result.timestamps {
            Some(timestamps) if !timestamps.is_empty() => {
                build_segments(&result.text, &result.tokens, &timestamps)
            }
            _ if !result.text.trim().is_empty() => vec![TranscriptionSegment {
                start: 0.0,
                end: audio.duration_seconds(),
                text: result.text.trim().to_string(),
                confidence: None,
            }],
            _ => Vec::new(),
        };

        Ok(TranscriptionResult {
            text: result.text.trim().to_string(),
            segments,
            processing_time: start_time.elapsed(),
        })
    }
}

fn build_segments(text: &str, tokens: &[String], timestamps: &[f32]) -> Vec<TranscriptionSegment> {
    if tokens.is_empty() || timestamps.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        let cleaned = token.trim();
        if cleaned.is_empty() {
            continue;
        }

        let start = timestamps.get(index).copied().unwrap_or(0.0);
        let end = timestamps
            .get(index + 1)
            .copied()
            .unwrap_or(start.max(text.len() as f32));

        segments.push(TranscriptionSegment {
            start,
            end,
            text: cleaned.to_string(),
            confidence: None,
        });
    }
    segments
}
