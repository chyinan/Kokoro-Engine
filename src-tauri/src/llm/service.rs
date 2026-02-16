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
