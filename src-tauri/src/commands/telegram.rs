//! Telegram Bot IPC commands — frontend ↔ backend bridge.

use crate::error::KokoroError;
use crate::telegram::TelegramService;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct TelegramStatus {
    pub running: bool,
    pub enabled: bool,
    pub has_token: bool,
}

#[tauri::command]
pub async fn get_telegram_config(
    state: State<'_, TelegramService>,
) -> Result<crate::telegram::TelegramConfig, KokoroError> {
    Ok(state.get_config().await)
}

#[tauri::command]
pub async fn save_telegram_config(
    state: State<'_, TelegramService>,
    config: crate::telegram::TelegramConfig,
) -> Result<(), KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("telegram_config.json");
    crate::telegram::save_config(&config_path, &config).map_err(KokoroError::Config)?;
    state.update_config(config).await;
    Ok(())
}

#[tauri::command]
pub async fn start_telegram_bot(
    state: State<'_, TelegramService>,
    app: tauri::AppHandle,
) -> Result<(), KokoroError> {
    state.start(app).await.map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn stop_telegram_bot(
    state: State<'_, TelegramService>,
) -> Result<(), KokoroError> {
    state.stop().await.map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn get_telegram_status(
    state: State<'_, TelegramService>,
) -> Result<TelegramStatus, KokoroError> {
    let config = state.get_config().await;
    Ok(TelegramStatus {
        running: state.is_running().await,
        enabled: config.enabled,
        has_token: config.resolve_bot_token().is_some(),
    })
}
