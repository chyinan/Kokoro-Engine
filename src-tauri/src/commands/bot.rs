//! Tauri adapters for external bot configuration and lifecycle.

use crate::bot::{self, BotChatServices, BotConfig, BotRuntimeService, BotStatus};
use crate::error::KokoroError;
use crate::ipc::wire::{emit, Response};
use crate::telegram::TelegramService;
use tauri::State;
use tauri_specta::Event;

#[derive(Clone, serde::Serialize, specta::Type, Event)]
#[tauri_specta(event_name = "qq-authorization-approved")]
pub struct QqAuthorizationApproved {
    pub user_openid: String,
    pub group_openid: Option<String>,
    pub approved_user_openid: Option<String>,
    pub approved_group_openid: Option<String>,
    pub revision: u64,
}

#[tauri::command]
#[specta::specta]
pub async fn get_bot_config(
    runtime: State<'_, BotRuntimeService>,
) -> Result<Response<BotConfig>, KokoroError> {
    Ok(Response(runtime.current_config().await))
}

#[tauri::command]
#[specta::specta]
pub async fn save_bot_config(
    runtime: State<'_, BotRuntimeService>,
    config: BotConfig,
) -> Result<Response<BotConfig>, KokoroError> {
    runtime.save_config(config).await.map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn respond_qq_authorization(
    runtime: State<'_, BotRuntimeService>,
    app: tauri::AppHandle,
    request_id: String,
    user_openid: String,
    group_openid: Option<String>,
    approved: bool,
) -> Result<Response<QqAuthorizationApproved>, KokoroError> {
    let (revision, approved_user_openid, approved_group_openid) = runtime
        .respond_to_qq_authorization(&request_id, &user_openid, group_openid.as_deref(), approved)
        .await?;
    let response = QqAuthorizationApproved {
        user_openid: user_openid.trim().to_owned(),
        group_openid: group_openid.map(|openid| openid.trim().to_owned()),
        approved_user_openid,
        approved_group_openid,
        revision,
    };
    if approved {
        if let Err(error) = emit(&app, response.clone()) {
            tracing::warn!(target: "bot::qq", "failed to synchronize approved QQ OpenID to the settings UI: {}", error);
        }
    }
    Ok(Response(response))
}

#[tauri::command]
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
pub async fn get_bot_status(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
) -> Result<BotStatus, KokoroError> {
    Ok(bot::get_bot_status(&state, &runtime).await)
}
