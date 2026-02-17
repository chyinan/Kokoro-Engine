//! Whisper.cpp STT provider.
//!
//! Connects to a running `whisper.cpp` server.

use super::interface::{
    AudioSource, SttEngine, SttError, TranscriptionResult, TranscriptionSegment,
};
use async_trait::async_trait;
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

pub struct WhisperCppProvider {
    base_url: String,
    client: reqwest::Client,
}

impl WhisperCppProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct WhisperCppResponse {
    text: String,
    #[serde(default)]
    segments: Vec<WhisperCppSegment>,
}

#[derive(Deserialize)]
struct WhisperCppSegment {
    #[serde(default)]
    start: f32,
    #[serde(default)]
    end: f32,
    text: String,
}

#[async_trait]
impl SttEngine for WhisperCppProvider {
    fn id(&self) -> String {
        "whisper_cpp".to_string()
    }

    async fn is_available(&self) -> bool {
        // Simple health check check
        let url = self.base_url.clone();
        match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(res) => res.status().is_success(),
            Err(_) => false,
        }
    }

    async fn transcribe(
        &self,
        audio: &AudioSource,
        language: Option<&str>,
    ) -> Result<TranscriptionResult, SttError> {
        let start_time = std::time::Instant::now();
        let duration_sec = audio.duration_seconds();
        // generous timeout for local CPU inference
        let timeout_duration =
            Duration::from_secs(10) + Duration::from_secs_f32(duration_sec * 4.0);

        // whisper.cpp server endpoint is usually /inference or /v1/audio/transcriptions if generic
        // But the user code used /inference. Let's stick to it but try to send OpenAI parameters?
        // Standard whisper.cpp /inference takes multipart but response format might differ.
        // If we want detailed segments, we hope /inference supports formatting or returns them by default.
        // The default llama.cpp/whisper.cpp server returns "transcription" array in JSON.
        // Let's try to parse "transcription" alias for "segments".

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/inference", base);

        let (file_name, mime_type, file_bytes) = match audio {
            AudioSource::Chunk(chunk) => (
                "audio.wav".to_string(),
                "audio/wav".to_string(),
                chunk.to_wav_bytes(),
            ),
            AudioSource::Encoded { data, format } => {
                let fmt_lower = format.to_lowercase();
                let ext = match fmt_lower.as_str() {
                    "wav" => "wav",
                    // whisper.cpp usually demands wav, but let's try passing others if server supports
                    other => other,
                };
                (
                    format!("audio.{}", ext),
                    format!("audio/{}", ext),
                    data.clone(),
                )
            }
        };

        let part = multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str(&mime_type)
            .map_err(|e| SttError::AudioFormatInvalid(e.to_string()))?;

        let mut form = multipart::Form::new().part("file", part);

        // Map language
        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        form = form.text("response_format", "verbose_json");
        // Also try "temperature" if needed, but let's keep it simple.

        let response = self
            .client
            .post(&url)
            .multipart(form)
            .timeout(timeout_duration)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SttError::Timeout {
                        expected: format!("{:?}", timeout_duration),
                        actual: "timeout".to_string(),
                    }
                } else {
                    SttError::IOError(format!("Network error: {}", e))
                }
            })?;

        if !response.status().is_success() {
            return Err(SttError::EngineUnavailable(format!(
                "whisper.cpp returned {}",
                response.status()
            )));
        }

        let resp_json: WhisperCppResponse = response.json().await.map_err(|e| {
            SttError::Unknown(format!("Failed to parse whisper.cpp response: {}", e))
        })?;

        let text = resp_json.text.trim().to_string();

        let segments = if !resp_json.segments.is_empty() {
            resp_json
                .segments
                .into_iter()
                .map(|s| TranscriptionSegment {
                    start: s.start,
                    end: s.end,
                    text: s.text,
                    confidence: None,
                })
                .collect()
        } else {
            // Fallback if no segments returned
            vec![TranscriptionSegment {
                start: 0.0,
                end: duration_sec,
                text: text.clone(),
                confidence: None,
            }]
        };

        Ok(TranscriptionResult {
            text,
            segments,
            processing_time: start_time.elapsed(),
        })
    }
}
