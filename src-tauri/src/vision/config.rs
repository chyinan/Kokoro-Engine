//! Vision configuration — persisted to disk.

use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_vlm_base_url(provider: &str) -> Option<String> {
    match provider {
        "llm" => None,
        "llama_cpp" => Some("http://127.0.0.1:8080".to_string()),
        "openai" => Some("https://api.openai.com/v1".to_string()),
        _ => Some("http://localhost:11434/v1".to_string()),
    }
}

fn default_vlm_model(provider: &str) -> String {
    match provider {
        "ollama" | "llama_cpp" => "minicpm-v".to_string(),
        "openai" => "gpt-4o".to_string(),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Whether real-time vision is enabled.
    pub enabled: bool,
    /// Capture interval in seconds.
    pub interval_secs: u32,
    /// Change threshold (0.0–1.0). Lower = more sensitive.
    pub change_threshold: f64,
    /// Whether screen changes should trigger proactive character comments.
    #[serde(default)]
    pub proactive_enabled: bool,

    // ── Independent VLM Provider ──────────────────────────
    /// Provider type: "ollama", "openai", "llama_cpp", or "llm" (use active LLM)
    pub vlm_provider: String,
    /// Base URL for the VLM API (e.g. "http://localhost:11434/v1")
    pub vlm_base_url: Option<String>,
    /// Model name (e.g. "minicpm-v", "moondream2", "gpt-4o")
    pub vlm_model: String,
    /// API key (only needed for online services)
    pub vlm_api_key: Option<String>,

    // ── Camera (Webcam) ───────────────────────────────
    /// Whether webcam capture is enabled.
    #[serde(default)]
    pub camera_enabled: bool,
    /// Preferred camera device ID (browser MediaDeviceInfo.deviceId).
    #[serde(default)]
    pub camera_device_id: Option<String>,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 15,
            change_threshold: 0.05,
            proactive_enabled: false,
            vlm_provider: "ollama".to_string(),
            vlm_base_url: default_vlm_base_url("ollama"),
            vlm_model: default_vlm_model("ollama"),
            vlm_api_key: None,
            camera_enabled: false,
            camera_device_id: None,
        }
    }
}

/// Load config from disk, falling back to defaults.
pub fn load_config(path: &Path) -> VisionConfig {
    match std::fs::read_to_string(path) {
        Ok(json) => {
            let mut cfg: VisionConfig = serde_json::from_str(&json).unwrap_or_default();
            if cfg.vlm_base_url.is_none() {
                cfg.vlm_base_url = default_vlm_base_url(&cfg.vlm_provider);
            }
            // Heal empty model name that can occur when switching from "llm" provider
            // without saving a new model name first.
            if cfg.vlm_model.is_empty() && cfg.vlm_provider != "llm" {
                cfg.vlm_model = default_vlm_model(&cfg.vlm_provider);
            }
            cfg
        }
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

#[cfg(test)]
mod tests {
    use super::load_config;
    use tempfile::tempdir;

    #[test]
    fn load_config_heals_llama_cpp_defaults() {
        let temp = tempdir().expect("temp dir should be created");
        let path = temp.path().join("vision_config.json");

        std::fs::write(
            &path,
            serde_json::json!({
                "enabled": false,
                "interval_secs": 15,
                "change_threshold": 0.05,
                "vlm_provider": "llama_cpp",
                "vlm_base_url": null,
                "vlm_model": "",
                "vlm_api_key": null,
                "camera_enabled": false,
                "camera_device_id": null
            })
            .to_string(),
        )
        .expect("test config should be written");

        let config = load_config(&path);

        assert_eq!(
            config.vlm_base_url.as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(config.vlm_model, "minicpm-v");
    }
}
