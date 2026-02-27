//! LLM configuration — persisted to `llm_config.json`.

use crate::config;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderConfig {
    pub id: String,
    /// "openai" | "ollama"
    pub provider_type: String,
    #[serde(default = "default_true")]
    pub enabled: bool,

    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,

    /// Catch-all for provider-specific config.
    #[serde(default)]
    pub extra: HashMap<String, Value>,
}

impl LlmProviderConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        config::resolve_api_key(&self.api_key, &self.api_key_env)
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPreset {
    pub id: String,
    pub name: String,
    pub active_provider: String,
    pub system_provider: Option<String>,
    pub system_model: Option<String>,
    pub providers: Vec<LlmProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// ID of the active provider (must match one of `providers[].id`).
    #[serde(default = "default_active_provider")]
    pub active_provider: String,

    /// Optional: Separate provider for system tasks (Intent Parsing).
    /// If None, uses `active_provider`.
    pub system_provider: Option<String>,

    /// Optional: Override model for system tasks.
    pub system_model: Option<String>,

    #[serde(default = "default_providers")]
    pub providers: Vec<LlmProviderConfig>,

    #[serde(default)]
    pub presets: Vec<LlmPreset>,
}

fn default_active_provider() -> String {
    "openai".to_string()
}

fn default_providers() -> Vec<LlmProviderConfig> {
    vec![
        LlmProviderConfig {
            id: "openai".to_string(),
            provider_type: "openai".to_string(),
            enabled: true,
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            base_url: Some("https://api.openai.com/v1".to_string()),
            model: Some("gpt-4".to_string()),
            extra: HashMap::new(),
        },
        LlmProviderConfig {
            id: "ollama".to_string(),
            provider_type: "ollama".to_string(),
            enabled: false,
            api_key: None,
            api_key_env: None,
            base_url: Some("http://localhost:11434".to_string()),
            model: Some("llama3".to_string()),
            extra: HashMap::new(),
        },
    ]
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            active_provider: default_active_provider(),
            system_provider: None,
            system_model: None,
            providers: default_providers(),
            presets: Vec::new(),
        }
    }
}

pub fn load_config(path: &Path) -> LlmConfig {
    config::load_json_config(path, "LLM")
}

pub fn save_config(path: &Path, config: &LlmConfig) -> Result<(), String> {
    config::save_json_config(path, config, "LLM")
}
