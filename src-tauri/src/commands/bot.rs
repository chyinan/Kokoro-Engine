//! Tauri adapters for external bot configuration and lifecycle.

use crate::bot::{self, BotChatServices, BotConfig, BotRuntimeService, BotStatus};
use crate::error::KokoroError;
use crate::telegram::TelegramService;
use serde_json::{json, Value};
use tauri::{Emitter, State};

#[tauri::command]
pub async fn get_bot_config() -> Result<BotConfig, KokoroError> {
    Ok(bot::load_bot_config())
}

#[tauri::command]
pub async fn save_bot_config(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
    config: BotConfig,
) -> Result<BotConfig, KokoroError> {
    bot::save_bot_config(&state, &runtime, config).await
}

#[tauri::command]
pub async fn respond_qq_authorization(
    runtime: State<'_, BotRuntimeService>,
    app: tauri::AppHandle,
    request_id: String,
    user_openid: String,
    group_openid: Option<String>,
    approved: bool,
) -> Result<Value, KokoroError> {
    let (revision, approved_user_openid, approved_group_openid) = runtime
        .respond_to_qq_authorization(&request_id, &user_openid, group_openid.as_deref(), approved)
        .await
        .map_err(KokoroError::ExternalService)?;
    let response = json!({
        "user_openid": user_openid.trim(),
        "group_openid": group_openid.as_deref().map(str::trim),
        "approved_user_openid": approved_user_openid,
        "approved_group_openid": approved_group_openid,
        "revision": revision
    });
    if approved {
        if let Err(error) = app.emit("qq-authorization-approved", response.clone()) {
            tracing::warn!(target: "bot::qq", "failed to synchronize approved QQ OpenID to the settings UI: {}", error);
        }
    }
    Ok(response)
}

#[tauri::command]
pub async fn start_bot_platform(
    runtime: State<'_, BotRuntimeService>,
    services: State<'_, BotChatServices>,
    platform: String,
) -> Result<(), KokoroError> {
    runtime
        .start_platform(&platform, services.inner().clone())
        .await
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn stop_bot_platform(
    runtime: State<'_, BotRuntimeService>,
    platform: String,
) -> Result<(), KokoroError> {
    runtime
        .stop_platform(&platform)
        .await
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn get_bot_status(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
) -> Result<BotStatus, KokoroError> {
    Ok(bot::get_bot_status(&state, &runtime).await)
}
