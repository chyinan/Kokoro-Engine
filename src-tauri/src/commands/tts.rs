use crate::chat::runtime::ChatRuntimeState;
use crate::error::KokoroError;
use crate::hooks::HookRuntime;
use crate::ipc::wire::Response;
use crate::lifecycle::{AppLifecycle, TaskStage};
use crate::tts::config::{ProviderConfig, TtsSystemConfig};
use crate::tts::desktop::TauriTtsPlayback;
use crate::tts::local_gpt_sovits::GptSovitsModels;
use crate::tts::{ProviderStatus, TtsParams, TtsService, VoiceProfile};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{command, AppHandle, Manager, State};

#[derive(serde::Deserialize, specta::Type)]
pub struct TtsConfig {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub pitch: Option<f32>,
    #[serde(default)]
    pub emotion: Option<String>,
}

#[command]
#[specta::specta]
pub async fn synthesize(
    app: AppHandle,
    state: State<'_, TtsService>,
    text: String,
    config: TtsConfig,
) -> Result<(), KokoroError> {
    let params = TtsParams {
        voice: config.voice,
        speed: config.speed,
        pitch: config.pitch,
        emotion: config.emotion,
        required_capabilities: None,
        extra_params: None,
    };

    let runtime = app.try_state::<Arc<ChatRuntimeState>>();
    let playback = TauriTtsPlayback::new(&app, runtime.as_deref().cloned());
    let hooks = app.try_state::<HookRuntime>();
    let scope = app.state::<AppLifecycle>().scope(TaskStage::Work, "tts");
    let _work = scope
        .lease("desktop synthesis")
        .map_err(|error| KokoroError::Tts(error.to_string()))?;
    let stop = scope.token();
    let request_id = config
        .request_id
        .filter(|request_id| !request_id.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let speak = state.speak(
        &playback,
        hooks.as_deref(),
        text,
        config.provider_id,
        Some(params),
        request_id.clone(),
    );
    tokio::pin!(speak);
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => {
            state.cancel_request(Some(&request_id)).await;
            speak.await
        }
        result = &mut speak => result,
    };
    result.map_err(KokoroError::Tts)
}

#[command]
#[specta::specta]
pub async fn cancel_tts(
    state: State<'_, TtsService>,
    request_id: Option<String>,
) -> Result<(), KokoroError> {
    state.cancel_request(request_id.as_deref()).await;
    Ok(())
}

#[command]
#[specta::specta]
pub async fn list_tts_providers(
    state: State<'_, TtsService>,
) -> Result<Vec<ProviderStatus>, KokoroError> {
    Ok(state.list_providers().await)
}

#[command]
#[specta::specta]
pub async fn list_tts_voices(
    state: State<'_, TtsService>,
) -> Result<Vec<VoiceProfile>, KokoroError> {
    Ok(state.list_voices().await)
}

#[command]
#[specta::specta]
pub async fn list_openai_tts_models(config: ProviderConfig) -> Result<Vec<String>, KokoroError> {
    crate::tts::openai::list_openai_tts_models(&config)
        .await
        .map_err(|error| KokoroError::Tts(error.to_string()))
}

#[command]
#[specta::specta]
pub async fn get_tts_provider_status(
    state: State<'_, TtsService>,
    provider_id: String,
) -> Result<Option<ProviderStatus>, KokoroError> {
    Ok(state.get_provider_status(&provider_id).await)
}

#[command]
#[specta::specta]
pub async fn clear_tts_cache(state: State<'_, TtsService>) -> Result<(), KokoroError> {
    state.clear_cache().await;
    Ok(())
}

/// Return the current TTS config from disk.
#[command]
#[specta::specta]
pub async fn get_tts_config() -> Result<Response<TtsSystemConfig>, KokoroError> {
    Ok(Response(crate::tts::load_config(&tts_config_path())))
}

/// Save TTS config to disk and hot-reload providers.
#[command]
#[specta::specta]
pub async fn save_tts_config(
    state: State<'_, TtsService>,
    config: TtsSystemConfig,
) -> Result<(), KokoroError> {
    state
        .save_and_reload_config(&tts_config_path(), &config)
        .await
}

fn tts_config_path() -> PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("tts_config.json")
}

#[command]
#[specta::specta]
pub async fn list_gpt_sovits_models(install_path: String) -> Result<GptSovitsModels, KokoroError> {
    crate::tts::local_gpt_sovits::list_gpt_sovits_models(std::path::Path::new(&install_path))
}
