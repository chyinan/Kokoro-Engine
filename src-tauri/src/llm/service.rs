//! LLM Service — managed Tauri state holding the active LLM provider.

use crate::llm::llm_config::{LlmConfig, LlmProviderConfig};
use crate::llm::ollama::OllamaProvider;
use crate::llm::provider::{LlmProvider, OpenAIProvider};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Managed state for LLM access. Holds the active provider + config.
#[derive(Clone)]
pub struct LlmService {
    provider: Arc<RwLock<Arc<dyn LlmProvider>>>,
    config: Arc<RwLock<LlmConfig>>,
    config_path: PathBuf,
}

impl LlmService {
    /// Create a new LlmService from a persisted config.
    pub fn from_config(config: LlmConfig, config_path: PathBuf) -> Self {
        let provider: Arc<dyn LlmProvider> = Arc::from(build_provider(&config));
        Self {
            provider: Arc::new(RwLock::new(provider)),
            config: Arc::new(RwLock::new(config)),
            config_path,
        }
    }

    /// Get a clone of the active provider (Arc'd for async use).
    pub async fn provider(&self) -> Arc<dyn LlmProvider> {
        self.provider.read().await.clone()
    }

    /// Get a clone of the current config.
    pub async fn config(&self) -> LlmConfig {
        self.config.read().await.clone()
    }

    /// Update config, persist to disk, and hot-swap the active provider.
    pub async fn update_config(&self, new_config: LlmConfig) -> Result<(), String> {
        // Save to disk
        crate::llm::llm_config::save_config(&self.config_path, &new_config)?;

        // Build new provider
        let new_provider: Arc<dyn LlmProvider> = Arc::from(build_provider(&new_config));

        // Swap
        *self.provider.write().await = new_provider;
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
        self.provider.read().await.clone()
    }
}

/// Factory: build the appropriate LlmProvider from config.
fn build_provider(config: &LlmConfig) -> Box<dyn LlmProvider> {
    let active_id = &config.active_provider;

    let provider_cfg = config
        .providers
        .iter()
        .find(|p| p.id == *active_id)
        .or_else(|| config.providers.iter().find(|p| p.enabled))
        .or_else(|| config.providers.first());

    match provider_cfg {
        Some(cfg) => build_from_provider_config(cfg),
        None => {
            eprintln!("[LLM] No provider configured, falling back to OpenAI defaults");
            Box::new(OpenAIProvider::new(
                String::new(),
                Some("https://api.openai.com/v1".to_string()),
                Some("gpt-4".to_string()),
            ))
        }
    }
}

fn build_from_provider_config(cfg: &LlmProviderConfig) -> Box<dyn LlmProvider> {
    match cfg.provider_type.as_str() {
        "ollama" => {
            let model = cfg.model.clone().unwrap_or_else(|| "llama3".to_string());
            println!("[LLM] Initializing Ollama provider: model={}", model);
            Box::new(OllamaProvider::new(cfg.base_url.clone(), model))
        }
        _ => {
            // "openai" or any OpenAI-compatible provider
            let api_key = cfg.resolve_api_key().unwrap_or_default();
            let model = cfg.model.clone().unwrap_or_else(|| "gpt-4".to_string());
            println!(
                "[LLM] Initializing OpenAI provider: base_url={}, model={}",
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
