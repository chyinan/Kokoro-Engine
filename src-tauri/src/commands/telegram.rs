//! Telegram Bot IPC commands — frontend ↔ backend bridge.

use crate::bot::{self, BotChatServices};
use crate::error::KokoroError;
use crate::ipc::wire::Response;
use crate::telegram::TelegramService;
use serde::Serialize;
use tauri::State;

#[derive(Serialize, specta::Type)]
pub struct TelegramStatus {
    pub running: bool,
    pub enabled: bool,
    pub has_token: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn get_telegram_config(
    state: State<'_, TelegramService>,
) -> Result<Response<crate::telegram::TelegramConfig>, KokoroError> {
    Ok(Response(state.get_config().await))
}

#[tauri::command]
#[specta::specta]
pub async fn save_telegram_config(
    state: State<'_, TelegramService>,
    config: crate::telegram::TelegramConfig,
) -> Result<(), KokoroError> {
    bot::save_telegram_config(&state, config).await
}

#[tauri::command]
#[specta::specta]
pub async fn start_telegram_bot(
    state: State<'_, TelegramService>,
    services: State<'_, BotChatServices>,
) -> Result<(), KokoroError> {
    state
        .start(services.inner().clone())
        .await
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
#[specta::specta]
pub async fn stop_telegram_bot(state: State<'_, TelegramService>) -> Result<(), KokoroError> {
    state.stop().await.map_err(KokoroError::ExternalService)
}

#[tauri::command]
#[specta::specta]
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
