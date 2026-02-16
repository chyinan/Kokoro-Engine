use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

// ── Provider Config ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenProviderConfig {
    pub id: String,
    pub provider_type: String, // "openai", "stable_diffusion"
    #[serde(default = "default_true")]
    pub enabled: bool,

    // Common fields
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,   // e.g. "dall-e-3", "sd_xl"
    pub size: Option<String>,    // e.g. "1024x1024"
    pub quality: Option<String>, // e.g. "standard", "hd"
    pub style: Option<String>,   // e.g. "vivid", "natural"

    /// Catch-all for provider-specific config
    #[serde(default)]
    pub extra: HashMap<String, Value>,
}

impl ImageGenProviderConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        crate::config::resolve_api_key(&self.api_key, &self.api_key_env)
    }
}

fn default_true() -> bool {
    true
}

// ── System Config ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenSystemConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub providers: Vec<ImageGenProviderConfig>,
}

impl Default for ImageGenSystemConfig {
    fn default() -> Self {
        Self {
            default_provider: Some("openai".to_string()),
            enabled: true,
            providers: vec![
                // Default OpenAI entry
                ImageGenProviderConfig {
                    id: "openai".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    api_key: None,
                    api_key_env: Some("OPENAI_API_KEY".to_string()),
                    base_url: None,
                    model: Some("dall-e-3".to_string()),
                    size: Some("1024x1024".to_string()),
                    quality: Some("standard".to_string()),
                    style: Some("vivid".to_string()),
                    extra: HashMap::new(),
                },
                // Default Stable Diffusion WebUI entry
                ImageGenProviderConfig {
                    id: "sd_local".to_string(),
                    provider_type: "stable_diffusion".to_string(),
                    enabled: false, // Disabled by default
                    api_key: None,
                    api_key_env: None,
                    base_url: Some("http://127.0.0.1:7860".to_string()),
                    model: None,
                    size: Some("512x512".to_string()), // SD v1.5 default
                    quality: None,
                    style: None,
                    extra: HashMap::new(),
                },
            ],
        }
    }
}

/// Load config from a JSON file. Falls back to defaults if file is missing or invalid.
pub fn load_config(path: &Path) -> ImageGenSystemConfig {
    crate::config::load_json_config(path, "ImageGen")
}

/// Save config to a JSON file.
pub fn save_config(path: &Path, config: &ImageGenSystemConfig) -> Result<(), String> {
    crate::config::save_json_config(path, config, "ImageGen")
}
