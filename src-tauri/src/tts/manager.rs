use super::azure::AzureTtsProvider;
use super::browser::BrowserTTSProvider;
use super::cache::{CacheKey, TtsCache};
use super::cloud_base::CloudTTSProvider;
use super::config::{ProviderConfig, TtsSystemConfig};
use super::edge::EdgeTtsProvider;
use super::interface::{
    AudioFormat, ProviderCapabilities, TtsAudioChunk, TtsAudioStream, TtsError, TtsParams,
    TtsProvider, VoiceProfile,
};
use super::local_gpt_sovits::LocalGPTSoVITSProvider;
use super::local_vits::LocalVITSProvider;
use super::omnivoice::OmniVoiceProvider;
use super::openai::OpenAITtsProvider;
use super::openai_realtime::OpenAIRealtimeTtsProvider;
use super::queue::TtsQueue;
use super::router::TtsRouter;
use super::voice_registry::VoiceRegistry;

use crate::chat::runtime::ChatRuntimeState;
use crate::hooks::{HookEvent, HookPayload, HookRuntime, TtsHookPayload};
use futures::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Mutex, RwLock};

const MAX_CANCELLED_TTS_REQUESTS: usize = 128;

// ── Tauri Event Payloads ───────────────────────────────

#[derive(Clone, Serialize)]
struct TtsStartEvent {
    request_id: String,
    text: String,
}

#[derive(Clone, Serialize)]
struct TtsAudioEvent {
    request_id: String,
    sentence_id: usize,
    sequence: u64,
    response_id: Option<String>,
    format: Option<AudioFormat>,
    data: Vec<u8>,
}

#[derive(Clone, Serialize)]
struct TtsEndEvent {
    request_id: String,
    text: String,
    status: TtsFinishStatus,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum TtsFinishStatus {
    Completed,
    Cancelled,
    Error,
}

#[derive(Clone, Serialize)]
struct TtsErrorEvent {
    request_id: String,
    error: String,
}

#[derive(Clone, Serialize)]
struct TtsBrowserDelegateEvent {
    request_id: String,
    sentence_id: usize,
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
    providers: Arc<RwLock<HashMap<String, Arc<dyn TtsProvider>>>>,
    default_provider: Arc<RwLock<Option<String>>>,
    voice_registry: Arc<RwLock<VoiceRegistry>>,
    cache: Arc<RwLock<TtsCache>>,
    requests: Arc<Mutex<TtsRequests>>,
    _queue: Arc<TtsQueue>,
    cache_enabled: bool,
}

struct ActiveTtsRequest {
    request_id: String,
    cancel: watch::Sender<bool>,
}

#[derive(Default)]
struct TtsRequests {
    active: Option<ActiveTtsRequest>,
    cancelled: VecDeque<String>,
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
            requests: Arc::new(Mutex::new(TtsRequests::default())),
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
            requests: Arc::new(Mutex::new(TtsRequests::default())),
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
    async fn build_provider(config: &ProviderConfig) -> Option<Arc<dyn TtsProvider>> {
        match config.provider_type.as_str() {
            "openai" => {
                OpenAITtsProvider::from_config(config).map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
            }
            "openai_realtime" => OpenAIRealtimeTtsProvider::from_config(config)
                .map(|p| Arc::new(p) as Arc<dyn TtsProvider>),
            "edge_tts" => EdgeTtsProvider::from_config(config)
                .await
                .map(|p| Arc::new(p) as Arc<dyn TtsProvider>),
            "browser" => {
                BrowserTTSProvider::from_config(config).map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
            }
            "local_vits" => {
                LocalVITSProvider::from_config(config).map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
            }
            "gpt_sovits" => LocalGPTSoVITSProvider::from_config(config)
                .map(|p| Arc::new(p) as Arc<dyn TtsProvider>),
            "omnivoice" => {
                OmniVoiceProvider::from_config(config).map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
            }
            "azure" => {
                AzureTtsProvider::from_config(config).map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
            }
            "elevenlabs" => CloudTTSProvider::elevenlabs_style(config)
                .map(|p| Arc::new(p) as Arc<dyn TtsProvider>),
            other => {
                tracing::error!(target: "tts", "Unknown provider type: {}", other);
                None
            }
        }
    }

    /// Register a provider and its voices.
    pub async fn register_provider(&self, provider: Arc<dyn TtsProvider>) {
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
        request_id: String,
    ) -> Result<(), String> {
        let Some(cancel) = self.register_request(&request_id).await else {
            return Ok(());
        };

        let result = self
            .speak_request(
                app,
                text,
                provider_id,
                params.unwrap_or_default(),
                request_id,
                &cancel,
            )
            .await;

        let mut requests = self.requests.lock().await;
        if requests
            .active
            .as_ref()
            .is_some_and(|current| current.cancel.same_channel(&cancel))
        {
            requests.active = None;
        }
        result
    }

    pub async fn cancel_request(&self, request_id: Option<&str>) -> bool {
        let mut requests = self.requests.lock().await;
        if let Some(current) = requests
            .active
            .as_ref()
            .filter(|current| request_id.is_none_or(|id| id == current.request_id))
        {
            current.cancel.send_replace(true);
            return true;
        }

        let Some(request_id) = request_id else {
            return false;
        };
        if !requests.cancelled.iter().any(|id| id == request_id) {
            if requests.cancelled.len() >= MAX_CANCELLED_TTS_REQUESTS {
                requests.cancelled.pop_front();
            }
            requests.cancelled.push_back(request_id.to_string());
        }
        true
    }

    async fn register_request(&self, request_id: &str) -> Option<watch::Sender<bool>> {
        let mut requests = self.requests.lock().await;
        if let Some(index) = requests.cancelled.iter().position(|id| id == request_id) {
            requests.cancelled.remove(index);
            return None;
        }

        let cancel = watch::Sender::new(false);
        if let Some(previous) = requests.active.replace(ActiveTtsRequest {
            request_id: request_id.to_owned(),
            cancel: cancel.clone(),
        }) {
            previous.cancel.send_replace(true);
        }
        Some(cancel)
    }

    async fn publish_playback_start(
        &self,
        cancel: &watch::Sender<bool>,
        window: &str,
        runtime: Option<&ChatRuntimeState>,
        emit_start: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<bool, String> {
        let requests = self.requests.lock().await;
        let Some(current) = requests
            .active
            .as_ref()
            .filter(|current| current.cancel.same_channel(cancel) && !*cancel.borrow())
        else {
            return Ok(false);
        };
        let request_id = &current.request_id;
        // A replacement or cancellation must wait until ownership and tts:start are published.
        if let Some(runtime) = runtime {
            runtime.begin_playback(request_id, window);
        }
        if let Err(error) = emit_start(request_id) {
            if let Some(runtime) = runtime {
                runtime.finish_playback(request_id, window, None);
            }
            return Err(error);
        }
        Ok(true)
    }

    async fn speak_request(
        &self,
        app: AppHandle,
        text: String,
        preferred_provider_id: Option<String>,
        params: TtsParams,
        request_id: String,
        cancellation: &watch::Sender<bool>,
    ) -> Result<(), String> {
        let mut cancel = cancellation.subscribe();
        let playback_window = if app.get_webview_window("main").is_some() { "main" } else { "pet" };
        let hook_runtime = app.try_state::<HookRuntime>();
        let router = TtsRouter::new(self.providers.clone(), self.default_provider.clone());
        let route = router
            .select_provider(
                preferred_provider_id.as_deref(),
                params.required_capabilities.as_ref(),
            )
            .await
            .map_err(|error| error.to_string())?;
        let provider = self
            .providers
            .read()
            .await
            .get(&route.provider_id)
            .cloned()
            .ok_or_else(|| format!("Provider {} not found", route.provider_id))?;

        if *cancel.borrow() {
            return Ok(());
        }

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

        let runtime = app.try_state::<Arc<ChatRuntimeState>>();
        if !self
            .publish_playback_start(
                cancellation,
                playback_window,
                runtime.as_ref().map(|state| state.inner().as_ref()),
                |request_id| {
                    app.emit_to(
                        playback_window,
                        "tts:start",
                        TtsStartEvent {
                            request_id: request_id.to_owned(),
                            text: text.clone(),
                        },
                    )
                    .map_err(|error| error.to_string())
                },
            )
            .await?
        {
            return Ok(());
        }

        let sentences: Vec<(usize, String)> = split_sentences(&text)
            .into_iter()
            .map(str::to_string)
            .filter(|sentence| !sentence.trim().is_empty())
            .enumerate()
            .map(|(index, sentence)| (index + 1, sentence))
            .collect();
        let concurrency = provider.sentence_concurrency().max(1);
        let provider_id = route.provider_id.clone();
        let provider_for_stream = provider.clone();
        let provider_output_format = provider.output_format();
        let params_for_stream = params.clone();
        let request_for_stream = request_id.clone();
        let cancel_template = cancel.clone();
        let service = self.clone();

        let mut sentences = futures::stream::iter(sentences)
            .map(move |(sentence_id, sentence)| {
                let params = params_for_stream.clone();
                let provider = provider_for_stream.clone();
                let provider_id = provider_id.clone();
                let request_id = request_for_stream.clone();
                let mut sentence_cancel = cancel_template.clone();
                let service = service.clone();
                async move {
                    if *sentence_cancel.borrow() {
                        return Err(TtsError::Cancelled);
                    }

                    let cache_salt = cache_variant_hash(provider.as_ref(), &params);
                    let voice_id = params.voice.clone().unwrap_or_default();
                    let cache_key = CacheKey::new(
                        &sentence,
                        &voice_id,
                        &provider_id,
                        params.speed,
                        params.pitch,
                        cache_salt.as_deref(),
                    );

                    if service.cache_enabled {
                        let cached = service.cache.write().await.get(&cache_key);
                        if let Some(audio) = cached {
                            let chunk = TtsAudioChunk {
                                data: audio,
                                format: provider.output_format(),
                                response_id: None,
                            };
                            let stream = futures::stream::once(async move { Ok(chunk) });
                            return Ok((
                                sentence_id,
                                sentence,
                                Some(Box::pin(stream) as TtsAudioStream),
                                None,
                            ));
                        }
                    }

                    let provider_cancel = sentence_cancel.clone();
                    let stream_result = tokio::select! {
                        _ = wait_until_cancelled(&mut sentence_cancel) => Err(TtsError::Cancelled),
                        result = provider.synthesize_stream_for_request(
                            &sentence,
                            params.clone(),
                            &request_id,
                            provider_cancel,
                        ) => result,
                    };
                    match stream_result {
                        Ok(stream) => Ok((sentence_id, sentence, Some(stream), Some(cache_key))),
                        Err(TtsError::BrowserDelegate) => Ok((sentence_id, sentence, None, None)),
                        Err(error) => Err(error),
                    }
                }
            })
            .buffered(concurrency);

        let mut sequence = 0_u64;
        let mut synthesis_error: Option<String> = None;
        let mut cancelled = false;
        'sentences: while let Some(result) = sentences.next().await {
            if *cancel.borrow() {
                cancelled = true;
                provider.cancel_request(&request_id).await;
                break;
            }
            match result {
                Ok((sentence_id, _sentence, Some(mut audio_stream), cache_key)) => {
                    let mut full_audio = Vec::new();
                    loop {
                        tokio::select! {
                            _ = wait_until_cancelled(&mut cancel) => {
                                cancelled = true;
                                provider.cancel_request(&request_id).await;
                                break;
                            }
                            chunk = audio_stream.next() => {
                                match chunk {
                                    Some(Ok(chunk)) if chunk.data.is_empty() => continue,
                                    Some(Ok(chunk)) => {
                                        full_audio.extend_from_slice(&chunk.data);
                                        sequence += 1;
                                        if let Err(error) = app.emit_to(playback_window, "tts:audio", TtsAudioEvent {
                                            request_id: request_id.clone(),
                                            sentence_id,
                                            sequence,
                                            response_id: chunk.response_id,
                                            format: chunk.format.or_else(|| provider_output_format.clone()),
                                            data: chunk.data,
                                        }) {
                                            synthesis_error = Some(format!("failed to deliver TTS audio: {error}"));
                                            provider.cancel_request(&request_id).await;
                                            break;
                                        }
                                    }
                                    Some(Err(TtsError::Cancelled)) => {
                                        cancelled = true;
                                        provider.cancel_request(&request_id).await;
                                        break;
                                    }
                                    Some(Err(error)) => {
                                        tracing::warn!(target: "tts", "TTS stream failed: request_id={} sentence_id={} error={}", request_id, sentence_id, error);
                                        synthesis_error = Some(format!("TTS synthesis failed: {error}"));
                                        provider.cancel_request(&request_id).await;
                                        break;
                                    }
                                    None => break,
                                }
                            }
                        }
                    }

                    if cancelled || synthesis_error.is_some() {
                        break 'sentences;
                    }
                    if !full_audio.is_empty() {
                        if let Some(key) = cache_key {
                            if self.cache_enabled {
                                self.cache.write().await.put(key, full_audio);
                            }
                        }
                    }
                }
                Ok((sentence_id, sentence, None, None)) => {
                    app.emit_to(playback_window,
                        "tts:browser-delegate",
                        TtsBrowserDelegateEvent {
                            request_id: request_id.clone(),
                            sentence_id,
                            text: sentence,
                            voice: params.voice.clone(),
                            speed: params.speed,
                            pitch: params.pitch,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                }
                Ok((_, _, None, Some(_))) => {
                    synthesis_error = Some("TTS provider returned no audio stream".to_string());
                    break;
                }
                Err(TtsError::Cancelled) => {
                    cancelled = true;
                    provider.cancel_request(&request_id).await;
                    break;
                }
                Err(error) => {
                    tracing::warn!(target: "tts", "TTS synthesis failed: request_id={} error={}", request_id, error);
                    synthesis_error = Some(format!("TTS synthesis failed: {error}"));
                    provider.cancel_request(&request_id).await;
                    break;
                }
            }
        }

        if *cancel.borrow() {
            cancelled = true;
        }
        let finish_status = if cancelled {
            TtsFinishStatus::Cancelled
        } else if synthesis_error.is_some() {
            TtsFinishStatus::Error
        } else {
            TtsFinishStatus::Completed
        };

        if let Some(error) = synthesis_error.as_ref() {
            let _ = app.emit_to(playback_window,
                "tts:error",
                TtsErrorEvent {
                    request_id: request_id.clone(),
                    error: error.clone(),
                },
            );
        }
        app.emit_to(playback_window,
            "tts:end",
            TtsEndEvent {
                request_id: request_id.clone(),
                text: text.clone(),
                status: finish_status,
            },
        )
        .map_err(|error| error.to_string())?;

        if let Some(error) = synthesis_error {
            return Err(error);
        }
        if !cancelled {
            if let Some(hooks) = hook_runtime.as_ref() {
                hooks
                    .emit_best_effort(
                        &HookEvent::AfterTtsPlay,
                        &HookPayload::Tts(TtsHookPayload {
                            text,
                            provider_id: Some(route.provider_id),
                        }),
                    )
                    .await;
            }
        }
        Ok(())
    }

    // ── Query methods ──────────────────────────────────

    /// List all registered provider IDs with their status.
    pub async fn list_providers(&self) -> Vec<ProviderStatus> {
        let providers = self.providers.read().await.clone();
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
        let provider = self.providers.read().await.get(id).cloned()?;
        Some(ProviderStatus {
            id: id.to_string(),
            available: provider.is_available().await,
            capabilities: provider.capabilities(),
        })
    }

    /// Hot-reload: rebuild providers first, then atomically swap runtime state.
    pub async fn reload_from_config(&self, config: &TtsSystemConfig) -> Result<(), TtsError> {
        self.cancel_request(None).await;
        let mut new_providers: HashMap<String, Arc<dyn TtsProvider>> = HashMap::new();
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

        let provider = self
            .providers
            .read()
            .await
            .get(&route.provider_id)
            .cloned()
            .ok_or_else(|| format!("Provider {} not found", route.provider_id))?;
        let output_format = provider.output_format();

        let mut stream = provider
            .synthesize_stream(text, params)
            .await
            .map_err(|e| e.to_string())?;

        let mut audio = Vec::new();
        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(chunk) => audio.extend_from_slice(&chunk.data),
                Err(e) => return Err(format!("TTS stream error: {}", e)),
            }
        }
        match output_format {
            Some(AudioFormat::PcmS16Le {
                sample_rate,
                channels,
            }) => wrap_pcm_s16le_as_wav(&audio, sample_rate, channels),
            _ => Ok(audio),
        }
    }

    /// Clear the synthesis cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

fn cache_variant_hash(provider: &dyn TtsProvider, params: &TtsParams) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(salt) = provider.cache_key_salt() {
        if !salt.is_empty() {
            parts.push(format!("provider:{salt}"));
        }
    }

    if let Some(salt) = params.extra_cache_key_salt() {
        if !salt.is_empty() {
            parts.push(format!("params:{salt}"));
        }
    }

    if parts.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(parts.join("\n").as_bytes());
    Some(format!("{:x}", hasher.finalize()))
}

/// The non-streaming helper is used by file-oriented integrations. Keep their
/// audio contract recognizable without changing desktop PCM streaming.
fn wrap_pcm_s16le_as_wav(pcm: &[u8], sample_rate: u32, channels: u8) -> Result<Vec<u8>, String> {
    if sample_rate == 0 || channels == 0 || !pcm.len().is_multiple_of(usize::from(channels) * 2) {
        return Err("Realtime PCM audio has an invalid sample format".to_string());
    }
    let data_len =
        u32::try_from(pcm.len()).map_err(|_| "Realtime audio is too large".to_string())?;
    let riff_len = data_len
        .checked_add(36)
        .ok_or_else(|| "Realtime audio is too large".to_string())?;
    let block_align = u16::from(channels) * 2;
    let byte_rate = sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| "Realtime audio sample rate is too large".to_string())?;

    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&u16::from(channels).to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
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

async fn wait_until_cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow() {
        if cancel.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
fn record_tts_error(current: &mut Option<String>, error: String) {
    if current.is_none() {
        *current = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        record_tts_error, wrap_pcm_s16le_as_wav, ChatRuntimeState, TtsService,
        MAX_CANCELLED_TTS_REQUESTS,
    };

    #[test]
    fn records_the_first_synthesis_error_for_the_command_result() {
        let mut error = None;
        record_tts_error(&mut error, "provider unavailable".to_string());
        record_tts_error(&mut error, "second sentence failed".to_string());

        assert_eq!(error.as_deref(), Some("provider unavailable"));
    }

    #[tokio::test]
    async fn cancellation_before_request_registration_is_remembered_once() {
        let service = TtsService::new();
        let current = service.register_request("current").await.unwrap();
        assert!(service.cancel_request(Some("request-before-start")).await);
        assert!(service
            .register_request("request-before-start")
            .await
            .is_none());
        assert!(!*current.borrow());
        assert!(service
            .register_request("request-before-start")
            .await
            .is_some());
        assert!(*current.borrow());
    }

    #[tokio::test]
    async fn pending_cancellations_have_a_fixed_capacity() {
        let service = TtsService::new();
        for index in 0..=MAX_CANCELLED_TTS_REQUESTS {
            assert!(
                service
                    .cancel_request(Some(&format!("pending-{index}")))
                    .await
            );
        }
        let requests = service.requests.lock().await;
        let pending = &requests.cancelled;
        assert_eq!(pending.len(), MAX_CANCELLED_TTS_REQUESTS);
        assert_eq!(pending.front().map(String::as_str), Some("pending-1"));
    }

    #[tokio::test]
    async fn a_replaced_request_cannot_publish_start_after_the_replacement_starts() {
        let service = TtsService::new();
        let runtime = ChatRuntimeState::default();
        for (client_id, turn_id) in [("old", "old-turn"), ("new", "new-turn")] {
            let request = serde_json::from_value(serde_json::json!({
                "message": "reply", "client_request_id": client_id,
            }))
            .unwrap();
            runtime.begin(&request).unwrap();
            runtime
                .event(
                    "chat-turn-start",
                    &serde_json::json!({"client_request_id": client_id, "turn_id": turn_id}),
                )
                .unwrap();
        }

        let (old_paused, wait_for_old) = tokio::sync::oneshot::channel();
        let (new_started, wait_for_new) = tokio::sync::oneshot::channel();
        let (mut old_start, mut new_start) = (None, None);
        tokio::join!(
            async {
                let old = service.register_request("chat_old-turn").await.unwrap();
                assert!(!*old.borrow());
                old_paused.send(()).unwrap();
                wait_for_new.await.unwrap();
                assert!(!service
                    .publish_playback_start(&old, "main", Some(&runtime), |id| {
                        old_start = Some(id.to_owned());
                        Ok(())
                    })
                    .await
                    .unwrap());
            },
            async {
                wait_for_old.await.unwrap();
                let new = service.register_request("chat_new-turn").await.unwrap();
                assert!(service
                    .publish_playback_start(&new, "main", Some(&runtime), |id| {
                        new_start = Some(id.to_owned());
                        Ok(())
                    })
                    .await
                    .unwrap());
                new_started.send(()).unwrap();
            }
        );
        assert_eq!(new_start.as_deref(), Some("chat_new-turn"));
        assert!(old_start.is_none());
        assert!(runtime
            .finish_playback("chat_old-turn", "main", Some("late error".into()))
            .is_none());
        let owner = runtime
            .finish_playback("chat_new-turn", "main", Some("output failed".into()))
            .unwrap();
        assert_eq!(owner.client_request_id, "new");
    }

    #[tokio::test]
    async fn cancellation_after_registration_prevents_start() {
        let service = TtsService::new();
        let request = service.register_request("chat_cancelled").await.unwrap();
        assert!(service.cancel_request(Some("chat_cancelled")).await);
        assert!(!service
            .publish_playback_start(&request, "main", None, |_| {
                panic!("cancelled request must not publish start")
            })
            .await
            .unwrap());
    }

    #[test]
    fn non_streaming_pcm_adapter_returns_a_wav_for_file_consumers() {
        let wav = wrap_pcm_s16le_as_wav(&[1, 0, 2, 0], 24_000, 1).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 4);
        assert_eq!(&wav[44..], &[1, 0, 2, 0]);
    }

    #[test]
    fn non_streaming_pcm_adapter_rejects_incomplete_samples() {
        assert!(wrap_pcm_s16le_as_wav(&[1], 24_000, 1).is_err());
    }
}
