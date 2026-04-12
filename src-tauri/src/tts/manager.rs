use super::azure::AzureTtsProvider;
use super::browser::BrowserTTSProvider;
use super::cache::{CacheKey, TtsCache};
use super::cloud_base::CloudTTSProvider;
use super::config::{ProviderConfig, TtsSystemConfig};
use super::edge::EdgeTtsProvider;
use super::interface::{ProviderCapabilities, TtsError, TtsParams, TtsProvider, VoiceProfile};
use super::local_gpt_sovits::LocalGPTSoVITSProvider;
use super::local_rvc::LocalRVCProvider;
use super::local_vits::LocalVITSProvider;
use super::openai::OpenAITtsProvider;
use super::queue::TtsQueue;
use super::router::TtsRouter;
use super::voice_registry::VoiceRegistry;

use crate::hooks::{HookEvent, HookPayload, HookRuntime, TtsHookPayload};
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
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

impl Default for TtsService {
    fn default() -> Self {
        Self::new()
    }
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
                tracing::info!(target: "tts", "Skipping disabled provider: {}", provider_config.id);
                continue;
            }

            match Self::build_provider(provider_config).await {
                Some(provider) => {
                    tracing::info!(target: "tts", "Registering provider: {}", provider_config.id);
                    service.register_provider(provider).await;
                }
                None => {
                    tracing::error!(
                        target: "tts",
                        "Failed to build provider '{}' (type: {}). Check config and API keys.",
                        provider_config.id, provider_config.provider_type
                    );
                }
            }
        }

        service
    }

    /// Build a provider from config.
    async fn build_provider(config: &ProviderConfig) -> Option<Box<dyn TtsProvider>> {
        match config.provider_type.as_str() {
            "openai" => {
                OpenAITtsProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "edge_tts" => EdgeTtsProvider::from_config(config)
                .await
                .map(|p| Box::new(p) as Box<dyn TtsProvider>),
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
                AzureTtsProvider::from_config(config).map(|p| Box::new(p) as Box<dyn TtsProvider>)
            }
            "elevenlabs" => CloudTTSProvider::elevenlabs_style(config)
                .map(|p| Box::new(p) as Box<dyn TtsProvider>),
            other => {
                tracing::error!(target: "tts", "Unknown provider type: {}", other);
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

        let hook_runtime = app.try_state::<HookRuntime>();

        // Route to the best provider
        let router = TtsRouter::new(self.providers.clone(), self.default_provider.clone());
        let route = router
            .select_provider(
                provider_id.as_deref(),
                params.required_capabilities.as_ref(),
            )
            .await
            .map_err(|e| e.to_string())?;

        if let Some(hooks) = hook_runtime.as_ref() {
            hooks
                .emit_best_effort(
                    &HookEvent::BeforeTtsPlay,
                    &HookPayload::Tts(TtsHookPayload {
                        text: text.clone(),
                        provider_id: Some(route.provider_id.clone()),
                    }),
                )
                .await;
        }

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
                                tracing::error!(target: "tts", "Stream error for '{}': {}", sentence, e);
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
                    tracing::error!(target: "tts", "{}", e);
                }
            }
        }

        // Emit End
        app.emit("tts:end", TtsEndEvent { text: text.clone() })
            .map_err(|e| e.to_string())?;

        if let Some(hooks) = hook_runtime.as_ref() {
            hooks
                .emit_best_effort(
                    &HookEvent::AfterTtsPlay,
                    &HookPayload::Tts(TtsHookPayload {
                        text,
                        provider_id: Some(route.provider_id.clone()),
                    }),
                )
                .await;
        }

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

    /// Hot-reload: rebuild providers first, then atomically swap runtime state.
    pub async fn reload_from_config(&self, config: &TtsSystemConfig) -> Result<(), TtsError> {
        let mut new_providers: HashMap<String, Box<dyn TtsProvider>> = HashMap::new();
        let mut new_registry = VoiceRegistry::new();
        let mut first_provider_id: Option<String> = None;

        for provider_config in &config.providers {
            if !provider_config.enabled {
                tracing::info!(target: "tts", "Skipping disabled provider: {}", provider_config.id);
                continue;
            }

            match Self::build_provider(provider_config).await {
                Some(provider) => {
                    tracing::info!(target: "tts", "Registering provider: {}", provider_config.id);
                    let provider_id = provider.id();
                    if first_provider_id.is_none() {
                        first_provider_id = Some(provider_id.clone());
                    }
                    new_registry.register_all(provider.voices());
                    new_providers.insert(provider_id, provider);
                }
                None => {
                    tracing::error!(
                        target: "tts",
                        "Failed to build provider '{}' (type: {})",
                        provider_config.id, provider_config.provider_type
                    );
                }
            }
        }

        if new_providers.is_empty() {
            tracing::error!(
                target: "tts",
                "Reload skipped: no valid providers built; keeping existing runtime providers"
            );
            return Err(TtsError::ConfigError(
                "no valid TTS providers built from config; runtime providers unchanged".to_string(),
            ));
        }

        let mut new_default = config.default_provider.clone();
        if let Some(default_id) = new_default.as_ref() {
            if !new_providers.contains_key(default_id) {
                tracing::warn!(
                    target: "tts",
                    "Configured default provider '{}' is unavailable after reload; falling back",
                    default_id
                );
                new_default = first_provider_id.clone();
            }
        } else {
            new_default = first_provider_id.clone();
        }

        {
            let mut providers = self.providers.write().await;
            *providers = new_providers;
        }
        {
            let mut registry = self.voice_registry.write().await;
            *registry = new_registry;
        }
        {
            let mut default = self.default_provider.write().await;
            *default = new_default;
        }

        // Clear cache since providers changed
        self.clear_cache().await;
        tracing::info!(
            target: "tts",
            "Reloaded {} providers from config",
            self.providers.read().await.len()
        );

        Ok(())
    }

    /// Synthesize text to raw audio bytes without requiring an AppHandle.
    /// Used by services that need audio data directly (e.g. Telegram bot).
    pub async fn synthesize_text(
        &self,
        text: &str,
        params: Option<TtsParams>,
    ) -> Result<Vec<u8>, String> {
        let params = params.unwrap_or_default();
        let router = TtsRouter::new(self.providers.clone(), self.default_provider.clone());
        let route = router
            .select_provider(None, params.required_capabilities.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        let providers = self.providers.read().await;
        let provider = providers
            .get(&route.provider_id)
            .ok_or_else(|| format!("Provider {} not found", route.provider_id))?;

        let mut stream = provider
            .synthesize_stream(text, params)
            .await
            .map_err(|e| e.to_string())?;

        let mut audio = Vec::new();
        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(chunk) => audio.extend_from_slice(&chunk),
                Err(e) => return Err(format!("TTS stream error: {}", e)),
            }
        }
        Ok(audio)
    }

    /// Clear the synthesis cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    // 支持中英文标点分句
    let mut result = Vec::new();
    let mut last = 0;
    for (i, c) in text.char_indices() {
        match c {
            '.' | '!' | '?' | '。' | '！' | '？' => {
                let end = i + c.len_utf8();
                let segment = &text[last..end];
                if !segment.trim().is_empty() {
                    result.push(segment);
                }
                last = end;
            }
            _ => {}
        }
    }
    // 处理末尾没有标点的剩余文本
    if last < text.len() {
        let remaining = &text[last..];
        if !remaining.trim().is_empty() {
            result.push(remaining);
        }
    }
    result
}
