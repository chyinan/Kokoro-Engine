use crate::error::KokoroError;
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
pub fn save_config(path: &Path, config: &TtsSystemConfig) -> Result<(), KokoroError> {
    crate::config::save_json_config(path, config, "TTS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_config_defaults() {
        let cache = CacheConfig::default();
        assert!(cache.enabled, "Cache should be enabled by default");
        assert_eq!(cache.max_entries, 500, "Default max_entries should be 500");
        assert_eq!(cache.ttl_secs, 3600, "Default TTL should be 3600 seconds (1 hour)");
    }

    #[test]
    fn test_queue_config_defaults() {
        let queue = QueueConfig::default();
        assert_eq!(
            queue.max_concurrent, 3,
            "Default max_concurrent should be 3"
        );
    }

    #[test]
    fn test_tts_system_config_defaults() {
        let config = TtsSystemConfig::default();
        assert_eq!(
            config.default_provider,
            Some("browser".to_string()),
            "Default provider should be 'browser'"
        );
        assert!(
            config.cache.enabled,
            "Cache should be enabled in default config"
        );
        assert_eq!(
            config.queue.max_concurrent, 3,
            "Queue max_concurrent should be 3 in default config"
        );
        assert_eq!(
            config.providers.len(),
            1,
            "Default config should have exactly one provider"
        );
        assert_eq!(
            config.providers[0].id, "browser",
            "Default provider should be browser"
        );
        assert_eq!(
            config.providers[0].provider_type, "browser",
            "Default provider type should be browser"
        );
        assert!(
            config.providers[0].enabled,
            "Default browser provider should be enabled"
        );
    }

    #[test]
    fn test_tts_system_config_serde_roundtrip() {
        let original = TtsSystemConfig::default();
        let json = serde_json::to_string(&original)
            .expect("Failed to serialize TtsSystemConfig");
        let deserialized: TtsSystemConfig =
            serde_json::from_str(&json).expect("Failed to deserialize TtsSystemConfig");

        assert_eq!(
            original.default_provider, deserialized.default_provider,
            "default_provider should match after roundtrip"
        );
        assert_eq!(
            original.cache.enabled, deserialized.cache.enabled,
            "cache.enabled should match after roundtrip"
        );
        assert_eq!(
            original.cache.max_entries, deserialized.cache.max_entries,
            "cache.max_entries should match after roundtrip"
        );
        assert_eq!(
            original.cache.ttl_secs, deserialized.cache.ttl_secs,
            "cache.ttl_secs should match after roundtrip"
        );
        assert_eq!(
            original.queue.max_concurrent, deserialized.queue.max_concurrent,
            "queue.max_concurrent should match after roundtrip"
        );
        assert_eq!(
            original.providers.len(),
            deserialized.providers.len(),
            "providers length should match after roundtrip"
        );
    }

    #[test]
    fn test_provider_config_defaults() {
        let provider = ProviderConfig {
            id: "test".to_string(),
            provider_type: "openai".to_string(),
            enabled: true,
            api_key: None,
            api_key_env: None,
            base_url: None,
            endpoint: None,
            model: None,
            default_voice: None,
            model_path: None,
            extra: HashMap::new(),
        };

        assert_eq!(provider.id, "test", "Provider ID should be set");
        assert_eq!(provider.provider_type, "openai", "Provider type should be set");
        assert!(provider.enabled, "Provider should be enabled");
        assert_eq!(provider.api_key, None, "API key should be None");
    }
}
