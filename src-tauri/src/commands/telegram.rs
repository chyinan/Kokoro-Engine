//! Telegram Bot IPC commands — frontend ↔ backend bridge.

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
) -> Result<crate::telegram::TelegramConfig, String> {
    Ok(state.get_config().await)
}

#[tauri::command]
pub async fn save_telegram_config(
    state: State<'_, TelegramService>,
    config: crate::telegram::TelegramConfig,
) -> Result<(), String> {
    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("telegram_config.json");
    crate::telegram::save_config(&config_path, &config)?;

    // Update in-memory config
    state.update_config(config).await;
    Ok(())
}

#[tauri::command]
pub async fn start_telegram_bot(
    state: State<'_, TelegramService>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    state.start(app).await
}

#[tauri::command]
pub async fn stop_telegram_bot(
    state: State<'_, TelegramService>,
) -> Result<(), String> {
    state.stop().await
}

#[tauri::command]
pub async fn get_telegram_status(
    state: State<'_, TelegramService>,
) -> Result<TelegramStatus, String> {
    let config = state.get_config().await;
    Ok(TelegramStatus {
        running: state.is_running().await,
        enabled: config.enabled,
        has_token: config.resolve_bot_token().is_some(),
    })
}
