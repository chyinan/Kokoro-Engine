use crate::ai::context::AIOrchestrator;
use crate::chat::application::{
    approve_tool_approval_inner, cancel_chat_turn_inner, recover_chat_draft_submission_inner,
    reject_tool_approval_inner, submit_chat, ChatDraftSubmissionStatus, ChatRequest,
    PendingToolApprovalState, StreamChatResponse, TurnCancellationState,
};
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{command, Emitter, Manager, State};

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
    cancel_state: State<'_, Arc<TurnCancellationState>>,
    runtime: State<'_, Arc<crate::chat::runtime::ChatRuntimeState>>,
    approval_state: State<'_, Arc<PendingToolApprovalState>>,
) -> Result<(), String> {
    if let Some(resolved_turn_id) = cancel_state.resolve_turn_id(&turn_id).await {
        approval_state.cancel_turn(&resolved_turn_id).await;
    }
    approval_state.cancel_turn(&turn_id).await;
    let outcome = runtime.outcome(&turn_id);
    cancel_chat_turn_inner(turn_id, reason, cancel_state.inner().clone()).await?;
    if let Some(mut outcome) = outcome {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if outcome.borrow().is_some() {
                    return Ok::<(), String>(());
                }
                outcome
                    .changed()
                    .await
                    .map_err(|_| "chat worker stopped".to_string())?;
            }
        })
        .await
        .map_err(|_| {
            "chat_cancel_pending: cleanup is still running; retry after the terminal state"
                .to_string()
        })??;
    }
    Ok(())
}

#[tauri::command]
pub fn is_chat_busy(state: State<'_, AIOrchestrator>) -> bool {
    state.is_chat_busy()
}

#[derive(Serialize, Deserialize)]
pub struct ContextSettings {
    pub strategy: String,
    pub max_message_chars: usize,
}

#[tauri::command]
pub async fn get_context_settings(
    state: State<'_, AIOrchestrator>,
) -> Result<ContextSettings, KokoroError> {
    let (strategy, max_message_chars) = state.get_context_settings().await;
    Ok(ContextSettings {
        strategy,
        max_message_chars,
    })
}

#[tauri::command]
pub async fn set_context_settings(
    state: State<'_, AIOrchestrator>,
    settings: ContextSettings,
) -> Result<(), KokoroError> {
    // Validate strategy
    let strategy = if settings.strategy == "summary" {
        "summary".to_string()
    } else {
        "window".to_string()
    };
    // Clamp max_message_chars to safe range
    let max_chars = settings.max_message_chars.clamp(100, 50_000);

    state
        .set_context_settings(strategy.clone(), max_chars)
        .await;

    // Persist to disk
    let app_data = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let _ = std::fs::create_dir_all(&app_data);
    let path = app_data.join("context_settings.json");
    let json = serde_json::json!({
        "strategy": strategy,
        "max_message_chars": max_chars,
    });
    if let Err(e) = std::fs::write(&path, json.to_string()) {
        tracing::error!(target: "context", "[Context] Failed to persist context_settings: {}", e);
    }

    Ok(())
}

#[tauri::command]
pub fn finish_chat_playback(
    window: tauri::Window,
    request_id: String,
    error: Option<String>,
    runtime: State<'_, Arc<crate::chat::runtime::ChatRuntimeState>>,
) {
    if let Some(snapshot) = runtime.finish_playback(&request_id, window.label(), error) {
        let _ = window.app_handle().emit("chat-runtime-changed", snapshot);
    }
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
    submit_chat(app, request, crate::chat::application::ChatPolicy::Desktop).await
}
