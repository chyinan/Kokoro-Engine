//! STT configuration — persisted to `stt_config.json`.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttProviderConfig {
    pub id: String,
    /// "openai_whisper", "whisper_cpp", "faster_whisper", "local_whisper"
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

    #[serde(default = "default_providers")]
    pub providers: Vec<SttProviderConfig>,
}

fn default_active_provider() -> String {
    "openai_whisper".to_string()
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
        },
        SttProviderConfig {
            id: "whisper_cpp".to_string(),
            provider_type: "whisper_cpp".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://127.0.0.1:8080".to_string()),
            model: None,
        },
        SttProviderConfig {
            id: "faster_whisper".to_string(),
            provider_type: "faster_whisper".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://127.0.0.1:8000/v1".to_string()),
            model: Some("medium".to_string()),
        },
    ]
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            active_provider: default_active_provider(),
            language: None,
            auto_send: false,
            providers: default_providers(),
        }
    }
}

pub fn load_config(path: &Path) -> SttConfig {
    crate::config::load_json_config(path, "STT")
}

pub fn save_config(path: &Path, config: &SttConfig) -> Result<(), String> {
    crate::config::save_json_config(path, config, "STT")
}
