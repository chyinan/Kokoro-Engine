use super::browser::BrowserTTSProvider;
use super::cache::{CacheKey, TtsCache};
use super::cloud_base::CloudTTSProvider;
use super::config::{ProviderConfig, TtsSystemConfig};
use super::interface::{ProviderCapabilities, TtsError, TtsParams, TtsProvider, VoiceProfile};
use super::local_gpt_sovits::LocalGPTSoVITSProvider;
use super::local_rvc::LocalRVCProvider;
use super::local_vits::LocalVITSProvider;
use super::openai::OpenAITtsProvider;
use super::queue::TtsQueue;
use super::router::TtsRouter;
use super::voice_registry::VoiceRegistry;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

// ── Tauri Event Payloads ───────────────────────────────

#[derive(Clone, Serialize)]
struct TtsStartEvent {
    text: String,
}

#[derive(Clone, Serialize)]
struct TtsAudioEvent {
    data: Vec<u8>,
}

#[derive(Clone, Serialize)]
struct TtsEndEvent {
    text: String,
}

#[derive(Clone, Serialize)]
struct TtsBrowserDelegateEvent {
    text: String,
    voice: Option<String>,
    speed: Option<f32>,
    pitch: Option<f32>,
}

// ── Provider Status (for frontend queries) ─────────────

#[derive(Clone, Serialize)]
pub struct ProviderStatus {
    pub id: String,
    pub available: bool,
    pub capabilities: ProviderCapabilities,
}

// ── TtsService ─────────────────────────────────────────

#[derive(Clone)]
pub struct TtsService {
    providers: Arc<RwLock<HashMap<String, Box<dyn TtsProvider>>>>,
    default_provider: Arc<RwLock<Option<String>>>,
    voice_registry: Arc<RwLock<VoiceRegistry>>,
    cache: Arc<RwLock<TtsCache>>,
    _queue: Arc<TtsQueue>,
    cache_enabled: bool,
}

impl TtsService {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            default_provider: Arc::new(RwLock::new(None)),
            voice_registry: Arc::new(RwLock::new(VoiceRegistry::new())),
            cache: Arc::new(RwLock::new(TtsCache::new(500, 3600))),
            _queue: Arc::new(TtsQueue::new(3)),
            cache_enabled: true,
        }
    }

    /// Initialize TtsService from a config, building and registering all providers.
    pub async fn init_from_config(config: &TtsSystemConfig) -> Self {
        let service = Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            default_provider: Arc::new(RwLock::new(config.default_provider.clone())),
            voice_registry: Arc::new(RwLock::new(VoiceRegistry::new())),
            cache: Arc::new(RwLock::new(TtsCache::new(
                config.cache.max_entries,
                config.cache.ttl_secs,
            ))),
            _queue: Arc::new(TtsQueue::new(config.queue.max_concurrent)),
            cache_enabled: config.cache.enabled,
        };

        for provider_config in &config.providers {
            if !provider_config.enabled {
                println!("[TTS] Skipping disabled provider: {}", provider_config.id);
                continue;
            }

            match Self::build_provider(provider_config) {
                Some(provider) => {
                    println!("[TTS] Registering provider: {}", provider_config.id);
                    service.register_provider(provider).await;
                }
                None => {
                    eprintln!(
                        "[TTS] Failed to build provider '{}' (type: {}). Check config and API keys.",
                        provider_config.id, provider_config.provider_type
                    );
                }
            }
        }

        service
    }

    /// Build a provider from config.
    fn build_provider(config: &ProviderConfig) -> Option<Box<dyn TtsProvider>> {
        match config.provider_type.as_str() {
            "openai" => {
                OpenAITtsProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "browser" => {
                BrowserTTSProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "local_vits" => {
                LocalVITSProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "gpt_sovits" => LocalGPTSoVITSProvider::from_config(config)
                .map(|p| Box::new(p) as Box<dyn TtsProvider>),
            "local_rvc" => {
                LocalRVCProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "azure" => {
                CloudTTSProvider::azure_style(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "elevenlabs" => CloudTTSProvider::elevenlabs_style(config)
                .map(|p| Box::new(p) as Box<dyn TtsProvider>),
            other => {
                eprintln!("[TTS] Unknown provider type: {}", other);
                None
            }
        }
    }

    /// Register a provider and its voices.
    pub async fn register_provider(&self, provider: Box<dyn TtsProvider>) {
        let id = provider.id();
        let voices = provider.voices();

        // Register voices
        {
            let mut registry = self.voice_registry.write().await;
            registry.register_all(voices);
        }

        // Set as default if it's the first one and no default is configured
        {
            let providers = self.providers.read().await;
            if providers.is_empty() {
                let mut default = self.default_provider.write().await;
                if default.is_none() {
                    *default = Some(id.clone());
                }
            }
        }

        let mut providers = self.providers.write().await;
        providers.insert(id, provider);
    }

    /// Main synthesis method with cache → queue → route → synthesize pipeline.
    pub async fn speak(
        &self,
        app: AppHandle,
        text: String,
        provider_id: Option<String>,
        params: Option<TtsParams>,
    ) -> Result<(), String> {
        let params = params.unwrap_or_default();

        // Route to the best provider
        let router = TtsRouter::new(self.providers.clone(), self.default_provider.clone());
        let route = router
            .select_provider(
                provider_id.as_deref(),
                params.required_capabilities.as_ref(),
            )
            .await
            .map_err(|e| e.to_string())?;

        // Emit Start
        app.emit("tts:start", TtsStartEvent { text: text.clone() })
            .map_err(|e| e.to_string())?;

        // Split into sentences for incremental delivery
        let sentences = split_sentences(&text);

        for sentence in sentences {
            if sentence.trim().is_empty() {
                continue;
            }

            let voice_id = params.voice.clone().unwrap_or_default();
            let cache_key = CacheKey::new(
                sentence,
                &voice_id,
                &route.provider_id,
                params.speed,
                params.pitch,
            );

            // Check cache first
            if self.cache_enabled {
                let mut cache = self.cache.write().await;
                if let Some(cached_audio) = cache.get(&cache_key) {
                    app.emit("tts:audio", TtsAudioEvent { data: cached_audio })
                        .map_err(|e| e.to_string())?;
                    continue;
                }
            }

            // Synthesize via provider
            let providers = self.providers.read().await;
            let provider = providers
                .get(&route.provider_id)
                .ok_or(format!("Provider {} not found", route.provider_id))?;

            let synth_params = params.clone();
            match provider.synthesize(sentence, synth_params).await {
                Ok(audio_data) => {
                    // Cache the result
                    if self.cache_enabled {
                        let mut cache = self.cache.write().await;
                        cache.put(cache_key, audio_data.clone());
                    }
                    // Emit audio chunk
                    app.emit("tts:audio", TtsAudioEvent { data: audio_data })
                        .map_err(|e| e.to_string())?;
                }
                Err(TtsError::BrowserDelegate) => {
                    // Emit browser delegate event instead of audio
                    app.emit(
                        "tts:browser-delegate",
                        TtsBrowserDelegateEvent {
                            text: sentence.to_string(),
                            voice: params.voice.clone(),
                            speed: params.speed,
                            pitch: params.pitch,
                        },
                    )
                    .map_err(|e| e.to_string())?;
                }
                Err(e) => {
                    eprintln!("[TTS] Synthesis error for sentence: {}. Skipping.", e);
                    // Don't fallback to browser — local providers (e.g. GPT-SoVITS)
                    // may legitimately take a long time for large text.
                }
            }
        }

        // Emit End
        app.emit("tts:end", TtsEndEvent { text })
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    // ── Query methods ──────────────────────────────────

    /// List all registered provider IDs with their status.
    pub async fn list_providers(&self) -> Vec<ProviderStatus> {
        let providers = self.providers.read().await;
        let mut statuses = Vec::new();
        for (id, provider) in providers.iter() {
            statuses.push(ProviderStatus {
                id: id.clone(),
                available: provider.is_available().await,
                capabilities: provider.capabilities(),
            });
        }
        statuses
    }

    /// List all registered voices.
    pub async fn list_voices(&self) -> Vec<VoiceProfile> {
        let registry = self.voice_registry.read().await;
        registry.list().into_iter().cloned().collect()
    }

    /// Get status for a specific provider.
    pub async fn get_provider_status(&self, id: &str) -> Option<ProviderStatus> {
        let providers = self.providers.read().await;
        if let Some(provider) = providers.get(id) {
            Some(ProviderStatus {
                id: id.to_string(),
                available: provider.is_available().await,
                capabilities: provider.capabilities(),
            })
        } else {
            None
        }
    }

    /// Hot-reload: clear all providers and re-initialize from a new config.
    pub async fn reload_from_config(&self, config: &TtsSystemConfig) {
        // Clear existing providers and voice registry
        {
            let mut providers = self.providers.write().await;
            providers.clear();
        }
        {
            let mut registry = self.voice_registry.write().await;
            *registry = VoiceRegistry::new();
        }
        {
            let mut default = self.default_provider.write().await;
            *default = config.default_provider.clone();
        }

        // Re-register enabled providers
        for provider_config in &config.providers {
            if !provider_config.enabled {
                println!("[TTS] Skipping disabled provider: {}", provider_config.id);
                continue;
            }
            match Self::build_provider(provider_config) {
                Some(provider) => {
                    println!("[TTS] Registering provider: {}", provider_config.id);
                    self.register_provider(provider).await;
                }
                None => {
                    eprintln!(
                        "[TTS] Failed to build provider '{}' (type: {})",
                        provider_config.id, provider_config.provider_type
                    );
                }
            }
        }

        // Clear cache since providers changed
        self.clear_cache().await;
        println!(
            "[TTS] Reloaded {} providers from config",
            self.providers.read().await.len()
        );
    }

    /// Clear the synthesis cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split_inclusive(&['.', '!', '?'][..]).collect()
}
