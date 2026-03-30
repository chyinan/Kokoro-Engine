use crate::error::KokoroError;
use crate::stt::config::save_config;
use crate::stt::{
    AudioChunk, AudioSource, NativeMicState, NativeWakeWordState, SenseVoiceLocalModelStatus,
    SttConfig, SttService,
};
use std::sync::Arc;
use tauri::State;
use tauri::{command, AppHandle};

/// Transcribe audio bytes to text using the active STT provider.
#[command]
pub async fn transcribe_audio(
    state: State<'_, SttService>,
    audio_bytes: Vec<u8>,
    format: String,
) -> Result<String, KokoroError> {
    let source = AudioSource::Encoded { data: audio_bytes, format };
    let result = state.transcribe(&source, None).await.map_err(|e| KokoroError::Stt(e.to_string()))?;
    Ok(result.text)
}

/// Return the current STT config from disk.
/// Automatically merges any missing default providers so the UI always shows all options.
#[command]
pub async fn get_stt_config() -> Result<SttConfig, KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");
    let mut config = crate::stt::load_config(&config_path);

    // Merge missing default providers so new providers appear in the UI
    // without requiring users to manually edit stt_config.json.
    let defaults = crate::stt::config::default_providers_pub();
    let mut changed = false;
    for default in defaults {
        if !config.providers.iter().any(|p| p.id == default.id) {
            config.providers.push(default);
            changed = true;
        }
    }

    // Write back if we added new providers, so active_provider survives next load
    if changed {
        let _ = crate::stt::config::save_config(&config_path, &config);
    }

    Ok(config)
}

/// Save STT config to disk and hot-reload providers.
#[command]
pub async fn save_stt_config(
    state: State<'_, SttService>,
    config: SttConfig,
) -> Result<(), KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");
    save_config(&config_path, &config)?;
    state.reload_from_config(&config).await;
    Ok(())
}

/// Transcribe a short raw PCM audio clip (float32, 16kHz mono) for wake word detection.
/// Does NOT use the streaming buffer — fire-and-forget one-shot transcription.
#[command]
pub async fn transcribe_wake_word_audio(
    state: State<'_, SttService>,
    samples: Vec<f32>,
) -> Result<String, KokoroError> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    let chunk = AudioChunk { samples: Arc::new(samples), sample_rate: 16000 };
    let result = state.transcribe(&AudioSource::Chunk(chunk), None).await.map_err(|e| KokoroError::Stt(e.to_string()))?;
    Ok(result.text)}

#[command]
pub async fn start_native_mic(
    app: AppHandle,
    mic_state: State<'_, NativeMicState>,
    auto_stop_on_silence: Option<bool>,
) -> Result<(), KokoroError> {
    crate::stt::mic::start_native_mic_with_options(&app, mic_state.inner(), auto_stop_on_silence.unwrap_or(false))
        .map_err(KokoroError::Stt)
}

#[command]
pub async fn stop_native_mic(
    app: AppHandle,
    mic_state: State<'_, NativeMicState>,
) -> Result<(), KokoroError> {
    crate::stt::mic::stop_native_mic(&app, mic_state.inner()).map_err(KokoroError::Stt)
}

#[command]
pub async fn start_native_wake_word(
    app: AppHandle,
    wake_word_state: State<'_, NativeWakeWordState>,
    wake_word: String,
    trigger_on_speech: Option<bool>,
) -> Result<(), KokoroError> {
    crate::stt::wake_word::start_native_wake_word(&app, wake_word_state.inner(), wake_word, trigger_on_speech.unwrap_or(false))
        .map_err(KokoroError::Stt)
}

#[command]
pub async fn stop_native_wake_word(
    app: AppHandle,
    wake_word_state: State<'_, NativeWakeWordState>,
) -> Result<(), KokoroError> {
    crate::stt::wake_word::stop_native_wake_word(&app, wake_word_state.inner()).map_err(KokoroError::Stt)
}

#[command]
pub async fn get_sensevoice_local_status() -> Result<SenseVoiceLocalModelStatus, KokoroError> {
    Ok(crate::stt::sensevoice_local::recommended_model_status())
}

#[command]
pub async fn download_sensevoice_local_model(
    app: tauri::AppHandle,
) -> Result<SenseVoiceLocalModelStatus, KokoroError> {
    use tauri::Emitter;
    crate::stt::sensevoice_local::download_recommended_model(|progress| {
        app.emit("stt:sensevoice-local-progress", &progress)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(KokoroError::Stt)
}
