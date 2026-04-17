//! Shared config utilities for loading/saving JSON config files
//! and resolving API keys from fields or environment variables.
// pattern: Mixed (unavoidable)
// Reason: 该文件同时承载纯数据配置定义与文件系统读写封装，当前项目已将配置入口集中在这里，Phase 1 先做低侵入扩展。

use crate::error::KokoroError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// Generic load for any Serde config type with a `Default` implementation.
/// Falls back to `T::default()` if the file is missing or unparsable.
pub fn load_json_config<T: DeserializeOwned + Default>(path: &Path, label: &str) -> T {
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(config) => {
                tracing::info!(target: "config", "[{}] Loaded config from {}", label, path.display());
                config
            }
            Err(e) => {
                tracing::warn!(
                    target: "config",
                    "[{}] Failed to parse config {}: {} — using defaults",
                    label,
                    path.display(),
                    e
                );
                T::default()
            }
        },
        Err(_) => {
            tracing::info!(
                target: "config",
                "[{}] No config file at {} — using defaults",
                label,
                path.display()
            );
            T::default()
        }
    }
}

/// Generic save for any Serde config type.
pub fn save_json_config<T: Serialize>(
    path: &Path,
    config: &T,
    label: &str,
) -> Result<(), KokoroError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            KokoroError::Config(format!("Failed to create config directory: {}", e))
        })?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| KokoroError::Config(format!("Failed to serialize config: {}", e)))?;
    std::fs::write(path, json)
        .map_err(|e| KokoroError::Config(format!("Failed to write config file: {}", e)))?;
    tracing::info!(target: "config", "[{}] Saved config to {}", label, path.display());
    Ok(())
}

/// Resolve an API key: check the direct `api_key` field first,
/// then fall back to reading the environment variable named in `api_key_env`.
pub fn resolve_api_key(api_key: &Option<String>, api_key_env: &Option<String>) -> Option<String> {
    if let Some(ref key) = api_key {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }
    if let Some(ref env_var) = api_key_env {
        if let Ok(key) = std::env::var(env_var) {
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MemoryUpgradeConfig {
    pub observability_enabled: bool,
    pub event_trigger_enabled: bool,
    pub event_cooldown_secs: u64,
    pub structured_memory_enabled: bool,
    pub intent_routing_enabled: bool,
    pub retrieval_eval_enabled: bool,
}

impl Default for MemoryUpgradeConfig {
    fn default() -> Self {
        Self {
            observability_enabled: true,
            event_trigger_enabled: true,
            event_cooldown_secs: 120,
            structured_memory_enabled: true,
            intent_routing_enabled: true,
            retrieval_eval_enabled: true,
        }
    }
}

fn normalize_memory_upgrade_config(config: MemoryUpgradeConfig) -> MemoryUpgradeConfig {
    MemoryUpgradeConfig {
        observability_enabled: true,
        event_trigger_enabled: true,
        event_cooldown_secs: config.event_cooldown_secs,
        structured_memory_enabled: true,
        intent_routing_enabled: true,
        retrieval_eval_enabled: true,
    }
}

pub fn validate_memory_upgrade_config(
    config: MemoryUpgradeConfig,
) -> Result<MemoryUpgradeConfig, KokoroError> {
    if config.event_cooldown_secs == 0 {
        return Err(KokoroError::Validation(
            "event_cooldown_secs must be greater than 0".to_string(),
        ));
    }

    Ok(normalize_memory_upgrade_config(config))
}

pub fn load_memory_upgrade_config(path: &Path) -> MemoryUpgradeConfig {
    let config = load_json_config(path, "MEMORY_UPGRADE");
    validate_memory_upgrade_config(config).unwrap_or_default()
}

pub fn save_memory_upgrade_config(
    path: &Path,
    config: &MemoryUpgradeConfig,
) -> Result<(), KokoroError> {
    let validated = validate_memory_upgrade_config(config.clone())?;
    save_json_config(path, &validated, "MEMORY_UPGRADE")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_upgrade_config_defaults_include_event_cooldown() {
        let config = MemoryUpgradeConfig::default();

        assert_eq!(config.event_trigger_enabled, true);
        assert_eq!(config.event_cooldown_secs, 120);
        assert_eq!(
            config,
            MemoryUpgradeConfig {
                observability_enabled: true,
                event_trigger_enabled: true,
                event_cooldown_secs: 120,
                structured_memory_enabled: true,
                intent_routing_enabled: true,
                retrieval_eval_enabled: true,
            }
        );
    }

    #[test]
    fn validate_memory_upgrade_config_rejects_zero_event_cooldown() {
        let error = validate_memory_upgrade_config(MemoryUpgradeConfig {
            event_cooldown_secs: 0,
            ..MemoryUpgradeConfig::default()
        })
        .expect_err("config should be rejected");

        match error {
            KokoroError::Validation(message) => {
                assert_eq!(message, "event_cooldown_secs must be greater than 0");
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn validate_memory_upgrade_config_forces_flags_enabled() {
        let config = validate_memory_upgrade_config(MemoryUpgradeConfig {
            observability_enabled: false,
            event_trigger_enabled: false,
            structured_memory_enabled: false,
            intent_routing_enabled: false,
            retrieval_eval_enabled: false,
            ..MemoryUpgradeConfig::default()
        })
        .expect("config should be normalized");

        assert_eq!(config, MemoryUpgradeConfig::default());
    }

    #[test]
    fn load_memory_upgrade_config_falls_back_to_default_for_invalid_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("memory_upgrade_config.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "observability_enabled": false,
                "retrieval_eval_enabled": true
            })
            .to_string(),
        )
        .expect("write config");

        let config = load_memory_upgrade_config(&path);

        assert_eq!(config, MemoryUpgradeConfig::default());
    }

    #[test]
    fn save_memory_upgrade_config_normalizes_flags_to_enabled() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("memory_upgrade_config.json");
        save_memory_upgrade_config(
            &path,
            &MemoryUpgradeConfig {
                observability_enabled: false,
                event_trigger_enabled: false,
                structured_memory_enabled: false,
                intent_routing_enabled: false,
                retrieval_eval_enabled: false,
                ..MemoryUpgradeConfig::default()
            },
        )
        .expect("save should normalize");

        let config = load_memory_upgrade_config(&path);
        assert_eq!(config, MemoryUpgradeConfig::default());
    }
}
