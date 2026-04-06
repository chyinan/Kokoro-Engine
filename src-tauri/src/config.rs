//! Shared config utilities for loading/saving JSON config files
//! and resolving API keys from fields or environment variables.

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
