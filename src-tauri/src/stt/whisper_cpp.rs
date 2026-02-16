//! Whisper.cpp STT provider.
//!
//! Connects to a running `whisper.cpp` server (example server).
//! Default endpoint: `http://127.0.0.1:8080/inference`

use super::interface::{SttError, SttProvider};
use async_trait::async_trait;
use reqwest::multipart;

pub struct WhisperCppProvider {
    base_url: String,
}

impl WhisperCppProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
        }
    }
}

#[async_trait]
impl SttProvider for WhisperCppProvider {
    fn id(&self) -> String {
        "whisper_cpp".to_string()
    }

    async fn is_available(&self) -> bool {
        // Simple health check check
        let url = self.base_url.clone();
        let client = reqwest::Client::new();
        // whisper.cpp server usually serves a UI at root
        match client
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        {
            Ok(res) => res.status().is_success(),
            Err(_) => false,
        }
    }

    async fn transcribe(
        &self,
        audio: &[u8],
        _format: &str, // whisper.cpp expects WAV/PCM usually, but server handles conversion often
        language: Option<&str>,
    ) -> Result<String, SttError> {
        if audio.len() < 1024 {
            return Err(SttError::AudioTooShort);
        }

        // whisper.cpp server endpoint is usually /inference
        let url = format!("{}/inference", self.base_url.trim_end_matches('/'));

        let part = multipart::Part::bytes(audio.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| SttError::TranscriptionFailed(format!("MIME error: {}", e)))?;

        let mut form = multipart::Form::new().part("file", part);

        // Map language
        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        // Response format: json or text. We ask for json usually to get clean text, or just text.
        // The example server returns JSON by default with "text" field.
        form = form.text("response_format", "json");

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| SttError::TranscriptionFailed(format!("HTTP error: {}", e)))?;

        if !response.status().is_success() {
            return Err(SttError::TranscriptionFailed(format!(
                "whisper.cpp returned {}",
                response.status()
            )));
        }

        // Parse JSON response: { "text": "..." }
        #[derive(serde::Deserialize)]
        struct WhisperCppResponse {
            text: String,
        }

        let resp_json: WhisperCppResponse = response.json().await.map_err(|e| {
            SttError::TranscriptionFailed(format!("Failed to parse whisper.cpp response: {}", e))
        })?;

        Ok(resp_json.text.trim().to_string())
    }
}
