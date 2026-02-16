//! Vision configuration — persisted to disk.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Whether real-time vision is enabled.
    pub enabled: bool,
    /// Capture interval in seconds.
    pub interval_secs: u32,
    /// Change threshold (0.0–1.0). Lower = more sensitive.
    pub change_threshold: f64,

    // ── Independent VLM Provider ──────────────────────────
    /// Provider type: "ollama" or "openai"
    pub vlm_provider: String,
    /// Base URL for the VLM API (e.g. "http://localhost:11434/v1")
    pub vlm_base_url: Option<String>,
    /// Model name (e.g. "minicpm-v", "moondream2", "gpt-4o")
    pub vlm_model: String,
    /// API key (only needed for online services)
    pub vlm_api_key: Option<String>,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 15,
            change_threshold: 0.05,
            vlm_provider: "ollama".to_string(),
            vlm_base_url: Some("http://localhost:11434/v1".to_string()),
            vlm_model: "minicpm-v".to_string(),
            vlm_api_key: None,
        }
    }
}

/// Load config from disk, falling back to defaults.
pub fn load_config(path: &Path) -> VisionConfig {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => VisionConfig::default(),
    }
}

/// Save config to disk.
pub fn save_config(path: &Path, config: &VisionConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize vision config: {}", e))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to write vision config: {}", e))?;
    Ok(())
}
