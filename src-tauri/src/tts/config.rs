use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ── Provider Config ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub provider_type: String, // "openai", "local_vits", "local_rvc", "azure", "elevenlabs", "browser"
    #[serde(default = "default_true")]
    pub enabled: bool,

    // Common fields (optional, provider-specific)
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub default_voice: Option<String>,
    pub model_path: Option<String>,

    /// Catch-all for provider-specific config
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ProviderConfig {
    /// Resolve the API key: check `api_key` field first, then `api_key_env` environment variable.
    pub fn resolve_api_key(&self) -> Option<String> {
        crate::config::resolve_api_key(&self.api_key, &self.api_key_env)
    }
}

fn default_true() -> bool {
    true
}

// ── Cache Config ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 500,
            ttl_secs: 3600,
        }
    }
}

fn default_max_entries() -> usize {
    500
}
fn default_ttl_secs() -> u64 {
    3600
}

// ── Queue Config ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { max_concurrent: 3 }
    }
}

fn default_max_concurrent() -> usize {
    3
}

// ── Top-Level System Config ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSystemConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

impl Default for TtsSystemConfig {
    fn default() -> Self {
        Self {
            default_provider: Some("browser".to_string()),
            cache: CacheConfig::default(),
            queue: QueueConfig::default(),
            providers: vec![
                // Browser provider is always available as fallback
                ProviderConfig {
                    id: "browser".to_string(),
                    provider_type: "browser".to_string(),
                    enabled: true,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    endpoint: None,
                    model: None,
                    default_voice: None,
                    model_path: None,
                    extra: HashMap::new(),
                },
            ],
        }
    }
}

/// Load TTS config from a JSON file. Falls back to defaults if file is missing or invalid.
pub fn load_config(path: &Path) -> TtsSystemConfig {
    crate::config::load_json_config(path, "TTS")
}

/// Save TTS config to a JSON file.
pub fn save_config(path: &Path, config: &TtsSystemConfig) -> Result<(), String> {
    crate::config::save_json_config(path, config, "TTS")
}
