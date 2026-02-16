//! OpenAI Whisper STT provider.
//!
//! Uses the OpenAI Audio Transcription API (`/v1/audio/transcriptions`)
//! to convert audio to text. Supports WebM, WAV, MP3, and other formats.

use super::interface::{SttError, SttProvider};
use async_trait::async_trait;
use reqwest::multipart;

pub struct OpenAIWhisperProvider {
    provider_id: String,
    api_key: String,
    base_url: String,
    model: String,
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
        }
    }
}

/// Map audio format hint to a file extension that the Whisper API accepts.
fn format_to_extension(format: &str) -> &str {
    match format.to_lowercase().as_str() {
        "webm" => "webm",
        "wav" => "wav",
        "mp3" | "mpeg" => "mp3",
        "mp4" | "m4a" => "m4a",
        "ogg" | "oga" => "ogg",
        "flac" => "flac",
        _ => "webm", // Default to webm (browser MediaRecorder default)
    }
}

#[async_trait]
impl SttProvider for OpenAIWhisperProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn transcribe(
        &self,
        audio: &[u8],
        format: &str,
        language: Option<&str>,
    ) -> Result<String, SttError> {
        // Minimum audio size check (~0.5s of audio is roughly > 1KB)
        if audio.len() < 1024 {
            return Err(SttError::AudioTooShort);
        }

        let ext = format_to_extension(format);
        let filename = format!("audio.{}", ext);

        let file_part = multipart::Part::bytes(audio.to_vec())
            .file_name(filename)
            .mime_str(&format!("audio/{}", ext))
            .map_err(|e| SttError::TranscriptionFailed(format!("MIME error: {}", e)))?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "text".to_string());

        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        let url = format!(
            "{}/audio/transcriptions",
            self.base_url.trim_end_matches('/')
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| SttError::TranscriptionFailed(format!("HTTP error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(SttError::TranscriptionFailed(format!(
                "API returned {}: {}",
                status, body
            )));
        }

        let text = response.text().await.map_err(|e| {
            SttError::TranscriptionFailed(format!("Failed to read response: {}", e))
        })?;

        Ok(text.trim().to_string())
    }
}
