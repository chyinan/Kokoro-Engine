use crate::error::KokoroError;
use crate::stt::{
    AudioChunk, AudioSource, NativeMicState, NativeWakeWordState, SenseVoiceLocalModelStatus,
    SttConfig, SttService,
};
use std::sync::Arc;
use tauri::State;
use tauri::{command, AppHandle, Manager};

/// Transcribe audio bytes to text using the active STT provider.
#[command]
pub async fn transcribe_audio(
    state: State<'_, SttService>,
    audio_bytes: Vec<u8>,
    format: String,
) -> Result<String, KokoroError> {
    let source = AudioSource::Encoded {
        data: audio_bytes,
        format,
    };
    let result = state
        .transcribe(&source, None)
        .await
        .map_err(|e| KokoroError::Stt(e.to_string()))?;
    Ok(result.text)
}

/// Return the current STT config from disk.
/// Automatically merges any missing default providers so the UI always shows all options.
#[command]
pub async fn get_stt_config(state: State<'_, SttService>) -> Result<SttConfig, KokoroError> {
    let app_data = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");
    Ok(state.load_config_with_defaults(&config_path).await)
}

/// Save STT config to disk and hot-reload providers.
#[command]
pub async fn save_stt_config(
    state: State<'_, SttService>,
    config: SttConfig,
) -> Result<(), KokoroError> {
    let app_data = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");
    state.save_and_reload(&config_path, config).await
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
    let chunk = AudioChunk {
        samples: Arc::new(samples),
        sample_rate: 16000,
    };
    let result = state
        .transcribe(&AudioSource::Chunk(chunk), None)
        .await
        .map_err(|e| KokoroError::Stt(e.to_string()))?;
    Ok(result.text)
}

#[command]
pub async fn start_native_mic(
    app: AppHandle,
    auto_stop_on_silence: Option<bool>,
    only_when_chat_idle: Option<bool>,
) -> Result<bool, KokoroError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::stt::mic::start_native_mic_with_options(
            &app,
            app.state::<NativeMicState>().inner(),
            auto_stop_on_silence.unwrap_or(false),
            only_when_chat_idle.unwrap_or(false),
        )
    })
    .await
    .map_err(|error| KokoroError::Stt(error.to_string()))?
    .map_err(KokoroError::Stt)
}

#[command]
pub async fn stop_native_mic(app: AppHandle) -> Result<(), KokoroError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::stt::mic::stop_native_mic(&app, app.state::<NativeMicState>().inner())
    })
    .await
    .map_err(|error| KokoroError::Stt(error.to_string()))?
    .map_err(KokoroError::Stt)
}

#[command]
pub async fn start_native_wake_word(
    app: AppHandle,
    wake_word_state: State<'_, NativeWakeWordState>,
    wake_word: String,
    trigger_on_speech: Option<bool>,
    owner: Option<u64>,
) -> Result<(), KokoroError> {
    crate::stt::wake_word::start_native_wake_word(
        &app,
        wake_word_state.inner(),
        wake_word,
        trigger_on_speech.unwrap_or(false),
        owner,
    )
    .map_err(KokoroError::Stt)
}

#[command]
pub async fn stop_native_wake_word(
    app: AppHandle,
    wake_word_state: State<'_, NativeWakeWordState>,
    owner: Option<u64>,
) -> Result<(), KokoroError> {
    crate::stt::wake_word::stop_native_wake_word(&app, wake_word_state.inner(), owner)
        .map_err(KokoroError::Stt)
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
    crate::stt::sensevoice_local::download_recommended_model(move |progress| {
        app.emit("stt:sensevoice-local-progress", &progress)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(KokoroError::Stt)
}
