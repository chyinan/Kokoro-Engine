//! STT provider trait and shared types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Error Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SttError {
    ProviderNotFound(String),
    TranscriptionFailed(String),
    AudioTooShort,
    NoApiKey,
    Unavailable(String),
    ConfigError(String),
}

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SttError::ProviderNotFound(id) => write!(f, "STT provider not found: {}", id),
            SttError::TranscriptionFailed(msg) => write!(f, "Transcription failed: {}", msg),
            SttError::AudioTooShort => write!(f, "Audio recording too short"),
            SttError::NoApiKey => write!(f, "No API key configured for STT provider"),
            SttError::Unavailable(msg) => write!(f, "STT unavailable: {}", msg),
            SttError::ConfigError(msg) => write!(f, "STT config error: {}", msg),
        }
    }
}

impl std::error::Error for SttError {}

impl From<SttError> for String {
    fn from(e: SttError) -> String {
        e.to_string()
    }
}

// ── Provider Trait ──────────────────────────────────────

#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Unique identifier for this provider (e.g., "openai_whisper")
    fn id(&self) -> String;

    /// Check if the provider is currently usable
    async fn is_available(&self) -> bool;

    /// Transcribe audio bytes to text.
    ///
    /// `audio` — raw audio bytes (WebM, WAV, etc.)
    /// `format` — MIME hint, e.g. "webm", "wav"
    /// `language` — optional BCP-47 hint, e.g. "zh", "en", "ja"
    async fn transcribe(
        &self,
        audio: &[u8],
        format: &str,
        language: Option<&str>,
    ) -> Result<String, SttError>;
}
