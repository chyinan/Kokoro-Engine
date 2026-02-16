use crate::stt::config::save_config;
use crate::stt::{SttConfig, SttService};
use tauri::command;
use tauri::State;

/// Transcribe audio bytes to text using the active STT provider.
#[command]
pub async fn transcribe_audio(
    state: State<'_, SttService>,
    audio_bytes: Vec<u8>,
    format: String,
) -> Result<String, String> {
    state
        .transcribe(&audio_bytes, &format, None)
        .await
        .map_err(|e| e.to_string())
}

/// Return the current STT config from disk.
#[command]
pub async fn get_stt_config() -> Result<SttConfig, String> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");
    Ok(crate::stt::load_config(&config_path))
}

/// Save STT config to disk and hot-reload providers.
#[command]
pub async fn save_stt_config(
    state: State<'_, SttService>,
    config: SttConfig,
) -> Result<(), String> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("stt_config.json");

    // Write to disk
    save_config(&config_path, &config)?;

    // Hot-reload providers
    state.reload_from_config(&config).await;

    Ok(())
}
