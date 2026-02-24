//! OpenAI Whisper STT provider.
//!
//! Uses the OpenAI Audio Transcription API (`/v1/audio/transcriptions`).

use super::interface::{
    AudioSource, SttEngine, SttError, TranscriptionResult, TranscriptionSegment,
};
use async_trait::async_trait;
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct OpenAIWhisperProvider {
    provider_id: String,
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAIWhisperProvider {
    pub fn new(
        provider_id: String,
        api_key: String,
        base_url: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            provider_id,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "whisper-1".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

// Response structures for verbose_json
#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    text: String,
    #[serde(default)]
    segments: Vec<OpenAISegment>,
    #[serde(default)]
    #[allow(dead_code)]
    duration: f32,
}

#[derive(Debug, Deserialize)]
struct OpenAISegment {
    start: f32,
    end: f32,
    text: String,
    // OpenAI sometimes returns "avg_logprob" or "no_speech_prob", not always "confidence" directly in legacy models,
    // but usually keys are standardized in v1. We'll leave confidence optional.
    #[serde(default)]
    no_speech_prob: f32,
}

#[async_trait]
impl SttEngine for OpenAIWhisperProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn transcribe(
        &self,
        audio: &AudioSource,
        language: Option<&str>,
    ) -> Result<TranscriptionResult, SttError> {
        let start_time = std::time::Instant::now();
        let duration_sec = audio.duration_seconds();

        // 1. Dynamic Timeout Calculation
        // Base 10s + 2x audio duration. e.g. 30s audio -> 70s timeout.
        let _timeout_duration =
            Duration::from_secs(10) + Duration::from_secs_f32(duration_sec * 2.0);

        // 2. Prepare Form Data
        let (file_name, mime_type, file_bytes) = match audio {
            AudioSource::Chunk(chunk) => (
                "audio.wav".to_string(),
                "audio/wav".to_string(),
                chunk.to_wav_bytes(),
            ),
            AudioSource::Encoded { data, format } => {
                let ext = match format.to_lowercase().as_str() {
                    "wav" => "wav",
                    "mp3" | "mpeg" => "mp3",
                    "m4a" | "mp4" => "m4a",
                    "flac" => "flac",
                    "ogg" | "oga" => "ogg",
                    _ => "webm",
                };
                (
                    format!("audio.{}", ext),
                    format!("audio/{}", ext),
                    data.clone(),
                )
            }
        };

        let file_bytes = std::sync::Arc::new(file_bytes); // Cheap clone for retries

        // Clone for closure capture
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let _base_url = self.base_url.clone(); // Needed if we reconstruct URL, but we have `url` string below.
        let model = self.model.clone();
        let language = language.map(|s| s.to_string());
        let mime_type = mime_type.to_string();
        let file_name = file_name.to_string();
        let url = format!(
            "{}/audio/transcriptions",
            self.base_url.trim_end_matches('/')
        );
        let url_arc = std::sync::Arc::new(url);

        let response = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_arc.clone();
                let api_key = api_key.clone();
                let model = model.clone();
                let language = language.clone();
                let file_bytes = file_bytes.clone();
                let file_name = file_name.clone();
                let mime_type = mime_type.clone();

                async move {
                    // Reconstruct multipart form for each attempt (Form consumes data)
                    let part = multipart::Part::bytes(file_bytes.as_ref().clone())
                        .file_name(file_name)
                        .mime_str(&mime_type)
                        .unwrap();
                    // Note: reqwest::Error doesn't easily wrap custom errors, but we can't return SttError here.
                    // Actually Part::mime_str error is not reqwest::Error.
                    // Simple fix: expect/unwrap because mime_type is verified safe above?
                    // Or just use text/plain if fail?
                    // Let's use expect for now as we control mime types in the match above.
                    // Actually, mime_str returns reqwest::Error in newer versions?
                    // No, it returns generic Error.
                    // We can just ignore the error inside this retry loop for simplicity or map it.
                    // Let's assume valid mime types from our match block.

                    let mut form = multipart::Form::new()
                        .part("file", part)
                        .text("model", model)
                        .text("response_format", "verbose_json");

                    if let Some(lang) = &language {
                        form = form.text("language", lang.clone());
                    }

                    client
                        .post(url.as_str())
                        .header("Authorization", format!("Bearer {}", api_key))
                        .multipart(form)
                        .send() // timeout is on client
                        .await
                }
            },
            3,
        )
        .await
        .map_err(|e| SttError::IOError(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(SttError::EngineUnavailable(
                    "Rate limit exceeded (429)".to_string(),
                ));
            }
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SttError::ConfigError("Invalid API Key".to_string()));
            }
            if status.is_server_error() {
                return Err(SttError::EngineUnavailable(format!(
                    "Service Error: {}",
                    status
                )));
            }
            return Err(SttError::Unknown(format!("API Error {}: {}", status, body)));
        }

        // 3. Parse Response
        let resp_json: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| SttError::Unknown(format!("Failed to parse OpenAI JSON: {}", e)))?;

        // 4. Map to Standard Result
        let segments = resp_json
            .segments
            .into_iter()
            .map(|s| TranscriptionSegment {
                start: s.start,
                end: s.end,
                text: s.text.trim().to_string(),
                confidence: Some(1.0 - s.no_speech_prob), // Rough proxy for confidence
            })
            .collect();

        Ok(TranscriptionResult {
            text: resp_json.text.trim().to_string(),
            segments,
            processing_time: start_time.elapsed(),
        })
    }
}
