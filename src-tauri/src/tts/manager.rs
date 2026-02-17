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

use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock; // Add this

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
        let sentences: Vec<String> = split_sentences(&text)
            .into_iter()
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
            .collect();

        // Pipelined synthesis: Concurrency = 2
        // We iterate over sentences, map them to async synthesis tasks, and buffer them.
        // buffered(n) ensures we have at most n tasks running, but yields results IN ORDER.
        let service = self.clone();
        let service_for_cache = self.clone();
        let app_handle = app.clone();
        let provider_id_route = route.provider_id.clone();
        let params_clone = params.clone();

        let mut stream = futures::stream::iter(sentences)
            .map(move |sentence| {
                let service = service.clone();
                let params = params_clone.clone();
                let provider_id = provider_id_route.clone();

                async move {
                    let voice_id = params.voice.clone().unwrap_or_default();
                    let cache_key = CacheKey::new(
                        &sentence,
                        &voice_id,
                        &provider_id,
                        params.speed,
                        params.pitch,
                    );

                    // 1. Check cache
                    if service.cache_enabled {
                        let mut cache = service.cache.write().await;
                        if let Some(cached_audio) = cache.get(&cache_key) {
                            let stream = futures::stream::once(async move { Ok(cached_audio) });
                            return Ok((
                                sentence,
                                Some(Box::pin(stream)
                                    as Pin<
                                        Box<
                                            dyn futures::Stream<Item = Result<Vec<u8>, TtsError>>
                                                + Send,
                                        >,
                                    >),
                                None,
                                Some(cache_key),
                            ));
                            // (text, stream, delegate, cache_key)
                            // Note: we pass cache_key even on hit, but we won't overwrite cache.
                        }
                    }

                    // 2. Synthesize
                    let providers = service.providers.read().await;
                    let provider = providers
                        .get(&provider_id)
                        .ok_or_else(|| format!("Provider {} not found", provider_id))?;

                    match provider.synthesize_stream(&sentence, params.clone()).await {
                        Ok(stream) => Ok((sentence, Some(stream), None, Some(cache_key))),
                        Err(TtsError::BrowserDelegate) => {
                            let evt = TtsBrowserDelegateEvent {
                                text: sentence.clone(),
                                voice: params.voice.clone(),
                                speed: params.speed,
                                pitch: params.pitch,
                            };
                            Ok((sentence, None, Some(evt), None))
                        }
                        Err(e) => Err(format!("Synthesis error for '{}': {}", sentence, e)),
                    }
                }
            })
            .buffered(2); // Pipeline depth

        // Process results in order
        while let Some(result) = stream.next().await {
            match result {
                Ok((sentence, Some(mut audio_stream), _, cache_key_opt)) => {
                    let mut full_audio = Vec::new();
                    let mut failed = false;

                    while let Some(chunk_res) = audio_stream.next().await {
                        match chunk_res {
                            Ok(chunk) => {
                                full_audio.extend_from_slice(&chunk);
                                app_handle
                                    .emit("tts:audio", TtsAudioEvent { data: chunk })
                                    .map_err(|e| e.to_string())?;
                            }
                            Err(e) => {
                                eprintln!("[TTS] Stream error for '{}': {}", sentence, e);
                                failed = true;
                                break;
                            }
                        }
                    }

                    // Cache if successful and not already cached
                    if !failed && !full_audio.is_empty() {
                        if let Some(key) = cache_key_opt {
                            if service_for_cache.cache_enabled {
                                // Only write to cache if it wasn't a hit?
                                // Actually we don't know if it was a hit inside here unless we track it.
                                // But overwriting with same data is harmless but wasteful lock.
                                // We can check if it exists implicitly or just rely on the fact that
                                // if it was a hit, we just streamed it back.
                                // Optimization: check if we need to cache.
                                // Implementation detail: TtsCache::put overwrites.
                                // Let's just put it.
                                let mut cache = service_for_cache.cache.write().await;
                                cache.put(key, full_audio);
                            }
                        }
                    }
                }
                Ok((_text, None, Some(delegate_evt), _)) => {
                    app_handle
                        .emit("tts:browser-delegate", delegate_evt)
                        .map_err(|e| e.to_string())?;
                }
                Ok(_) => {} // Should not happen
                Err(e) => {
                    eprintln!("[TTS] {}", e);
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
