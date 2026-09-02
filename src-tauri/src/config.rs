//! Shared config utilities for loading/saving JSON config files
//! and resolving API keys from fields or environment variables.
// pattern: Mixed (unavoidable)
// Reason: 该文件同时承载纯数据配置定义与文件系统读写封装，当前项目已将配置入口集中在这里，Phase 1 先做低侵入扩展。

use crate::error::KokoroError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// Helper to find candidate backup files for a config path in order of preference
fn find_backup_candidates(path: &Path) -> Vec<std::path::PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");
    let mut candidates = Vec::new();

    let dot_backup = parent.join(format!(".{file_name}.backup"));
    if dot_backup.is_file() {
        candidates.push(dot_backup);
    }
    let direct_backup = parent.join(format!("{file_name}.backup"));
    if direct_backup.is_file() {
        candidates.push(direct_backup);
    }

    if let Ok(entries) = std::fs::read_dir(parent) {
        let prefix = format!(".{file_name}.");
        let suffix = ".backup";
        let mut uuid_backups = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&prefix) && name.ends_with(suffix) && p.is_file() {
                    if let Ok(metadata) = p.metadata() {
                        let mtime = metadata
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        uuid_backups.push((p, mtime));
                    }
                }
            }
        }
        uuid_backups.sort_by(|a, b| b.1.cmp(&a.1));
        for (p, _) in uuid_backups {
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }

    candidates
}

/// Attempt to recover a missing or corrupted config from backup files.
fn try_recover_from_backup<T: DeserializeOwned>(path: &Path, label: &str) -> Option<T> {
    let candidates = find_backup_candidates(path);
    for candidate in candidates {
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            if let Ok(config) = serde_json::from_str::<T>(&content) {
                tracing::warn!(
                    target: "config",
                    "[{}] Recovered valid config from backup file {} to {}",
                    label,
                    candidate.display(),
                    path.display()
                );
                let _ = std::fs::copy(&candidate, path);
                return Some(config);
            }
        }
    }
    None
}

/// Generic load for any Serde config type with a `Default` implementation.
/// Falls back to `T::default()` if the file is missing or unparsable.
pub fn load_json_config<T: DeserializeOwned + Default>(path: &Path, label: &str) -> T {
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(config) => {
                tracing::debug!(target: "config", "[{}] Loaded config from {}", label, path.display());
                config
            }
            Err(e) => {
                tracing::warn!(
                    target: "config",
                    "[{}] Failed to parse config {}: {} — searching for backup",
                    label,
                    path.display(),
                    e
                );
                try_recover_from_backup::<T>(path, label).unwrap_or_default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            try_recover_from_backup::<T>(path, label).unwrap_or_default()
        }
        Err(e) => {
            tracing::warn!(
                target: "config",
                "[{}] Failed to read config {}: {} — searching for backup",
                label,
                path.display(),
                e
            );
            try_recover_from_backup::<T>(path, label).unwrap_or_default()
        }
    }
}

use uuid::Uuid;

/// Generic save for any Serde config type with atomic replace semantics.
pub fn save_json_config<T: Serialize>(
    path: &Path,
    config: &T,
    label: &str,
) -> Result<(), KokoroError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| KokoroError::Config(format!("Failed to create config directory: {}", e)))?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| KokoroError::Config(format!("Failed to serialize config: {}", e)))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let backup = parent.join(format!(".{file_name}.backup"));

    std::fs::write(&temporary, json.as_bytes()).map_err(|e| {
        KokoroError::Config(format!("Failed to write temporary config file: {}", e))
    })?;

    if path.exists() {
        let _ = std::fs::copy(path, &backup);
    }

    if let Err(e) = std::fs::rename(&temporary, path) {
        // Fallback for filesystem differences: try copy + remove
        if let Err(copy_err) = std::fs::copy(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(KokoroError::Config(format!(
                "Failed to persist config file (rename: {e}, copy: {copy_err})"
            )));
        }
        let _ = std::fs::remove_file(&temporary);
    }

    let _ = std::fs::remove_file(&backup);
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
#[serde(default)]
pub struct MemoryUpgradeConfig {
    pub observability_enabled: bool,
    pub event_trigger_enabled: bool,
    pub event_cooldown_secs: u64,
    pub structured_memory_enabled: bool,
    pub intent_routing_enabled: bool,
    pub retrieval_eval_enabled: bool,
    pub dreaming_enabled: bool,
    pub dream_auto_apply_level: String,
    pub dream_daily_hour: u8,
    pub dream_review_required_for_conflicts: bool,
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
            dreaming_enabled: true,
            dream_auto_apply_level: "aggressive".to_string(),
            dream_daily_hour: 3,
            dream_review_required_for_conflicts: true,
        }
    }
}

fn normalize_memory_upgrade_config(config: MemoryUpgradeConfig) -> MemoryUpgradeConfig {
    let auto_apply_level = match config.dream_auto_apply_level.as_str() {
        "conservative" | "review_only" | "aggressive" => config.dream_auto_apply_level,
        _ => "aggressive".to_string(),
    };

    MemoryUpgradeConfig {
        observability_enabled: true,
        event_trigger_enabled: true,
        event_cooldown_secs: config.event_cooldown_secs,
        structured_memory_enabled: true,
        intent_routing_enabled: true,
        retrieval_eval_enabled: true,
        dreaming_enabled: true,
        dream_auto_apply_level: auto_apply_level,
        dream_daily_hour: config.dream_daily_hour,
        dream_review_required_for_conflicts: true,
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
    if config.dream_daily_hour > 23 {
        return Err(KokoroError::Validation(
            "dream_daily_hour must be between 0 and 23".to_string(),
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
                dreaming_enabled: true,
                dream_auto_apply_level: "aggressive".to_string(),
                dream_daily_hour: 3,
                dream_review_required_for_conflicts: true,
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
    fn validate_memory_upgrade_config_forces_flags_enabled_and_preserves_valid_dream_level() {
        let config = validate_memory_upgrade_config(MemoryUpgradeConfig {
            observability_enabled: false,
            event_trigger_enabled: false,
            structured_memory_enabled: false,
            intent_routing_enabled: false,
            retrieval_eval_enabled: false,
            dreaming_enabled: false,
            dream_auto_apply_level: "review_only".to_string(),
            dream_review_required_for_conflicts: false,
            ..MemoryUpgradeConfig::default()
        })
        .expect("config should be normalized");

        assert_eq!(
            config,
            MemoryUpgradeConfig {
                dream_auto_apply_level: "review_only".to_string(),
                ..MemoryUpgradeConfig::default()
            }
        );
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

    #[test]
    fn save_json_config_writes_and_replaces_atomically() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("test_config.json");

        #[derive(Debug, Default, Serialize, serde::Deserialize, PartialEq, Eq)]
        struct TestData {
            message: String,
        }

        let initial = TestData {
            message: "hello".into(),
        };
        save_json_config(&path, &initial, "TEST").expect("save initial");
        let loaded: TestData = load_json_config(&path, "TEST");
        assert_eq!(loaded, initial);

        let updated = TestData {
            message: "world".into(),
        };
        save_json_config(&path, &updated, "TEST").expect("atomic replace");
        let loaded_updated: TestData = load_json_config(&path, "TEST");
        assert_eq!(loaded_updated, updated);

        // Ensure no temporary or backup files left behind
        let files: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files, vec!["test_config.json"]);
    }

    #[test]
    fn load_json_config_recovers_from_backup_when_target_missing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("app_config.json");
        let backup_path = temp_dir.path().join(".app_config.json.backup");

        #[derive(Debug, Default, Serialize, serde::Deserialize, PartialEq, Eq)]
        struct TestData {
            counter: u32,
        }

        let saved = TestData { counter: 42 };
        let json = serde_json::to_string(&saved).unwrap();
        std::fs::write(&backup_path, json).expect("write backup");

        assert!(!path.exists());
        let loaded: TestData = load_json_config(&path, "RECOVERY_TEST");
        assert_eq!(loaded, saved);
        // Target file should have been restored from backup
        assert!(path.exists());
    }

    #[test]
    fn load_json_config_recovers_from_backup_when_target_corrupted() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("app_config.json");
        let backup_path = temp_dir.path().join(".app_config.json.backup");

        #[derive(Debug, Default, Serialize, serde::Deserialize, PartialEq, Eq)]
        struct TestData {
            counter: u32,
        }

        let saved = TestData { counter: 99 };
        let json = serde_json::to_string(&saved).unwrap();
        std::fs::write(&backup_path, json).expect("write backup");
        std::fs::write(&path, "CORRUPTED_JSON_DATA{{{").expect("write corrupt target");

        let loaded: TestData = load_json_config(&path, "RECOVERY_TEST");
        assert_eq!(loaded, saved);
        // Target file should now contain recovered valid content
        let reloaded: TestData = load_json_config(&path, "RELOAD_TEST");
        assert_eq!(reloaded, saved);
    }
}
