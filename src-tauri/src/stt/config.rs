//! STT configuration — persisted to `stt_config.json`.

use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttProviderConfig {
    pub id: String,
    /// "openai_whisper", "whisper_cpp", "faster_whisper", "local_whisper", "sensevoice_local"
    pub provider_type: String,
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Direct API key (takes precedence over env var)
    pub api_key: Option<String>,
    /// Environment variable name to read API key from
    pub api_key_env: Option<String>,
    /// Base URL for the API
    pub base_url: Option<String>,
    /// Model name (e.g., "whisper-1")
    pub model: Option<String>,

    // ── sensevoice_local fields ────────────────────────
    /// Path to the ONNX model file (overrides recommended default when set)
    #[serde(default)]
    pub model_path: Option<String>,
    /// Path to the tokens.txt file (overrides recommended default when set)
    #[serde(default)]
    pub tokens_path: Option<String>,
    /// Number of inference threads (default: 2)
    #[serde(default)]
    pub num_threads: Option<i32>,
    /// Enable Inverse Text Normalization (default: true)
    #[serde(default)]
    pub use_itn: Option<bool>,
}

impl SttProviderConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        crate::config::resolve_api_key(&self.api_key, &self.api_key_env)
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// ID of the active STT provider
    #[serde(default = "default_active_provider")]
    pub active_provider: String,

    /// Optional language hint (BCP-47 code: "zh", "en", "ja", etc.)
    #[serde(default)]
    pub language: Option<String>,

    /// Auto-send transcribed text to chat (without requiring user to press Send)
    #[serde(default)]
    pub auto_send: bool,

    /// Wake word detection enabled
    #[serde(default)]
    pub wake_word_enabled: bool,

    /// Continuously listen for speech and start STT without a wake word.
    #[serde(default)]
    pub continuous_listening: bool,

    /// Wake word string (e.g. "你好心音"). Case-insensitive substring match.
    #[serde(default)]
    pub wake_word: Option<String>,

    #[serde(default = "default_providers")]
    pub providers: Vec<SttProviderConfig>,
}

fn default_active_provider() -> String {
    "openai_whisper".to_string()
}

pub fn default_providers_pub() -> Vec<SttProviderConfig> {
    default_providers()
}

fn default_providers() -> Vec<SttProviderConfig> {
    vec![
        SttProviderConfig {
            id: "openai_whisper".to_string(),
            provider_type: "openai_whisper".to_string(),
            enabled: true,
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            base_url: Some("https://api.openai.com/v1".to_string()),
            model: Some("whisper-1".to_string()),
            model_path: None,
            tokens_path: None,
            num_threads: None,
            use_itn: None,
        },
        SttProviderConfig {
            id: "whisper_cpp".to_string(),
            provider_type: "whisper_cpp".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://127.0.0.1:8080".to_string()),
            model: None,
            model_path: None,
            tokens_path: None,
            num_threads: None,
            use_itn: None,
        },
        SttProviderConfig {
            id: "faster_whisper".to_string(),
            provider_type: "faster_whisper".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://127.0.0.1:8000/v1".to_string()),
            model: Some("medium".to_string()),
            model_path: None,
            tokens_path: None,
            num_threads: None,
            use_itn: None,
        },
        SttProviderConfig {
            id: "sensevoice".to_string(),
            provider_type: "sensevoice".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://127.0.0.1:50000".to_string()),
            model: None,
            model_path: None,
            tokens_path: None,
            num_threads: None,
            use_itn: None,
        },
        SttProviderConfig {
            id: "sensevoice_local".to_string(),
            provider_type: "sensevoice_local".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: None,
            model: None,
            model_path: None,
            tokens_path: None,
            num_threads: Some(2),
            use_itn: Some(true),
        },
    ]
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            active_provider: default_active_provider(),
            language: None,
            auto_send: false,
            wake_word_enabled: false,
            continuous_listening: false,
            wake_word: None,
            providers: default_providers(),
        }
    }
}

pub fn load_config(path: &Path) -> SttConfig {
    crate::config::load_json_config(path, "STT")
}

pub fn save_config(path: &Path, config: &SttConfig) -> Result<(), KokoroError> {
    crate::config::save_json_config(path, config, "STT")
}
