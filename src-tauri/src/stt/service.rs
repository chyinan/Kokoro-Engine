//! STT Service — manages providers and routes transcription requests.

use super::config::{SttConfig, SttProviderConfig};
use super::interface::{SttError, SttProvider};
use super::openai::OpenAIWhisperProvider;
use super::whisper_cpp::WhisperCppProvider;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SttService {
    providers: Arc<RwLock<Vec<Box<dyn SttProvider>>>>,
    config: Arc<RwLock<SttConfig>>,
}

impl SttService {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(Vec::new())),
            config: Arc::new(RwLock::new(SttConfig::default())),
        }
    }

    /// Initialize from config, building all enabled providers.
    pub async fn init_from_config(config: &SttConfig) -> Self {
        let service = Self::new();
        {
            let mut cfg = service.config.write().await;
            *cfg = config.clone();
        }

        for provider_cfg in &config.providers {
            if provider_cfg.enabled {
                if let Some(provider) = Self::build_provider(provider_cfg) {
                    service.providers.write().await.push(provider);
                }
            }
        }

        let count = service.providers.read().await.len();
        println!("[STT] Initialized with {} provider(s)", count);
        service
    }

    /// Build a provider from config.
    fn build_provider(config: &SttProviderConfig) -> Option<Box<dyn SttProvider>> {
        match config.provider_type.as_str() {
            "openai_whisper" | "faster_whisper" | "local_whisper" => {
                let api_key = config.resolve_api_key().unwrap_or_default(); // Allow empty key for local
                Some(Box::new(OpenAIWhisperProvider::new(
                    config.id.clone(),
                    api_key,
                    config.base_url.clone(),
                    config.model.clone(),
                )))
            }
            "whisper_cpp" => Some(Box::new(WhisperCppProvider::new(config.base_url.clone()))),
            other => {
                eprintln!("[STT] Unknown provider type: {}", other);
                None
            }
        }
    }

    /// Transcribe audio using the active provider.
    /// If `language_override` is Some, use that; otherwise fall back to config language.
    pub async fn transcribe(
        &self,
        audio: &[u8],
        format: &str,
        language_override: Option<&str>,
    ) -> Result<String, SttError> {
        let config = self.config.read().await;
        let config_language = config.language.clone();
        let active_id = config.active_provider.clone();
        drop(config);

        let language = language_override.map(|s| s.to_string()).or(config_language);

        let providers = self.providers.read().await;

        // Find the active provider
        let provider = providers
            .iter()
            .find(|p| p.id() == active_id)
            .or_else(|| providers.first())
            .ok_or_else(|| SttError::ProviderNotFound("No STT providers configured".to_string()))?;

        provider
            .transcribe(audio, format, language.as_deref())
            .await
    }

    /// Get the current config.
    pub async fn get_config(&self) -> SttConfig {
        self.config.read().await.clone()
    }

    /// Hot-reload: update config and rebuild providers.
    pub async fn reload_from_config(&self, config: &SttConfig) {
        {
            let mut cfg = self.config.write().await;
            *cfg = config.clone();
        }

        let mut providers = self.providers.write().await;
        providers.clear();

        for provider_cfg in &config.providers {
            if provider_cfg.enabled {
                if let Some(provider) = Self::build_provider(provider_cfg) {
                    providers.push(provider);
                }
            }
        }

        println!("[STT] Reloaded with {} provider(s)", providers.len());
    }
}
