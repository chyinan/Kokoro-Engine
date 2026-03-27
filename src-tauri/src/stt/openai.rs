//! OpenAI Whisper STT provider.
//!
//! Uses the OpenAI Audio Transcription API (`/v1/audio/transcriptions`).

use super::interface::{
    AudioSource, SttEngine, SttError, TranscriptionResult, TranscriptionSegment,
};
use async_trait::async_trait;
use reqwest::multipart;
use reqwest::Url;
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
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300)) // 5 分钟默认超时，长音频需要更多时间
                .build()
                .expect("HTTP client build should not fail"),
        }
    }
}

fn transcription_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "https://api.openai.com/v1/audio/transcriptions".to_string();
    }

    if let Ok(mut url) = Url::parse(trimmed) {
        let normalized_path = if url.path().trim_end_matches('/').ends_with("/audio/transcriptions") {
            url.path().trim_end_matches('/').to_string()
        } else {
            format!("{}/audio/transcriptions", url.path().trim_end_matches('/'))
        };
        url.set_path(&normalized_path);
        return url.to_string().trim_end_matches('/').to_string();
    }

    if trimmed.ends_with("/audio/transcriptions") {
        trimmed.to_string()
    } else {
        format!("{}/audio/transcriptions", trimmed)
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
        let timeout_duration =
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
        let model = self.model.clone();
        let language = language.map(|s| s.to_string());
        let mime_type = mime_type.to_string();
        let file_name = file_name.to_string();
        let url = transcription_url(&self.base_url);
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
                        .timeout(timeout_duration)
                        .multipart(form)
                        .send()
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

#[cfg(test)]
mod tests {
    use super::transcription_url;

    #[test]
    fn appends_v1_for_root_base_url() {
        assert_eq!(
            transcription_url("https://example.com"),
            "https://example.com/audio/transcriptions"
        );
    }

    #[test]
    fn appends_transcriptions_for_v1_base_url() {
        assert_eq!(
            transcription_url("https://example.com/v1"),
            "https://example.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn preserves_existing_transcriptions_path() {
        assert_eq!(
            transcription_url("https://example.com/custom/audio/transcriptions"),
            "https://example.com/custom/audio/transcriptions"
        );
    }

    #[test]
    fn trims_whitespace_and_trailing_slash() {
        assert_eq!(
            transcription_url(" https://example.com/v1/ "),
            "https://example.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn preserves_query_parameters_on_full_endpoint() {
        assert_eq!(
            transcription_url("https://example.com/audio/transcriptions?api-version=2024-06-01"),
            "https://example.com/audio/transcriptions?api-version=2024-06-01"
        );
    }

    #[test]
    fn preserves_custom_prefix_without_injecting_v1() {
        assert_eq!(
            transcription_url("https://example.com/openai"),
            "https://example.com/openai/audio/transcriptions"
        );
    }

    #[test]
    fn empty_string_returns_default_url() {
        assert_eq!(
            transcription_url(""),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn whitespace_only_returns_default_url() {
        assert_eq!(
            transcription_url("   \t  \n  "),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn unparseable_url_falls_back_to_string_concat() {
        // Invalid URL scheme should fall through to string concatenation
        let result = transcription_url("not-a-valid-url");
        assert!(
            result.ends_with("/audio/transcriptions"),
            "Should append /audio/transcriptions even for unparseable URLs"
        );
    }

    #[test]
    fn url_with_port_number() {
        assert_eq!(
            transcription_url("https://example.com:8080/v1"),
            "https://example.com:8080/v1/audio/transcriptions"
        );
    }

    #[test]
    fn url_with_auth_credentials() {
        let result = transcription_url("https://user:pass@example.com/v1");
        assert!(
            result.contains("/audio/transcriptions"),
            "Should handle URLs with auth credentials"
        );
    }
}
