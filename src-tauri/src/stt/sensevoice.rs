//! SenseVoice STT provider.
//!
//! Calls the FunAudioLLM SenseVoice FastAPI server (`/api/v1/asr`).

use super::interface::{
    AudioSource, SttEngine, SttError, TranscriptionResult, TranscriptionSegment,
};
use async_trait::async_trait;
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct SenseVoiceProvider {
    provider_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl SenseVoiceProvider {
    pub fn new(provider_id: String, base_url: Option<String>) -> Self {
        Self {
            provider_id,
            base_url: base_url.unwrap_or_else(|| "http://127.0.0.1:50000".to_string()),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("HTTP client build should not fail"),
        }
    }
}

// ── Response Types ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SenseVoiceResponse {
    result: Vec<SenseVoiceItem>,
}

#[derive(Debug, Deserialize)]
struct SenseVoiceItem {
    /// Clean text with emotion/event tags stripped
    clean_text: String,
    /// Timestamp list: [[start_ms, end_ms, token], ...]
    #[serde(default)]
    timestamp: Vec<Vec<serde_json::Value>>,
}

// ── SttEngine impl ──────────────────────────────────────

#[async_trait]
impl SttEngine for SenseVoiceProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/", self.base_url.trim_end_matches('/'));
        self.client.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn transcribe(
        &self,
        audio: &AudioSource,
        language: Option<&str>,
    ) -> Result<TranscriptionResult, SttError> {
        let start_time = std::time::Instant::now();

        // Build WAV bytes
        let (file_bytes, file_name, mime) = match audio {
            AudioSource::Chunk(chunk) => (chunk.to_wav_bytes(), "audio.wav".to_string(), "audio/wav"),
            AudioSource::Encoded { data, format } => {
                let (ext, mime) = match format.to_lowercase().as_str() {
                    "mp3" | "mpeg" => ("mp3", "audio/mpeg"),
                    "m4a" | "mp4" => ("m4a", "audio/mp4"),
                    "flac" => ("flac", "audio/flac"),
                    "ogg" | "oga" => ("ogg", "audio/ogg"),
                    _ => ("wav", "audio/wav"),
                };
                (data.clone(), format!("audio.{}", ext), mime)
            }
        };

        let lang = language.unwrap_or("auto").to_string();
        let url = format!("{}/api/v1/asr", self.base_url.trim_end_matches('/'));

        let part = multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str(mime)
            .map_err(|e| SttError::IOError(e.to_string()))?;

        let form = multipart::Form::new()
            .part("files", part)
            .text("lang", lang);

        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| SttError::IOError(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SttError::EngineUnavailable(format!(
                "SenseVoice error {}: {}",
                status, body
            )));
        }

        let resp: SenseVoiceResponse = response
            .json()
            .await
            .map_err(|e| SttError::Unknown(format!("Failed to parse SenseVoice JSON: {}", e)))?;

        if resp.result.is_empty() {
            return Ok(TranscriptionResult {
                text: String::new(),
                segments: Vec::new(),
                processing_time: start_time.elapsed(),
            });
        }

        let item = &resp.result[0];
        let text = item.clean_text.trim().to_string();

        // Build segments from timestamp array [[start_ms, end_ms, token], ...]
        let segments: Vec<TranscriptionSegment> = item
            .timestamp
            .iter()
            .filter_map(|entry| {
                if entry.len() < 3 {
                    return None;
                }
                let start = entry[0].as_f64()? as f32 / 1000.0;
                let end = entry[1].as_f64()? as f32 / 1000.0;
                let token = entry[2].as_str()?.trim().to_string();
                if token.is_empty() {
                    return None;
                }
                Some(TranscriptionSegment {
                    start,
                    end,
                    text: token,
                    confidence: None,
                })
            })
            .collect();

        // If no timestamp data, create a single segment spanning the whole clip
        let segments = if segments.is_empty() && !text.is_empty() {
            vec![TranscriptionSegment {
                start: 0.0,
                end: audio.duration_seconds(),
                text: text.clone(),
                confidence: None,
            }]
        } else {
            segments
        };

        Ok(TranscriptionResult {
            text,
            segments,
            processing_time: start_time.elapsed(),
        })
    }
}
