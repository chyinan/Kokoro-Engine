//! STT Service — manages providers and routes transcription requests.

use super::config::{
    default_providers_pub, load_config, save_config, SttConfig, SttProviderConfig,
};
use super::interface::{AudioSource, SttEngine, SttError, TranscriptionResult};
use super::openai::OpenAIWhisperProvider;
use super::sensevoice::SenseVoiceProvider;
use super::sensevoice_local::SenseVoiceLocalProvider;
use super::whisper_cpp::WhisperCppProvider;
use crate::error::KokoroError;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SttService {
    providers: Arc<RwLock<Vec<Arc<dyn SttEngine>>>>,
    config: Arc<RwLock<SttConfig>>,
}

impl Default for SttService {
    fn default() -> Self {
        Self::new()
    }
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
        tracing::info!(target: "stt", "Initialized with {} provider(s)", count);
        service
    }

    /// Build a provider from config.
    fn build_provider(config: &SttProviderConfig) -> Option<Arc<dyn SttEngine>> {
        match config.provider_type.as_str() {
            "openai_whisper" | "faster_whisper" | "local_whisper" => {
                let api_key = config.resolve_api_key().unwrap_or_default();
                match OpenAIWhisperProvider::new(
                    config.id.clone(),
                    api_key,
                    config.base_url.clone(),
                    config.model.clone(),
                ) {
                    Ok(provider) => Some(Arc::new(provider)),
                    Err(e) => {
                        tracing::error!(
                            target: "stt",
                            "Failed to build OpenAI Whisper provider '{}': {}",
                            config.id,
                            e
                        );
                        None
                    }
                }
            }
            "whisper_cpp" => Some(Arc::new(WhisperCppProvider::new(config.base_url.clone()))),
            "sensevoice" => {
                match SenseVoiceProvider::new(config.id.clone(), config.base_url.clone()) {
                    Ok(provider) => Some(Arc::new(provider)),
                    Err(e) => {
                        tracing::error!(
                            target: "stt",
                            "Failed to build SenseVoice provider '{}': {}",
                            config.id,
                            e
                        );
                        None
                    }
                }
            }
            "sensevoice_local" => Some(Arc::new(SenseVoiceLocalProvider::new(config, None))),
            other => {
                tracing::error!(target: "stt", "Unknown provider type: {}", other);
                None
            }
        }
    }

    /// Transcribe audio using the active provider.
    /// If `language_override` is Some, use that; otherwise fall back to config language.
    pub async fn transcribe(
        &self,
        audio: &AudioSource,
        language_override: Option<&str>,
    ) -> Result<TranscriptionResult, SttError> {
        let config = self.config.read().await;
        let config_language = config.language.clone();

        let language = language_override.map(|s| s.to_string()).or(config_language);

        let provider = {
            let providers = self.providers.read().await;

            // Find the active provider
            providers
                .iter()
                .find(|p| p.id() == config.active_provider)
                .or_else(|| providers.first())
                .cloned()
                .ok_or_else(|| {
                    SttError::ProviderNotFound("No STT providers configured".to_string())
                })?
        };
        drop(config);

        // Lock is released here, so we can await safely without blocking
        provider.transcribe(audio, language.as_deref()).await
    }

    /// Get the current config.
    pub async fn get_config(&self) -> SttConfig {
        self.config.read().await.clone()
    }

    /// Merge missing defaults without letting a stale disk read overwrite a save.
    pub async fn load_config_with_defaults(&self, path: &Path) -> SttConfig {
        let _config = self.config.write().await;
        let mut config = load_config(path);
        let mut changed = false;
        for provider in default_providers_pub() {
            if !config.providers.iter().any(|p| p.id == provider.id) {
                config.providers.push(provider);
                changed = true;
            }
        }
        if changed {
            if let Err(error) = save_config(path, &config) {
                tracing::warn!(target: "stt", "Failed to persist merged default STT providers: {error}");
            }
        }
        config
    }

    /// Called on the native mic worker. A settings commit and the actual
    /// interrupt start must observe one another, even after slow device setup.
    pub(crate) fn with_interrupt_enabled(
        &self,
        start: impl FnOnce() -> Result<bool, String>,
    ) -> Result<bool, String> {
        let config = self.config.blocking_read();
        if config.auto_send
            && config
                .providers
                .iter()
                .any(|provider| provider.id == config.active_provider && provider.enabled)
        {
            start()
        } else {
            Ok(false)
        }
    }

    /// Prepare providers before committing the settings to disk and runtime.
    pub async fn save_and_reload(&self, path: &Path, config: SttConfig) -> Result<(), KokoroError> {
        let mut new_providers: Vec<Arc<dyn SttEngine>> = Vec::new();

        for provider_cfg in &config.providers {
            if provider_cfg.enabled {
                if let Some(provider) = Self::build_provider(provider_cfg) {
                    new_providers.push(provider);
                }
            }
        }

        if new_providers.is_empty() && config.providers.iter().any(|provider| provider.enabled) {
            tracing::error!(
                target: "stt",
                "Reload skipped: no valid providers built; keeping existing runtime providers"
            );
            return Err(
                SttError::ConfigError("no valid STT providers built from config".into()).into(),
            );
        }

        let mut current_config = self.config.write().await;
        let mut providers = self.providers.write().await;
        save_config(path, &config)?;
        *providers = new_providers;
        *current_config = config;
        tracing::info!(target: "stt", "Reloaded with {} provider(s)", providers.len());

        Ok(())
    }
}
