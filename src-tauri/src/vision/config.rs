//! Vision configuration — persisted to disk.

use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_vlm_base_url(provider: &str) -> Option<String> {
    match provider {
        "llm" => None,
        "anthropic" => Some("https://api.anthropic.com/v1".to_string()),
        "llama_cpp" => Some("http://127.0.0.1:8080".to_string()),
        "openai" => Some("https://api.openai.com/v1".to_string()),
        _ => Some("http://localhost:11434/v1".to_string()),
    }
}

fn default_vlm_model(provider: &str) -> String {
    match provider {
        "ollama" | "llama_cpp" => "minicpm-v".to_string(),
        "anthropic" => "claude-sonnet-4-20250514".to_string(),
        "openai" => "gpt-4o".to_string(),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub fn clamped(self) -> Option<Self> {
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        let max_width = 1.0 - x;
        let max_height = 1.0 - y;
        let width = self.width.clamp(0.0, max_width);
        let height = self.height.clamp(0.0, max_height);
        (width > 0.0 && height > 0.0).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Master screen/VLM capability gate.
    pub vlm_enabled: bool,
    /// Whether background screen observation is enabled.
    pub auto_vision_enabled: bool,
    /// Whether completed observations can trigger proactive comments.
    pub proactive_vision_enabled: bool,
    /// How much persisted screen context should be included in LLM history: "latest" or "full".
    #[serde(default = "default_vision_context_history_mode")]
    pub vision_context_history_mode: String,
    /// Capture interval in seconds.
    pub capture_interval_secs: u32,
    /// Change threshold (0.0–1.0). Lower = more sensitive.
    pub change_threshold: f64,
    /// Optional display identifier to capture.
    #[serde(default)]
    pub display_id: Option<String>,
    /// Optional normalized region to crop before analysis.
    #[serde(default)]
    pub vlm_region: Option<NormalizedRect>,

    // ── Independent VLM Provider ──────────────────────────
    /// Provider type: "ollama", "openai", "anthropic", "llama_cpp", or "llm" (use active LLM)
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
            vlm_enabled: false,
            auto_vision_enabled: false,
            proactive_vision_enabled: false,
            vision_context_history_mode: default_vision_context_history_mode(),
            capture_interval_secs: 15,
            change_threshold: 0.05,
            display_id: None,
            vlm_region: None,
            vlm_provider: "ollama".to_string(),
            vlm_base_url: default_vlm_base_url("ollama"),
            vlm_model: default_vlm_model("ollama"),
            vlm_api_key: None,
            camera_enabled: false,
            camera_device_id: None,
        }
    }
}

pub fn default_vision_context_history_mode() -> String {
    "latest".to_string()
}

pub fn normalize_vision_context_history_mode(mode: &str) -> String {
    if mode == "full" {
        "full".to_string()
    } else {
        "latest".to_string()
    }
}

fn bool_field(
    value: &serde_json::Value,
    new_key: &str,
    old_key: Option<&str>,
    default: bool,
) -> bool {
    value
        .get(new_key)
        .or_else(|| old_key.and_then(|key| value.get(key)))
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

fn u32_field(value: &serde_json::Value, new_key: &str, old_key: Option<&str>, default: u32) -> u32 {
    value
        .get(new_key)
        .or_else(|| old_key.and_then(|key| value.get(key)))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(default)
}

fn f64_field(value: &serde_json::Value, key: &str, default: f64) -> f64 {
    value
        .get(key)
        .and_then(|value| value.as_f64())
        .unwrap_or(default)
}

fn string_field(value: &serde_json::Value, key: &str, default: String) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .unwrap_or(default)
}

fn option_string_field(
    value: &serde_json::Value,
    key: &str,
    default: Option<String>,
) -> Option<String> {
    match value.get(key) {
        Some(serde_json::Value::String(value)) => Some(value.clone()),
        Some(serde_json::Value::Null) => None,
        Some(_) => default,
        None => default,
    }
}

fn migrate_config_value(value: serde_json::Value) -> VisionConfig {
    let defaults = VisionConfig::default();
    let old_enabled = value.get("enabled").and_then(|value| value.as_bool());
    let provider = string_field(&value, "vlm_provider", defaults.vlm_provider.clone());
    let mut cfg = VisionConfig {
        vlm_enabled: bool_field(&value, "vlm_enabled", Some("enabled"), defaults.vlm_enabled),
        auto_vision_enabled: value
            .get("auto_vision_enabled")
            .and_then(|value| value.as_bool())
            .or(old_enabled)
            .unwrap_or(defaults.auto_vision_enabled),
        proactive_vision_enabled: bool_field(
            &value,
            "proactive_vision_enabled",
            Some("proactive_enabled"),
            defaults.proactive_vision_enabled,
        ),
        vision_context_history_mode: normalize_vision_context_history_mode(
            value
                .get("vision_context_history_mode")
                .and_then(|value| value.as_str())
                .unwrap_or(&defaults.vision_context_history_mode),
        ),
        capture_interval_secs: u32_field(
            &value,
            "capture_interval_secs",
            Some("interval_secs"),
            defaults.capture_interval_secs,
        ),
        change_threshold: f64_field(&value, "change_threshold", defaults.change_threshold),
        display_id: option_string_field(&value, "display_id", defaults.display_id),
        vlm_region: value
            .get("vlm_region")
            .and_then(|value| serde_json::from_value::<NormalizedRect>(value.clone()).ok())
            .and_then(NormalizedRect::clamped),
        vlm_provider: provider.clone(),
        vlm_base_url: option_string_field(&value, "vlm_base_url", default_vlm_base_url(&provider)),
        vlm_model: string_field(&value, "vlm_model", default_vlm_model(&provider)),
        vlm_api_key: option_string_field(&value, "vlm_api_key", defaults.vlm_api_key),
        camera_enabled: bool_field(&value, "camera_enabled", None, defaults.camera_enabled),
        camera_device_id: option_string_field(
            &value,
            "camera_device_id",
            defaults.camera_device_id,
        ),
    };

    if cfg.vlm_base_url.is_none() {
        cfg.vlm_base_url = default_vlm_base_url(&cfg.vlm_provider);
    }
    if cfg.vlm_model.is_empty() && cfg.vlm_provider != "llm" {
        cfg.vlm_model = default_vlm_model(&cfg.vlm_provider);
    }
    cfg
}

/// Load config from disk, falling back to defaults.
pub fn load_config(path: &Path) -> VisionConfig {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str::<serde_json::Value>(&json)
            .map(migrate_config_value)
            .unwrap_or_default(),
        Err(_) => VisionConfig::default(),
    }
}

/// Save config to disk.
pub fn save_config(path: &Path, config: &VisionConfig) -> Result<(), String> {
    let mut config = config.clone();
    config.vlm_region = config.vlm_region.and_then(NormalizedRect::clamped);
    config.vision_context_history_mode =
        normalize_vision_context_history_mode(&config.vision_context_history_mode);
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize vision config: {}", e))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to write vision config: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load_config, save_config, NormalizedRect};
    use tempfile::tempdir;

    #[test]
    fn load_config_migrates_legacy_fields() {
        let temp = tempdir().expect("temp dir should be created");
        let path = temp.path().join("vision_config.json");

        std::fs::write(
            &path,
            serde_json::json!({
                "enabled": true,
                "interval_secs": 7,
                "change_threshold": 0.12,
                "proactive_enabled": true,
                "vlm_provider": "llama_cpp",
                "vlm_base_url": null,
                "vlm_model": "",
                "vlm_api_key": "secret",
                "camera_enabled": true,
                "camera_device_id": "cam-1"
            })
            .to_string(),
        )
        .expect("test config should be written");

        let config = load_config(&path);

        assert!(config.vlm_enabled);
        assert!(config.auto_vision_enabled);
        assert!(config.proactive_vision_enabled);
        assert_eq!(config.vision_context_history_mode, "latest");
        assert_eq!(config.capture_interval_secs, 7);
        assert_eq!(config.change_threshold, 0.12);
        assert_eq!(
            config.vlm_base_url.as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(config.vlm_model, "minicpm-v");
        assert_eq!(config.vlm_api_key.as_deref(), Some("secret"));
        assert!(config.camera_enabled);
        assert_eq!(config.camera_device_id.as_deref(), Some("cam-1"));
    }

    #[test]
    fn save_config_writes_v2_fields_only() {
        let temp = tempdir().expect("temp dir should be created");
        let path = temp.path().join("vision_config.json");
        let mut config = load_config(&path);
        config.vlm_enabled = true;
        config.auto_vision_enabled = false;
        config.capture_interval_secs = 22;
        config.vision_context_history_mode = "full".to_string();

        save_config(&path, &config).expect("config should save");
        let raw = std::fs::read_to_string(&path).expect("config should be readable");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("config JSON");

        assert_eq!(value["vlm_enabled"], true);
        assert_eq!(value["auto_vision_enabled"], false);
        assert_eq!(value["capture_interval_secs"], 22);
        assert_eq!(value["vision_context_history_mode"], "full");
        assert!(value.get("enabled").is_none());
        assert!(value.get("interval_secs").is_none());
        assert!(value.get("proactive_enabled").is_none());
    }

    #[test]
    fn normalized_rect_clamps_and_rejects_empty() {
        assert_eq!(
            NormalizedRect {
                x: -0.5,
                y: 0.5,
                width: 2.0,
                height: 0.8
            }
            .clamped(),
            Some(NormalizedRect {
                x: 0.0,
                y: 0.5,
                width: 1.0,
                height: 0.5
            })
        );
        assert!(NormalizedRect {
            x: 1.0,
            y: 0.0,
            width: 0.5,
            height: 0.5
        }
        .clamped()
        .is_none());
    }
}
