//! LLM Service — managed Tauri state holding the active LLM provider.

use crate::error::KokoroError;
use crate::llm::llm_config::{LlmConfig, LlmProviderConfig};
use crate::llm::ollama::OllamaProvider;
use crate::llm::provider::{LlmProvider, OpenAIProvider};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Managed state for LLM access. Holds provider map + active provider id + config.
#[derive(Clone)]
pub struct LlmService {
    providers: Arc<RwLock<HashMap<String, Arc<dyn LlmProvider>>>>,
    active_provider_id: Arc<RwLock<String>>,
    config: Arc<RwLock<LlmConfig>>,
    config_path: PathBuf,
}

impl LlmService {
    /// Create a new LlmService from a persisted config.
    pub fn from_config(config: LlmConfig, config_path: PathBuf) -> Self {
        let providers = build_provider_map(&config);
        let active_provider_id = resolve_active_provider_id(&config)
            .map(str::to_owned)
            .unwrap_or_else(|| "openai".to_string());

        Self {
            providers: Arc::new(RwLock::new(providers)),
            active_provider_id: Arc::new(RwLock::new(active_provider_id)),
            config: Arc::new(RwLock::new(config)),
            config_path,
        }
    }

    /// Get a clone of the active provider (Arc'd for async use).
    pub async fn provider(&self) -> Arc<dyn LlmProvider> {
        let active_id = self.active_provider_id.read().await.clone();
        let providers = self.providers.read().await;

        providers
            .get(&active_id)
            .cloned()
            .or_else(|| providers.values().next().cloned())
            .unwrap_or_else(default_provider)
    }

    /// Get a clone of the current config.
    pub async fn config(&self) -> LlmConfig {
        self.config.read().await.clone()
    }

    /// Update config, persist to disk, and hot-swap the active provider.
    pub async fn update_config(&self, new_config: LlmConfig) -> Result<(), KokoroError> {
        // Save to disk
        crate::llm::llm_config::save_config(&self.config_path, &new_config)?;

        // Rebuild providers + active id
        let providers = build_provider_map(&new_config);
        let active_provider_id = resolve_active_provider_id(&new_config)
            .map(str::to_owned)
            .unwrap_or_else(|| "openai".to_string());

        // Swap
        *self.providers.write().await = providers;
        *self.active_provider_id.write().await = active_provider_id;
        *self.config.write().await = new_config;

        Ok(())
    }
    /// Get the system provider (or fallback to active).
    pub async fn system_provider(&self) -> Arc<dyn LlmProvider> {
        let config = self.config.read().await;
        let system_id = config
            .system_provider
            .as_ref()
            .unwrap_or(&config.active_provider);

        // We can't easily reuse `build_provider` here without cloning config or restructuring.
        // For simplicity, we'll re-implement lookup logic or better yet, store all providers in a map.
        // BUT `LlmService` currently only holds the *active* provider instance.
        // To support multi-provider efficiently, we should probably refactor LlmService to hold a map of providers.
        // For now, let's just rebuild it on demand if it's different, OR (better) updated LlmService to hold a map.

        // Wait, `LlmService` struct:
        // provider: Arc<RwLock<Arc<dyn LlmProvider>>>,
        // This only holds ONE.

        // Refactoring LlmService to hold strict "active" is limiting.
        // Let's change `LlmService` to hold the config and build providers on-the-fly OR hold a cache.
        // Given the code structure, I will instantiate a new provider if `system_provider` is requested.
        // This is safe because `build_provider` is cheap (just struct creation).

        let provider_cfg = config
            .providers
            .iter()
            .find(|p| p.id == *system_id)
            .or_else(|| config.providers.iter().find(|p| p.enabled))
            .or_else(|| config.providers.first());

        if let Some(cfg) = provider_cfg {
            // Apply system_model override if present
            if let Some(ref model_override) = config.system_model {
                let mut overlaid_cfg = cfg.clone();
                overlaid_cfg.model = Some(model_override.clone());
                return Arc::from(build_from_provider_config(&overlaid_cfg));
            }
            return Arc::from(build_from_provider_config(cfg));
        }

        // Fallback
        self.provider().await
    }
}

fn resolve_active_provider_id(config: &LlmConfig) -> Option<&str> {
    if config.providers.iter().any(|p| p.id == config.active_provider) {
        Some(config.active_provider.as_str())
    } else if let Some(provider) = config.providers.iter().find(|p| p.enabled) {
        Some(provider.id.as_str())
    } else {
        config.providers.first().map(|p| p.id.as_str())
    }
}

fn build_provider_map(config: &LlmConfig) -> HashMap<String, Arc<dyn LlmProvider>> {
    config
        .providers
        .iter()
        .map(|cfg| {
            (
                cfg.id.clone(),
                Arc::<dyn LlmProvider>::from(build_from_provider_config(cfg)),
            )
        })
        .collect()
}

fn default_provider() -> Arc<dyn LlmProvider> {
    tracing::warn!(target: "llm", "No provider configured, falling back to OpenAI defaults");
    Arc::new(OpenAIProvider::new(
        String::new(),
        Some("https://api.openai.com/v1".to_string()),
        Some("gpt-4".to_string()),
    ))
}

fn build_from_provider_config(cfg: &LlmProviderConfig) -> Box<dyn LlmProvider> {
    match cfg.provider_type.as_str() {
        "ollama" => {
            let model = cfg.model.clone().unwrap_or_else(|| "llama3".to_string());
            tracing::info!(target: "llm", "Initializing Ollama provider: model={}", model);
            Box::new(OllamaProvider::new(cfg.base_url.clone(), model))
        }
        _ => {
            // "openai" or any OpenAI-compatible provider
            let api_key = cfg.resolve_api_key().unwrap_or_default();
            let model = cfg.model.clone().unwrap_or_else(|| "gpt-4".to_string());
            tracing::info!(
                target: "llm",
                "Initializing OpenAI provider: base_url={}, model={}",
                cfg.base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1"),
                model
            );
            Box::new(
                OpenAIProvider::new(api_key, cfg.base_url.clone(), Some(model))
                    .with_id(cfg.id.clone()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn from_config_builds_provider_map_and_returns_active_provider() {
        let (config, path) = test_llm_config_with_two_enabled_providers();
        let service = LlmService::from_config(config.clone(), path);

        let provider = service.provider().await;
        assert_eq!(provider.id(), config.active_provider);

        let providers = service.providers.read().await;
        assert_eq!(providers.len(), 2);
        assert!(providers.contains_key(&config.active_provider));

        let active_provider_id = service.active_provider_id.read().await.clone();
        assert_eq!(active_provider_id, config.active_provider);
    }

    fn test_llm_config_with_two_enabled_providers() -> (LlmConfig, PathBuf) {
        let config = LlmConfig {
            active_provider: "provider-b".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![
                LlmProviderConfig {
                    id: "provider-a".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-a".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "provider-b".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-b".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![],
        };

        (config, PathBuf::from("llm_config.test.json"))
    }
}
