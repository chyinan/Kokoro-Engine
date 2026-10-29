use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::pin::Pin; // Add this
use tokio::sync::watch;

// ── Error Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TtsError {
    ProviderNotFound(String),
    SynthesisFailed(String),
    Timeout(String),
    ConfigError(String),
    Unavailable(String),
    CacheError(String),
    Cancelled,
    BrowserDelegate, // Sentinel: frontend should use window.speechSynthesis
}

impl fmt::Display for TtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TtsError::ProviderNotFound(id) => write!(f, "TTS provider not found: {}", id),
            TtsError::SynthesisFailed(msg) => write!(f, "Synthesis failed: {}", msg),
            TtsError::Timeout(msg) => write!(f, "TTS timeout: {}", msg),
            TtsError::ConfigError(msg) => write!(f, "TTS config error: {}", msg),
            TtsError::Unavailable(msg) => write!(f, "TTS unavailable: {}", msg),
            TtsError::CacheError(msg) => write!(f, "TTS cache error: {}", msg),
            TtsError::Cancelled => write!(f, "TTS request cancelled"),
            TtsError::BrowserDelegate => write!(f, "BROWSER_TTS_DELEGATE"),
        }
    }
}

impl std::error::Error for TtsError {}

// For Tauri command return compatibility
impl From<TtsError> for String {
    fn from(e: TtsError) -> String {
        e.to_string()
    }
}

// ── Capability Flags ───────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub supports_streaming: bool,
    pub supports_emotions: bool,
    pub supports_speed: bool,
    pub supports_pitch: bool,
    pub supports_cloning: bool,
    pub supports_ssml: bool,
}

impl ProviderCapabilities {
    /// Score how well this provider matches a set of requested capabilities.
    /// Returns a value between 0.0 and 1.0.
    pub fn match_score(&self, requested: &ProviderCapabilities) -> f32 {
        let checks: Vec<(bool, bool)> = vec![
            (requested.supports_streaming, self.supports_streaming),
            (requested.supports_emotions, self.supports_emotions),
            (requested.supports_speed, self.supports_speed),
            (requested.supports_pitch, self.supports_pitch),
            (requested.supports_cloning, self.supports_cloning),
            (requested.supports_ssml, self.supports_ssml),
        ];

        let requested_count = checks.iter().filter(|(req, _)| *req).count();
        if requested_count == 0 {
            return 1.0; // No specific requirements → everything matches
        }

        let matched = checks.iter().filter(|(req, has)| *req && *has).count();
        matched as f32 / requested_count as f32
    }
}

// ── Voice Profiles ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    Male,
    Female,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TtsEngine {
    Vits,
    Cloud,
    Native, // Browser SpeechSynthesis
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceProfile {
    pub voice_id: String,
    pub name: String,
    pub gender: Gender,
    pub language: String,
    pub engine: TtsEngine,
    pub provider_id: String,
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
}

// ── Synthesis Parameters ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsParams {
    pub voice: Option<String>,
    pub speed: Option<f32>,
    pub pitch: Option<f32>,
    pub emotion: Option<String>,
    /// Optional capability hints for smart routing
    #[serde(default)]
    pub required_capabilities: Option<ProviderCapabilities>,
    #[serde(default)]
    pub extra_params: Option<HashMap<String, serde_json::Value>>,
}

/// Explicit audio metadata for streams that cannot be detected from their bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "codec", rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
    Wav,
    PcmS16Le { sample_rate: u32, channels: u8 },
}

/// One ordered audio delta from a provider response.
#[derive(Debug, Clone)]
pub struct TtsAudioChunk {
    pub data: Vec<u8>,
    pub format: Option<AudioFormat>,
    pub response_id: Option<String>,
}

pub type TtsAudioStream =
    Pin<Box<dyn futures::Stream<Item = Result<TtsAudioChunk, TtsError>> + Send>>;

impl Default for TtsParams {
    fn default() -> Self {
        Self {
            voice: None,
            speed: Some(1.0),
            pitch: Some(1.0),
            emotion: None,
            required_capabilities: None,
            extra_params: None,
        }
    }
}

impl TtsParams {
    /// Stable salt for cache keys derived from per-request extra params.
    pub fn extra_cache_key_salt(&self) -> Option<String> {
        let extra = self.extra_params.as_ref()?;
        if extra.is_empty() {
            return None;
        }

        let mut keys: Vec<_> = extra.keys().collect();
        keys.sort();

        let mut map = serde_json::Map::new();
        for key in keys {
            if let Some(value) = extra.get(key) {
                map.insert(key.clone(), value.clone());
            }
        }

        Some(serde_json::Value::Object(map).to_string())
    }
}

// ── Provider Trait ──────────────────────────────────────

#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Unique identifier for this provider (e.g., "openai", "azure", "browser")
    fn id(&self) -> String;

    /// Declare what this provider can do
    fn capabilities(&self) -> ProviderCapabilities;

    /// List available voices for this provider
    fn voices(&self) -> Vec<VoiceProfile>;

    /// Optional stable salt for cache keys derived from provider-specific
    /// settings that affect audio output.
    fn cache_key_salt(&self) -> Option<String> {
        None
    }

    /// Check if the provider is currently reachable / operational
    async fn is_available(&self) -> bool;

    /// Actionable reason returned when an explicitly selected provider is unavailable.
    fn unavailable_reason(&self) -> Option<String> {
        None
    }

    /// Synthesize text to audio bytes (MP3/WAV/PCM)
    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError>;

    /// Streaming synthesis — returns chunks incrementally.
    /// Default implementation falls back to non-streaming `synthesize`.
    /// Streaming synthesis — returns chunks incrementally.
    /// Default implementation falls back to non-streaming `synthesize`.
    async fn synthesize_stream(
        &self,
        text: &str,
        params: TtsParams,
    ) -> Result<TtsAudioStream, TtsError> {
        // Default: return entire audio as a single chunk
        let audio = self.synthesize(text, params).await?;
        let stream = futures::stream::once(async move {
            Ok(TtsAudioChunk {
                data: audio,
                format: None,
                response_id: None,
            })
        });
        Ok(Box::pin(stream))
    }

    /// Per-request entry point. Providers with remote streaming protocols can
    /// use the stable request ID and cancellation signal to cancel server work.
    async fn synthesize_stream_for_request(
        &self,
        text: &str,
        params: TtsParams,
        _request_id: &str,
        _cancel: watch::Receiver<bool>,
    ) -> Result<TtsAudioStream, TtsError> {
        self.synthesize_stream(text, params).await
    }

    /// Cancel remote work belonging to one request. Implementations that own
    /// request-scoped connections should cancel the matching response only.
    async fn cancel_request(&self, _request_id: &str) {}

    /// Number of sentences this provider can safely synthesize concurrently.
    fn sentence_concurrency(&self) -> usize {
        2
    }

    /// Metadata used for cached chunks and providers whose stream has no header.
    fn output_format(&self) -> Option<AudioFormat> {
        None
    }
}
