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

        // Rebuild providers + active id first
        let rebuilt_providers = try_build_provider_map(&new_config)?;
        let rebuilt_active_provider_id = resolve_active_provider_id(&new_config)
            .map(str::to_owned)
            .unwrap_or_else(|| "openai".to_string());

        // Swap only after successful rebuild
        *self.providers.write().await = rebuilt_providers;
        *self.active_provider_id.write().await = rebuilt_active_provider_id;
        *self.config.write().await = new_config;

        Ok(())
    }
    /// Get the system provider (or fallback to active).
    pub async fn system_provider(&self) -> Arc<dyn LlmProvider> {
        let config = self.config.read().await.clone();
        let active_id = self.active_provider_id.read().await.clone();
        let providers = self.providers.read().await;

        let resolved_provider = config
            .system_provider
            .as_ref()
            .and_then(|system_id| providers.get(system_id).cloned())
            .or_else(|| providers.get(&active_id).cloned())
            .or_else(|| providers.values().next().cloned())
            .unwrap_or_else(default_provider);

        if let Some(model_override) = config.system_model {
            let resolved_id = config
                .system_provider
                .as_ref()
                .filter(|system_id| providers.contains_key(*system_id))
                .cloned()
                .unwrap_or(active_id);

            if let Some(provider_config) = config.providers.iter().find(|cfg| cfg.id == resolved_id) {
                let mut temporary_provider_config = provider_config.clone();
                temporary_provider_config.model = Some(model_override);
                return Arc::from(build_from_provider_config(&temporary_provider_config));
            }
        }

        resolved_provider
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

fn try_build_provider_map(
    config: &LlmConfig,
) -> Result<HashMap<String, Arc<dyn LlmProvider>>, KokoroError> {
    config
        .providers
        .iter()
        .map(|cfg| {
            Ok((
                cfg.id.clone(),
                Arc::<dyn LlmProvider>::from(try_build_from_provider_config(cfg)?),
            ))
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
    try_build_from_provider_config(cfg).unwrap_or_else(|error| {
        tracing::warn!(
            target: "llm",
            "Failed to build provider {}: {}. Falling back to OpenAI-compatible provider",
            cfg.id,
            error
        );

        let api_key = cfg.resolve_api_key().unwrap_or_default();
        let model = cfg.model.clone().unwrap_or_else(|| "gpt-4".to_string());
        Box::new(
            OpenAIProvider::new(api_key, cfg.base_url.clone(), Some(model)).with_id(cfg.id.clone()),
        )
    })
}

fn try_build_from_provider_config(cfg: &LlmProviderConfig) -> Result<Box<dyn LlmProvider>, KokoroError> {
    match cfg.provider_type.as_str() {
        "ollama" => {
            let model = cfg.model.clone().unwrap_or_else(|| "llama3".to_string());
            tracing::info!(target: "llm", "Initializing Ollama provider: model={}", model);
            Ok(Box::new(OllamaProvider::new(cfg.base_url.clone(), model)))
        }
        "openai" => {
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
            Ok(Box::new(
                OpenAIProvider::new(api_key, cfg.base_url.clone(), Some(model))
                    .with_id(cfg.id.clone()),
            ))
        }
        unsupported => Err(KokoroError::Config(format!(
            "Unsupported LLM provider type: {}",
            unsupported
        ))),
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

    #[tokio::test]
    async fn system_provider_prefers_system_provider_id_when_present() {
        let service = make_service_with_active_and_system_provider();
        let expected = {
            let providers = service.providers.read().await;
            providers.get("system-provider").cloned().unwrap()
        };

        let provider = service.system_provider().await;

        assert_eq!(provider.id(), "system-provider");
        assert!(Arc::ptr_eq(&provider, &expected));
    }

    #[tokio::test]
    async fn system_provider_falls_back_to_active_when_system_missing() {
        let service = make_service_with_missing_system_provider();
        let expected_active = {
            let providers = service.providers.read().await;
            providers.get("active-provider").cloned().unwrap()
        };

        let provider = service.system_provider().await;

        assert_eq!(provider.id(), "active-provider");
        assert!(Arc::ptr_eq(&provider, &expected_active));
    }

    #[tokio::test]
    async fn update_config_rebuilds_provider_map_and_switches_active_consistently() {
        let config_path = std::env::temp_dir().join(format!(
            "llm_config_update_config_atomic_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let service = LlmService::from_config(test_config_with_named_providers(), config_path.clone());

        let mut new_config = test_config_with_named_providers();
        new_config.active_provider = "system-provider".to_string();
        new_config.providers.retain(|p| p.id != "other-provider");

        service.update_config(new_config.clone()).await.unwrap();

        let updated_provider = service.provider().await;
        assert_eq!(updated_provider.id(), "system-provider");

        let updated_config = service.config().await;
        assert_eq!(updated_config.active_provider, "system-provider");

        let updated_providers = service.providers.read().await;
        assert_eq!(updated_providers.len(), 2);
        assert!(updated_providers.contains_key("system-provider"));
        assert!(!updated_providers.contains_key("other-provider"));
        drop(updated_providers);

        let mut invalid_config = new_config;
        invalid_config.active_provider = "broken-provider".to_string();
        invalid_config.providers = vec![LlmProviderConfig {
            id: "broken-provider".to_string(),
            provider_type: "unsupported-provider".to_string(),
            enabled: true,
            supports_native_tools: true,
            api_key: None,
            api_key_env: None,
            base_url: None,
            model: None,
            extra: std::collections::HashMap::new(),
        }];

        let result = service.update_config(invalid_config).await;
        assert!(result.is_err());

        let provider_after_failed_update = service.provider().await;
        assert_eq!(provider_after_failed_update.id(), "system-provider");

        let config_after_failed_update = service.config().await;
        assert_eq!(config_after_failed_update.active_provider, "system-provider");

        let providers_after_failed_update = service.providers.read().await;
        assert_eq!(providers_after_failed_update.len(), 2);
        assert!(providers_after_failed_update.contains_key("system-provider"));
        assert!(!providers_after_failed_update.contains_key("broken-provider"));

        let _ = std::fs::remove_file(config_path);
    }

    fn make_service_with_active_and_system_provider() -> LlmService {
        let mut config = test_config_with_named_providers();
        config.active_provider = "active-provider".to_string();
        config.system_provider = Some("system-provider".to_string());
        LlmService::from_config(config, PathBuf::from("llm_config.test.json"))
    }

    fn make_service_with_missing_system_provider() -> LlmService {
        let mut config = test_config_with_named_providers();
        config.active_provider = "active-provider".to_string();
        config.system_provider = Some("missing-system-provider".to_string());
        LlmService::from_config(config, PathBuf::from("llm_config.test.json"))
    }

    fn test_config_with_named_providers() -> LlmConfig {
        LlmConfig {
            active_provider: "active-provider".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![
                LlmProviderConfig {
                    id: "other-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-other".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "active-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-active".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "system-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-system".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4.1-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![],
        }
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
