//! LLM Service — managed Tauri state holding the active LLM provider.

use crate::error::KokoroError;
use crate::llm::llm_config::{LlmConfig, LlmPreset, LlmProviderConfig};
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
        let original_config = config.clone();
        let normalized_config = normalize_config(config);
        if normalized_config != original_config {
            tracing::warn!(
                target: "llm",
                "Normalized inconsistent LLM config: ensured selected providers are enabled and provider IDs are valid"
            );
            if let Err(error) = crate::llm::llm_config::save_config(&config_path, &normalized_config)
            {
                tracing::warn!(
                    target: "llm",
                    "Failed to persist normalized LLM config to {}: {}",
                    config_path.display(),
                    error
                );
            }
        }

        let providers = build_provider_map(&normalized_config);
        let active_provider_id = resolve_active_provider_id(&normalized_config)
            .map(str::to_owned)
            .unwrap_or_else(|| "openai".to_string());

        Self {
            providers: Arc::new(RwLock::new(providers)),
            active_provider_id: Arc::new(RwLock::new(active_provider_id)),
            config: Arc::new(RwLock::new(normalized_config)),
            config_path,
        }
    }

    /// Try get a clone of the active provider (Arc'd for async use).
    pub async fn try_provider(&self) -> Result<Arc<dyn LlmProvider>, KokoroError> {
        let active_id = self.active_provider_id.read().await.clone();
        let providers = self.providers.read().await;

        if providers.is_empty() {
            return Err(KokoroError::Config(
                "No available LLM provider: provider map is empty".to_string(),
            ));
        }

        providers.get(&active_id).cloned().ok_or_else(|| {
            KokoroError::Config(format!(
                "No available LLM provider: active provider '{}' is not configured",
                active_id
            ))
        })
    }

    /// Get a clone of the active provider (Arc'd for async use).
    pub async fn provider(&self) -> Arc<dyn LlmProvider> {
        self.try_provider().await.unwrap_or_else(|error| {
            tracing::error!(target: "llm", "Failed to resolve active provider: {}", error);
            default_provider()
        })
    }

    /// Get a clone of the current config.
    pub async fn config(&self) -> LlmConfig {
        self.config.read().await.clone()
    }

    /// Update config, persist to disk, and hot-swap the active provider.
    pub async fn update_config(&self, new_config: LlmConfig) -> Result<(), KokoroError> {
        let normalized_config = normalize_config(new_config);
        // Rebuild providers + active id first
        let rebuilt_providers = try_build_provider_map(&normalized_config)?;
        let rebuilt_active_provider_id = resolve_active_provider_id(&normalized_config)
            .map(str::to_owned)
            .unwrap_or_else(|| "openai".to_string());

        // Persist only after successful rebuild
        crate::llm::llm_config::save_config(&self.config_path, &normalized_config)?;

        // Swap only after successful rebuild + persistence
        *self.providers.write().await = rebuilt_providers;
        *self.active_provider_id.write().await = rebuilt_active_provider_id;
        *self.config.write().await = normalized_config;

        Ok(())
    }
    /// Get the system provider (or fallback to active).
    pub async fn system_provider(&self) -> Arc<dyn LlmProvider> {
        let config = self.config.read().await.clone();
        let active_id = self.active_provider_id.read().await.clone();
        let providers = self.providers.read().await;

        let resolved_id = config
            .system_provider
            .as_ref()
            .filter(|system_id| providers.contains_key(*system_id))
            .cloned()
            .unwrap_or(active_id);

        let resolved_provider =
            try_provider_by_id(&providers, &resolved_id).unwrap_or_else(|error| {
                tracing::error!(
                    target: "llm",
                    "Failed to resolve system provider {}: {}",
                    resolved_id,
                    error
                );
                default_provider()
            });

        if let Some(model_override) = config.system_model {
            if let Some(provider_config) = config
                .providers
                .iter()
                .find(|cfg| cfg.id == resolved_id && cfg.enabled)
            {
                let mut temporary_provider_config = provider_config.clone();
                temporary_provider_config.model = Some(model_override);
                return Arc::from(build_from_provider_config(&temporary_provider_config));
            }
        }

        resolved_provider
    }
}

fn try_provider_by_id(
    providers: &HashMap<String, Arc<dyn LlmProvider>>,
    provider_id: &str,
) -> Result<Arc<dyn LlmProvider>, KokoroError> {
    if providers.is_empty() {
        return Err(KokoroError::Config(
            "No available LLM provider: provider map is empty".to_string(),
        ));
    }

    providers.get(provider_id).cloned().ok_or_else(|| {
        KokoroError::Config(format!(
            "No available LLM provider: target provider '{}' is not configured",
            provider_id
        ))
    })
}

fn normalize_config(mut config: LlmConfig) -> LlmConfig {
    normalize_provider_selection(
        &mut config.active_provider,
        &mut config.system_provider,
        &mut config.providers,
    );

    for preset in &mut config.presets {
        normalize_preset(preset);
    }

    config
}

fn normalize_preset(preset: &mut LlmPreset) {
    normalize_provider_selection(
        &mut preset.active_provider,
        &mut preset.system_provider,
        &mut preset.providers,
    );
}

fn normalize_provider_selection(
    active_provider: &mut String,
    system_provider: &mut Option<String>,
    providers: &mut [LlmProviderConfig],
) {
    if let Some(active_index) = providers.iter().position(|provider| provider.id == *active_provider)
    {
        providers[active_index].enabled = true;
    } else if let Some(resolved_id) = providers
        .iter()
        .find(|provider| provider.enabled)
        .or_else(|| providers.first())
        .map(|provider| provider.id.clone())
    {
        *active_provider = resolved_id;
    }

    if let Some(system_id) = system_provider.clone() {
        if let Some(system_index) = providers.iter().position(|provider| provider.id == system_id)
        {
            providers[system_index].enabled = true;
        } else {
            *system_provider = None;
        }
    }
}

fn resolve_active_provider_id(config: &LlmConfig) -> Option<&str> {
    if config
        .providers
        .iter()
        .any(|p| p.id == config.active_provider && p.enabled)
    {
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
        .filter(|cfg| cfg.enabled)
        .map(|cfg| {
            (
                cfg.id.clone(),
                Arc::<dyn LlmProvider>::from(build_from_provider_config(cfg)),
            )
        })
        .collect()
}

fn default_provider() -> Arc<dyn LlmProvider> {
    Arc::new(OpenAIProvider::new(
        String::new(),
        Some("https://api.openai.com/v1".to_string()),
        Some("gpt-4".to_string()),
    ))
}

fn try_build_provider_map(
    config: &LlmConfig,
) -> Result<HashMap<String, Arc<dyn LlmProvider>>, KokoroError> {
    config
        .providers
        .iter()
        .filter(|cfg| cfg.enabled)
        .map(|cfg| {
            Ok((
                cfg.id.clone(),
                Arc::<dyn LlmProvider>::from(try_build_from_provider_config(cfg)?),
            ))
        })
        .collect()
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

fn try_build_from_provider_config(
    cfg: &LlmProviderConfig,
) -> Result<Box<dyn LlmProvider>, KokoroError> {
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
    async fn from_config_uses_first_enabled_when_active_provider_is_missing() {
        let config = LlmConfig {
            active_provider: "missing-provider".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![
                LlmProviderConfig {
                    id: "disabled-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: false,
                    supports_native_tools: true,
                    api_key: Some("test-key-disabled".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "enabled-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-enabled".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![],
        };

        let config_path = temp_config_path("llm_config_missing_active_provider");
        let service = LlmService::from_config(config, config_path.clone());

        let provider = service.provider().await;
        assert_eq!(provider.id(), "enabled-provider");

        let normalized_config = service.config().await;
        assert_eq!(normalized_config.active_provider, "enabled-provider");

        let persisted_config = crate::llm::llm_config::load_config(&config_path);
        assert_eq!(persisted_config.active_provider, "enabled-provider");

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn build_provider_map_excludes_disabled_providers() {
        let config = LlmConfig {
            active_provider: "enabled-provider".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![
                LlmProviderConfig {
                    id: "disabled-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: false,
                    supports_native_tools: true,
                    api_key: Some("test-key-disabled".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "enabled-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-enabled".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![],
        };

        let providers = build_provider_map(&config);

        assert_eq!(providers.len(), 1);
        assert!(providers.contains_key("enabled-provider"));
        assert!(!providers.contains_key("disabled-provider"));
    }

    #[tokio::test]
    async fn try_build_provider_map_excludes_disabled_providers() {
        let config = LlmConfig {
            active_provider: "enabled-provider".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![
                LlmProviderConfig {
                    id: "disabled-provider".to_string(),
                    provider_type: "unsupported-provider".to_string(),
                    enabled: false,
                    supports_native_tools: true,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    model: None,
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "enabled-provider".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: true,
                    supports_native_tools: true,
                    api_key: Some("test-key-enabled".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![],
        };

        let providers = try_build_provider_map(&config).unwrap();

        assert_eq!(providers.len(), 1);
        assert!(providers.contains_key("enabled-provider"));
        assert!(!providers.contains_key("disabled-provider"));
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
    async fn returns_explicit_error_when_no_available_provider() {
        let service = make_service_with_no_enabled_provider();

        let result = service.try_provider().await;

        assert!(result.is_err());
        let error_message = result.err().unwrap().to_string();
        assert!(error_message.contains("No available LLM provider"));
    }

    #[tokio::test]
    async fn provider_falls_back_to_default_when_no_available_provider() {
        let service = make_service_with_no_enabled_provider();

        let provider = service.provider().await;

        assert_eq!(provider.id(), "openai");
    }

    #[tokio::test]
    async fn system_provider_falls_back_to_default_when_no_available_provider() {
        let service = make_service_with_no_enabled_provider();

        let provider = service.system_provider().await;

        assert_eq!(provider.id(), "openai");
    }

    #[tokio::test]
    async fn from_config_normalizes_disabled_selected_providers_and_presets() {
        let config_path = temp_config_path("llm_config_from_config_normalization");
        let config = LlmConfig {
            active_provider: "ollama".to_string(),
            system_provider: Some("openai".to_string()),
            system_model: Some("qwen3-coder:30b".to_string()),
            providers: vec![
                LlmProviderConfig {
                    id: "openai".to_string(),
                    provider_type: "openai".to_string(),
                    enabled: false,
                    supports_native_tools: true,
                    api_key: Some("test-key-openai".to_string()),
                    api_key_env: None,
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    model: Some("gpt-4o-mini".to_string()),
                    extra: std::collections::HashMap::new(),
                },
                LlmProviderConfig {
                    id: "ollama".to_string(),
                    provider_type: "ollama".to_string(),
                    enabled: false,
                    supports_native_tools: true,
                    api_key: None,
                    api_key_env: None,
                    base_url: Some("http://localhost:11434".to_string()),
                    model: Some("qwen3-coder:30b".to_string()),
                    extra: std::collections::HashMap::new(),
                },
            ],
            presets: vec![LlmPreset {
                id: "preset-1".to_string(),
                name: "Broken preset".to_string(),
                active_provider: "ollama".to_string(),
                system_provider: Some("openai".to_string()),
                system_model: None,
                providers: vec![
                    LlmProviderConfig {
                        id: "openai".to_string(),
                        provider_type: "openai".to_string(),
                        enabled: false,
                        supports_native_tools: true,
                        api_key: Some("test-key-openai".to_string()),
                        api_key_env: None,
                        base_url: Some("https://api.openai.com/v1".to_string()),
                        model: Some("gpt-4o-mini".to_string()),
                        extra: std::collections::HashMap::new(),
                    },
                    LlmProviderConfig {
                        id: "ollama".to_string(),
                        provider_type: "ollama".to_string(),
                        enabled: false,
                        supports_native_tools: true,
                        api_key: None,
                        api_key_env: None,
                        base_url: Some("http://localhost:11434".to_string()),
                        model: Some("qwen3-coder:30b".to_string()),
                        extra: std::collections::HashMap::new(),
                    },
                ],
            }],
        };

        let service = LlmService::from_config(config, config_path.clone());

        let provider = service.provider().await;
        assert_eq!(provider.id(), "ollama");

        let system_provider = service.system_provider().await;
        assert_eq!(system_provider.id(), "openai");

        let normalized_config = service.config().await;
        assert!(normalized_config
            .providers
            .iter()
            .find(|provider| provider.id == "ollama")
            .unwrap()
            .enabled);
        assert!(normalized_config
            .providers
            .iter()
            .find(|provider| provider.id == "openai")
            .unwrap()
            .enabled);
        assert!(normalized_config.presets[0]
            .providers
            .iter()
            .find(|provider| provider.id == "ollama")
            .unwrap()
            .enabled);
        assert!(normalized_config.presets[0]
            .providers
            .iter()
            .find(|provider| provider.id == "openai")
            .unwrap()
            .enabled);

        let persisted_config = crate::llm::llm_config::load_config(&config_path);
        assert!(persisted_config
            .providers
            .iter()
            .find(|provider| provider.id == "ollama")
            .unwrap()
            .enabled);
        assert!(persisted_config.presets[0]
            .providers
            .iter()
            .find(|provider| provider.id == "openai")
            .unwrap()
            .enabled);

        let _ = std::fs::remove_file(config_path);
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

        let service =
            LlmService::from_config(test_config_with_named_providers(), config_path.clone());

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
        assert_eq!(
            config_after_failed_update.active_provider,
            "system-provider"
        );

        let providers_after_failed_update = service.providers.read().await;
        assert_eq!(providers_after_failed_update.len(), 2);
        assert!(providers_after_failed_update.contains_key("system-provider"));
        assert!(!providers_after_failed_update.contains_key("broken-provider"));

        let persisted_config = crate::llm::llm_config::load_config(&config_path);
        assert_eq!(persisted_config.active_provider, "system-provider");

        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn update_config_enables_selected_providers_before_persisting() {
        let config_path = temp_config_path("llm_config_update_config_normalization");
        let service =
            LlmService::from_config(test_config_with_named_providers(), config_path.clone());

        let mut new_config = test_config_with_named_providers();
        new_config.active_provider = "other-provider".to_string();
        new_config.system_provider = Some("system-provider".to_string());
        for provider in &mut new_config.providers {
            if provider.id == "other-provider" || provider.id == "system-provider" {
                provider.enabled = false;
            }
        }

        service.update_config(new_config).await.unwrap();

        let provider = service.provider().await;
        assert_eq!(provider.id(), "other-provider");

        let system_provider = service.system_provider().await;
        assert_eq!(system_provider.id(), "system-provider");

        let updated_config = service.config().await;
        assert!(updated_config
            .providers
            .iter()
            .find(|provider| provider.id == "other-provider")
            .unwrap()
            .enabled);
        assert!(updated_config
            .providers
            .iter()
            .find(|provider| provider.id == "system-provider")
            .unwrap()
            .enabled);

        let persisted_config = crate::llm::llm_config::load_config(&config_path);
        assert!(persisted_config
            .providers
            .iter()
            .find(|provider| provider.id == "other-provider")
            .unwrap()
            .enabled);
        assert!(persisted_config
            .providers
            .iter()
            .find(|provider| provider.id == "system-provider")
            .unwrap()
            .enabled);

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
        LlmService::from_config(
            config,
            temp_config_path("llm_config_missing_system_provider"),
        )
    }

    fn make_service_with_no_enabled_provider() -> LlmService {
        let config = LlmConfig {
            active_provider: "missing-provider".to_string(),
            system_provider: None,
            system_model: None,
            providers: vec![],
            presets: vec![],
        };

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

    fn temp_config_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{}_{}.json",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
