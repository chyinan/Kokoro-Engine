//! Telegram Bot IPC commands — frontend ↔ backend bridge.

use crate::bot::{BotChatServices, BotConfig, BotRuntimeService};
use crate::error::KokoroError;
use crate::ipc::wire::Response;
use crate::telegram::{TelegramConfig, TelegramService};
use serde::Serialize;
use tauri::State;

#[derive(Serialize, specta::Type)]
pub struct TelegramStatus {
    pub running: bool,
    pub enabled: bool,
    pub has_token: bool,
}

#[derive(Serialize, specta::Type)]
pub struct TelegramConfigSnapshot {
    pub revision: u64,
    pub config: TelegramConfig,
}

#[tauri::command]
#[specta::specta]
pub async fn get_telegram_config(
    runtime: State<'_, BotRuntimeService>,
) -> Result<Response<TelegramConfigSnapshot>, KokoroError> {
    let (revision, config) = runtime.current_telegram_config().await;
    Ok(Response(TelegramConfigSnapshot { revision, config }))
}

#[tauri::command]
#[specta::specta]
pub async fn save_telegram_config(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
    expected_revision: u64,
    config: TelegramConfig,
) -> Result<Response<BotConfig>, KokoroError> {
    let _lease = state
        .admit_control("save Telegram configuration")
        .map_err(KokoroError::ExternalService)?;
    runtime
        .save_telegram_config(expected_revision, config)
        .await
        .map(Response)
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
