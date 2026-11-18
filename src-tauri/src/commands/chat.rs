use crate::ai::context::AIOrchestrator;
use crate::chat::application::{
    approve_tool_approval_inner, cancel_and_wait, recover_chat_draft_submission_inner,
    reject_tool_approval_inner, ChatDraftSubmissionStatus, ChatPolicy, ChatRequest, ChatService,
    PendingToolApprovalState, StreamChatResponse, TurnCancellationState,
};
use crate::chat::events::ChatEvents;
use crate::chat::settings::ContextSettings;
use crate::error::KokoroError;
use std::sync::Arc;
use tauri::{command, Manager, State};

#[command]
pub async fn approve_tool_approval(
    approval_request_id: String,
    approval_state: State<'_, Arc<PendingToolApprovalState>>,
) -> Result<(), KokoroError> {
    approve_tool_approval_inner(approval_state.inner().as_ref(), approval_request_id).await
}

#[command]
pub async fn reject_tool_approval(
    approval_request_id: String,
    reason: Option<String>,
    approval_state: State<'_, Arc<PendingToolApprovalState>>,
) -> Result<(), KokoroError> {
    reject_tool_approval_inner(approval_state.inner().as_ref(), approval_request_id, reason).await
}

#[tauri::command]
pub async fn cancel_chat_turn(
    turn_id: String,
    reason: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(chat) = app.try_state::<ChatService>() {
        return chat.cancel(turn_id, reason).await;
    }
    // Cancellation is still available when the AI database did not initialize.
    cancel_and_wait(
        turn_id,
        reason,
        &app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>(),
        app.state::<Arc<TurnCancellationState>>().inner().clone(),
        &app.state::<Arc<PendingToolApprovalState>>(),
    )
    .await
}

#[tauri::command]
pub fn is_chat_busy(state: State<'_, AIOrchestrator>) -> bool {
    state.is_chat_busy()
}

#[tauri::command]
pub async fn get_context_settings(
    state: State<'_, AIOrchestrator>,
) -> Result<ContextSettings, KokoroError> {
    Ok(ContextSettings::current(&state).await)
}

#[tauri::command]
pub async fn set_context_settings(
    state: State<'_, AIOrchestrator>,
    settings: ContextSettings,
) -> Result<(), KokoroError> {
    let app_data = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    settings
        .save(&state, &app_data.join("context_settings.json"))
        .await;

    Ok(())
}

#[tauri::command]
pub(crate) fn finish_chat_playback(
    window: tauri::Window,
    request_id: String,
    error: Option<String>,
    events: State<'_, ChatEvents>,
) {
    events.finish_playback(&request_id, window.label(), error);
}

#[tauri::command]
pub async fn get_chat_runtime_snapshot(
    state: State<'_, AIOrchestrator>,
    runtime: State<'_, Arc<crate::chat::runtime::ChatRuntimeState>>,
) -> Result<Vec<crate::chat::runtime::ChatTurnSnapshot>, KokoroError> {
    let (conversation, _) = state.conversation_state_snapshot().await;
    Ok(runtime.desktop_snapshots(conversation.as_deref()).1)
}

#[tauri::command]
pub async fn get_chat_view(
    state: State<'_, AIOrchestrator>,
    runtime: State<'_, Arc<crate::chat::runtime::ChatRuntimeState>>,
    known_version: Option<crate::chat::runtime::ChatViewVersion>,
) -> Result<Option<crate::chat::runtime::ChatViewSnapshot>, KokoroError> {
    crate::chat::runtime::get_chat_view(&state, &runtime, known_version).await
}

#[tauri::command]
pub async fn recover_chat_draft_submission(
    state: State<'_, AIOrchestrator>,
    runtime: State<'_, Arc<crate::chat::runtime::ChatRuntimeState>>,
    character_id: String,
    client_request_id: String,
) -> Result<ChatDraftSubmissionStatus, KokoroError> {
    recover_chat_draft_submission_inner(&state.db, &runtime, &character_id, &client_request_id)
        .await
}

/// The IPC future only waits on the application task; dropping a WebView cannot
/// drop execution. Every producer enters this same idempotent admission path.
#[tauri::command]
pub async fn stream_chat(
    app: tauri::AppHandle,
    request: ChatRequest,
) -> Result<StreamChatResponse, KokoroError> {
    let chat = app
        .try_state::<ChatService>()
        .ok_or_else(|| KokoroError::Chat("AI is not available".into()))?;
    chat.submit(request, ChatPolicy::Desktop).await
}

#[tauri::command]
pub async fn retry_chat_persistence(
    app: tauri::AppHandle,
    client_request_id: String,
) -> Result<StreamChatResponse, KokoroError> {
    let chat = app
        .try_state::<ChatService>()
        .ok_or_else(|| KokoroError::Chat("AI is not available".into()))?;
    chat.retry_chat_persistence(client_request_id).await
}
