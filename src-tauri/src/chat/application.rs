//! Application-owned chat turn execution.
use crate::actions::executor::{
    action_result_is_success, apply_before_action_args_payload, assistant_tool_call_metadata_value,
    build_action_hook_payload, build_before_action_args_payload,
    execute_tool_calls_with_cancellation, tool_metadata_value, ToolExecutionScope,
};
use crate::actions::tool_settings::ToolSettings;
use crate::actions::{
    build_tool_audit_event, builtin_tool_id, evaluate_permission_decision, ActionContext,
    ActionRegistry, ActionResult, PermissionDecision, ToolAuditInput, ToolCancellationError,
    ToolInvocation,
};
use crate::ai::context::AIOrchestrator;
use crate::ai::context::Message;
use crate::ai::memory_event_ingress::{
    select_memory_ingress_decision, should_use_structured_extraction, MemoryEventIngressOptions,
};
use crate::chat::persistence::{
    self, ConversationTarget, MessageWrite, PendingChatSave, PendingUserSave,
};
use crate::chat::retry::{FailureRetry, Regeneration, RegenerationPlan, ReplyRegeneration};
use crate::chat::tags::{
    extract_translate_tags, merge_continuation_text, merge_round_tool_calls, parse_tool_call_tags,
    strip_leaked_tags, strip_leaked_tool_results, strip_translate_tags, ToolCall,
};
use crate::error::{ChatErrorEvent, KokoroError};
use crate::hooks::types::HookModifyPolicy;
use crate::hooks::{
    BeforeLlmRequestMessage, BeforeLlmRequestPayload, ChatHookPayload, HookEvent, HookPayload,
    HookRuntime,
};
use crate::imagegen::ImageGenService;
use crate::imagegen::WindowSizeState;
use crate::llm::messages::{
    assistant_tool_calls_message, extract_message_text, history_message_to_llm_chat_message,
    render_vision_context_user_message, replace_user_message_with_images, system_message,
    tool_result_message, user_text_message,
};
use crate::llm::provider::LlmChatMessage;
#[cfg(test)]
use crate::llm::provider::LlmStreamEvent;
use crate::llm::service::LlmService;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::{oneshot, Mutex, RwLock};
use uuid::Uuid;

const FAILURE_EVENTS_LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;

fn failure_events_log_path() -> PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("failure_events.jsonl")
}

async fn rotate_failure_events_log_if_needed(path: &Path) -> Result<(), std::io::Error> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    if metadata.len() < FAILURE_EVENTS_LOG_MAX_BYTES {
        return Ok(());
    }

    let rotated_path = path.with_file_name("failure_events.1.jsonl");
    if tokio::fs::try_exists(&rotated_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&rotated_path).await;
    }

    tokio::fs::rename(path, &rotated_path).await
}

async fn append_failure_event_jsonl(
    failure_event: &crate::error::FailureEvent,
) -> Result<(), std::io::Error> {
    let path = failure_events_log_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    rotate_failure_events_log_if_needed(&path).await?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;

    let line = serde_json::to_string(failure_event)
        .unwrap_or_else(|_| "{\"code\":\"FAILURE_EVENT_SERIALIZE_ERROR\"}".to_string());
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await
}

async fn persist_failure_event_to_conversation(
    state: &AIOrchestrator,
    policy: ChatPolicy,
    failure_event: &crate::error::FailureEvent,
    user_message_id: Option<i64>,
    keep_through_message_id: Option<i64>,
    scope: &crate::chat::ownership::MemoryWriteScope,
) -> Result<(), KokoroError> {
    let (Some(conversation_id), Some(character_id)) =
        (&failure_event.conversation_id, &failure_event.character_id)
    else {
        return Ok(());
    };
    let _selection = state.selection.conversation_switch_lock.lock().await;
    scope.ensure_owner_current()?;
    let mut metadata = serde_json::json!({
        "type": "failure_event",
        "event": failure_event,
        "user_message_id": user_message_id.filter(|_| failure_event.retryable),
    });
    if !policy.is_desktop() {
        // Origin is independent of retry permission: a tool may already have
        // run even though the diagnostic still belongs to an external turn.
        metadata["source"] = policy.source().into();
        metadata["external_user_message_id"] = user_message_id.into();
    }
    if let Some(id) = keep_through_message_id.filter(|_| failure_event.retryable) {
        metadata["keep_through_message_id"] = id.into();
    }
    let target = ConversationTarget {
        require_character: false,
        character_id: character_id.clone(),
        conversation_id: conversation_id.clone(),
        create: false,
    };
    persistence::save(
        &state.db,
        &target,
        &[MessageWrite::new(
            "system",
            &failure_event.message,
            Some(metadata.to_string()),
        )],
    )
    .await?;

    Ok(())
}

async fn emit_and_persist_failure_event(
    app: &tauri::AppHandle,
    policy: ChatPolicy,
    state: &AIOrchestrator,
    failure_event: crate::error::FailureEvent,
    user_message_id: Option<i64>,
    keep_through_message_id: Option<i64>,
    scope: &crate::chat::ownership::MemoryWriteScope,
) -> Result<(), KokoroError> {
    policy
        .emit(app, "chat-failure", failure_event.clone())
        .await
        .map_err(|error| KokoroError::Chat(error.to_string()))?;

    if let Err(error) = persist_failure_event_to_conversation(
        state,
        policy,
        &failure_event,
        user_message_id,
        keep_through_message_id,
        scope,
    )
    .await
    {
        // A failed diagnostic write must preserve the original generation error.
        tracing::warn!(target: "chat", "Could not save generation diagnostic: {error}");
    }

    if let Err(error) = append_failure_event_jsonl(&failure_event).await {
        tracing::error!(
            target: "chat",
            "[Chat] failed to append failure_events.jsonl: {}",
            error
        );
    }

    Ok(())
}

#[derive(Debug)]
enum ToolApprovalDecision {
    Approved,
    Rejected { reason: Option<String> },
}

#[derive(Debug)]
struct PendingToolApproval {
    approval_request_id: String,
    turn_id: String,
    tool_id: String,
    tool_name: String,
    args: HashMap<String, String>,
    decision_tx: Option<oneshot::Sender<ToolApprovalDecision>>,
    decision_rx: Option<oneshot::Receiver<ToolApprovalDecision>>,
}

struct TurnCancellationRecord {
    reason: Option<String>,
    cancel_tx: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
struct TurnCancellationInner {
    cancelled: HashMap<String, TurnCancellationRecord>,
    request_to_turn: HashMap<String, String>,
    tombstones: HashMap<String, (Option<String>, std::time::Instant, u64)>,
    tombstone_counter: u64,
}

const TOMBSTONE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const MAX_TOMBSTONES: usize = 256;

fn prune_tombstones(tombstones: &mut HashMap<String, (Option<String>, std::time::Instant, u64)>) {
    let now = std::time::Instant::now();
    tombstones.retain(|_, (_, created, _)| now.duration_since(*created) < TOMBSTONE_TTL);
    if tombstones.len() > MAX_TOMBSTONES {
        let mut entries: Vec<(String, std::time::Instant, u64)> = tombstones
            .iter()
            .map(|(k, (_, created, seq))| (k.clone(), *created, *seq))
            .collect();
        entries.sort_by(|(_, t1, s1), (_, t2, s2)| t1.cmp(t2).then_with(|| s1.cmp(s2)));
        let remove_count = entries.len() - MAX_TOMBSTONES;
        for (k, _, _) in entries.into_iter().take(remove_count) {
            tombstones.remove(&k);
        }
    }
}

pub struct TurnCancellationState {
    inner: RwLock<TurnCancellationInner>,
    finished_tx: tokio::sync::broadcast::Sender<String>,
}

pub(crate) const TURN_CANCELLED_BY_USER_MESSAGE: &str = "turn cancelled by user";

impl Default for TurnCancellationState {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnCancellationState {
    pub fn new() -> Self {
        let (finished_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            inner: RwLock::new(TurnCancellationInner::default()),
            finished_tx,
        }
    }

    pub async fn register_turn(&self, turn_id: &str) {
        self.register_turn_with_request(turn_id, None).await;
    }

    pub async fn register_turn_with_request(&self, turn_id: &str, client_request_id: Option<&str>) {
        let mut inner = self.inner.write().await;
        prune_tombstones(&mut inner.tombstones);

        let tombstone = inner
            .tombstones
            .remove(turn_id)
            .or_else(|| client_request_id.and_then(|req| inner.tombstones.remove(req)));

        if let std::collections::hash_map::Entry::Vacant(e) =
            inner.cancelled.entry(turn_id.to_string())
        {
            if let Some((reason, _, _)) = tombstone {
                let (tx, _) = tokio::sync::watch::channel(true);
                e.insert(TurnCancellationRecord {
                    reason: reason.or_else(|| Some(TURN_CANCELLED_BY_USER_MESSAGE.to_string())),
                    cancel_tx: tx,
                });
            } else {
                let (tx, _) = tokio::sync::watch::channel(false);
                e.insert(TurnCancellationRecord {
                    reason: None,
                    cancel_tx: tx,
                });
            }
        } else if let Some((reason, _, _)) = tombstone {
            if let Some(entry) = inner.cancelled.get_mut(turn_id) {
                if entry.reason.is_none() {
                    entry.reason =
                        reason.or_else(|| Some(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
                }
                entry.cancel_tx.send_replace(true);
            }
        }

        if let Some(req_id) = client_request_id {
            inner
                .request_to_turn
                .insert(req_id.to_string(), turn_id.to_string());
        }
    }

    async fn ensure_turn_not_cancelled(&self, turn_id: &str) -> Result<(), String> {
        if self.is_cancelled(turn_id).await {
            return Err(TURN_CANCELLED_BY_USER_MESSAGE.to_string());
        }
        Ok(())
    }

    async fn build_turn_delta_payload_if_not_cancelled(
        &self,
        turn_id: &str,
        delta: String,
    ) -> Result<serde_json::Value, String> {
        self.ensure_turn_not_cancelled(turn_id).await?;
        Ok(serde_json::json!({
            "turn_id": turn_id,
            "delta": delta,
        }))
    }

    async fn cancel_turn(&self, target_id: &str, reason: Option<String>) -> Result<(), String> {
        let target_trimmed = target_id.trim();
        if target_trimmed.is_empty() {
            return Err("target_id cannot be empty".to_string());
        }

        let mut inner = self.inner.write().await;
        if let Some(entry) = inner.cancelled.get_mut(target_trimmed) {
            if entry.reason.is_none() {
                entry.reason = reason;
            }
            entry.cancel_tx.send_replace(true);
            return Ok(());
        }

        if let Some(turn_id) = inner.request_to_turn.get(target_trimmed).cloned() {
            if let Some(entry) = inner.cancelled.get_mut(&turn_id) {
                if entry.reason.is_none() {
                    entry.reason = reason;
                }
                entry.cancel_tx.send_replace(true);
                return Ok(());
            }
        }

        // Target not yet registered: record cancellation tombstone so subsequent
        // registration inherits the cancelled state.
        inner.tombstone_counter = inner.tombstone_counter.wrapping_add(1);
        let seq = inner.tombstone_counter;
        inner.tombstones.insert(
            target_trimmed.to_string(),
            (reason, std::time::Instant::now(), seq),
        );
        prune_tombstones(&mut inner.tombstones);
        Ok(())
    }

    async fn is_cancelled(&self, turn_id: &str) -> bool {
        let inner = self.inner.read().await;
        if let Some(v) = inner.cancelled.get(turn_id) {
            return *v.cancel_tx.borrow();
        }
        if let Some(tid) = inner.request_to_turn.get(turn_id) {
            if let Some(v) = inner.cancelled.get(tid) {
                return *v.cancel_tx.borrow();
            }
        }
        inner.tombstones.contains_key(turn_id)
    }

    async fn has_turn(&self, turn_id: &str) -> bool {
        let inner = self.inner.read().await;
        inner.cancelled.contains_key(turn_id)
            || inner
                .request_to_turn
                .get(turn_id)
                .is_some_and(|tid| inner.cancelled.contains_key(tid))
            || inner.tombstones.contains_key(turn_id)
    }

    pub async fn subscribe_cancellation(
        &self,
        turn_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<bool>> {
        let inner = self.inner.read().await;
        if let Some(v) = inner.cancelled.get(turn_id) {
            return Some(v.cancel_tx.subscribe());
        }
        if let Some(tid) = inner.request_to_turn.get(turn_id) {
            if let Some(v) = inner.cancelled.get(tid) {
                return Some(v.cancel_tx.subscribe());
            }
        }
        None
    }

    pub async fn resolve_turn_id(&self, target_id: &str) -> Option<String> {
        let inner = self.inner.read().await;
        if inner.cancelled.contains_key(target_id) {
            Some(target_id.to_string())
        } else {
            inner.request_to_turn.get(target_id).cloned()
        }
    }

    async fn clear_turn(&self, turn_id: &str) {
        {
            let mut inner = self.inner.write().await;
            inner.cancelled.remove(turn_id);
            inner.request_to_turn.retain(|_, v| v != turn_id);
            inner.tombstones.remove(turn_id);
        }
        let _ = self.finished_tx.send(turn_id.to_string());
    }

    async fn cancel_turn_and_wait(
        &self,
        target_id: &str,
        reason: Option<String>,
        timeout: std::time::Duration,
        approval_state: &PendingToolApprovalState,
    ) -> Result<(), String> {
        let mut rx = self.finished_tx.subscribe();
        self.cancel_turn(target_id, reason).await?;

        let resolved_turn_id = self.resolve_turn_id(target_id).await;

        // Publish cancellation before dropping approval senders. Otherwise a
        // worker can observe a closed approval channel as an unexpected error.
        if let Some(turn_id) = resolved_turn_id.as_deref() {
            approval_state.cancel_turn(turn_id).await;
        }
        approval_state.cancel_turn(target_id).await;

        let Some(resolved_turn_id) = resolved_turn_id else {
            return Ok(());
        };

        if !self.has_turn(&resolved_turn_id).await {
            return Ok(());
        }

        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            let remaining = timeout.saturating_sub(start.elapsed());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(finished_id)) if finished_id == resolved_turn_id => return Ok(()),
                Ok(Ok(_)) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    if !self.has_turn(&resolved_turn_id).await {
                        return Ok(());
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }
}

pub(super) async fn wait_for_cancel_event(rx: &mut Option<tokio::sync::watch::Receiver<bool>>) {
    if let Some(rx) = rx.as_mut() {
        if *rx.borrow() {
            return;
        }
        let is_ok = rx.wait_for(|&c| c).await.is_ok();
        if !is_ok && !*rx.borrow() {
            std::future::pending::<()>().await;
        }
    } else {
        std::future::pending::<()>().await;
    }
}

pub(super) fn stream_first_chunk_timeout() -> std::time::Duration {
    std::env::var("KOKORO_LLM_FIRST_CHUNK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(90))
}

pub(super) fn stream_chunk_idle_timeout() -> std::time::Duration {
    std::env::var("KOKORO_LLM_CHUNK_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(60))
}

fn tool_execution_timeout() -> std::time::Duration {
    std::env::var("KOKORO_TOOL_EXECUTION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(60))
}

fn chat_turn_preparation_timeout() -> std::time::Duration {
    std::env::var("KOKORO_CHAT_PREPARATION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(60))
}

fn chat_hook_execution_timeout() -> std::time::Duration {
    std::env::var("KOKORO_CHAT_HOOK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(15))
}

fn chat_fallback_execution_timeout() -> std::time::Duration {
    if let Ok(ms) = std::env::var("KOKORO_CHAT_FALLBACK_TIMEOUT_MS") {
        if let Ok(val) = ms.parse::<u64>() {
            return std::time::Duration::from_millis(val);
        }
    }
    std::env::var("KOKORO_CHAT_FALLBACK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(15))
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamPollResult<T> {
    Item(T),
    Cancelled,
    TimedOut(String),
    StreamEnded,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RoundStreamTermination {
    Completed,
    Cancelled,
    TimedOut(String),
    Failed(String),
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RoundExecutionDecision {
    ExecuteTools {
        tool_calls: Vec<ToolCall>,
        cleaned_text: String,
    },
    FinalizeCompleted {
        cleaned_text: String,
    },
    TerminateTimedOut {
        err_msg: String,
        partial_text: String,
    },
    TerminateFailed {
        err_msg: String,
        partial_text: String,
    },
}

#[allow(dead_code)]
pub(crate) fn evaluate_round_stream_outcome(
    termination: RoundStreamTermination,
    round_response: &str,
    native_tool_calls: Vec<ToolCall>,
) -> RoundExecutionDecision {
    match termination {
        RoundStreamTermination::TimedOut(err_msg) => {
            let (cleaned_text, _) = parse_tool_call_tags(round_response);
            let (cleaned_text, _) = extract_translate_tags(&cleaned_text);
            let partial_text = strip_leaked_tags(&cleaned_text);
            RoundExecutionDecision::TerminateTimedOut {
                err_msg,
                partial_text,
            }
        }
        RoundStreamTermination::Failed(err_msg) => {
            let (cleaned_text, _) = parse_tool_call_tags(round_response);
            let (cleaned_text, _) = extract_translate_tags(&cleaned_text);
            let partial_text = strip_leaked_tags(&cleaned_text);
            RoundExecutionDecision::TerminateFailed {
                err_msg,
                partial_text,
            }
        }
        RoundStreamTermination::Cancelled => RoundExecutionDecision::TerminateFailed {
            err_msg: TURN_CANCELLED_BY_USER_MESSAGE.to_string(),
            partial_text: String::new(),
        },
        RoundStreamTermination::Completed => {
            let (cleaned_text, parsed_tool_calls) = parse_tool_call_tags(round_response);
            let (cleaned_text, _) = extract_translate_tags(&cleaned_text);
            let (tool_calls, _) = merge_round_tool_calls(parsed_tool_calls, native_tool_calls);
            if tool_calls.is_empty() {
                RoundExecutionDecision::FinalizeCompleted { cleaned_text }
            } else {
                RoundExecutionDecision::ExecuteTools {
                    tool_calls,
                    cleaned_text,
                }
            }
        }
    }
}

pub async fn poll_stream_with_cancellation_and_timeout<S, T>(
    stream: &mut S,
    cancel_rx: &mut Option<tokio::sync::watch::Receiver<bool>>,
    timeout_duration: std::time::Duration,
) -> StreamPollResult<T>
where
    S: futures::Stream<Item = T> + Unpin,
{
    tokio::select! {
        biased;
        _ = wait_for_cancel_event(cancel_rx) => {
            StreamPollResult::Cancelled
        }
        _ = tokio::time::sleep(timeout_duration) => {
            StreamPollResult::TimedOut(format!("timeout after {}s", timeout_duration.as_secs()))
        }
        item = stream.next() => {
            match item {
                Some(val) => StreamPollResult::Item(val),
                None => StreamPollResult::StreamEnded,
            }
        }
    }
}

async fn ensure_turn_not_cancelled(
    state: &TurnCancellationState,
    turn_id: &str,
) -> Result<(), String> {
    state.ensure_turn_not_cancelled(turn_id).await
}

async fn build_turn_delta_payload_if_not_cancelled(
    state: &TurnCancellationState,
    turn_id: &str,
    delta: String,
    client_request_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut payload = state
        .build_turn_delta_payload_if_not_cancelled(turn_id, delta)
        .await?;
    if let Some(client_request_id) = client_request_id {
        payload["client_request_id"] = serde_json::Value::String(client_request_id.to_string());
    }
    Ok(payload)
}

fn is_turn_cancelled_error_message(message: &str) -> bool {
    message == TURN_CANCELLED_BY_USER_MESSAGE
}

/// Best-effort cleanup of the conversation rows left behind by a cancelled or failed
/// turn: technical rows (assistant_tool_calls / tool_result) matched by `turn_id`, plus
/// the streaming draft when `draft_row_id` is Some. Cleanup must never mask the
/// original error, so failures are only logged.
async fn save_visible_draft(
    state: &AIOrchestrator,
    character_id: &str,
    conversation_id: &str,
    turn_id: &str,
    row_id: Option<i64>,
    text: &str,
    scope: &crate::chat::ownership::MemoryWriteScope,
) -> Option<i64> {
    let _selection = state.selection.conversation_switch_lock.lock().await;
    if !scope.owner_is_current() {
        return row_id;
    }
    let target = ConversationTarget {
        require_character: false,
        character_id: character_id.into(),
        conversation_id: conversation_id.into(),
        create: false,
    };
    match persistence::save_draft(&state.db, &target, row_id, turn_id, text).await {
        Ok(id) => Some(id),
        // This is only an optional checkpoint. The final save still must succeed,
        // either by promoting this draft or inserting a final row when none exists.
        Err(_) => row_id,
    }
}

async fn cleanup_turn_artifacts(
    state: &AIOrchestrator,
    conversation_id: &str,
    turn_id: &str,
    draft_row_id: Option<i64>,
) {
    let extra_row_ids = draft_row_id.into_iter().collect::<Vec<_>>();
    match state
        .delete_turn_artifacts(conversation_id, turn_id, &extra_row_ids)
        .await
    {
        Ok(deleted) => {
            tracing::info!(
                target: "chat",
                "[Chat] Cleaned up {} row(s) for turn {}",
                deleted,
                turn_id
            );
        }
        Err(error) => {
            tracing::error!(
                target: "chat",
                "[Chat] Failed to clean up turn {} artifacts: {}",
                turn_id,
                error
            );
        }
    }
}

async fn delete_empty_conversation_if_unused(
    state: &AIOrchestrator,
    conversation_id: &str,
    expected_turn_id: Option<&str>,
    draft_row_id: Option<i64>,
) -> Result<bool, KokoroError> {
    let _switch_guard = state.selection.conversation_switch_lock.lock().await;

    // 1. Query all message rows currently belonging to this conversation
    let rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
    )
    .bind(conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    // 2. Classify rows into current turn's artifacts vs other messages
    let mut turn_artifact_ids = HashSet::new();
    let mut other_message_count = 0;

    for (id, _role, _content, metadata) in &rows {
        let is_turn_technical = if let Some(expected_turn) = expected_turn_id {
            metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .is_some_and(|meta| {
                    meta.get("turn_id").and_then(|v| v.as_str()) == Some(expected_turn)
                        && matches!(
                            meta.get("type").and_then(|t| t.as_str()),
                            Some("assistant_tool_calls") | Some("tool_result")
                        )
                })
        } else {
            false
        };

        let is_turn_draft = if let Some(expected_draft_id) = draft_row_id {
            *id == expected_draft_id
                && (metadata.is_none()
                    || expected_turn_id
                        .is_some_and(|et| metadata.as_deref().is_some_and(|m| m.contains(et))))
        } else {
            false
        };

        if is_turn_technical || is_turn_draft {
            turn_artifact_ids.insert(*id);
        } else {
            other_message_count += 1;
        }
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    // 3. Delete only the current turn's technical rows / draft by their specific IDs
    // (Never delete wholesale by conversation_id)
    for id in &turn_artifact_ids {
        sqlx::query("DELETE FROM conversation_messages WHERE id = ? AND conversation_id = ?")
            .bind(id)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;
    }

    // 4. If other messages exist (e.g. from pet, Telegram, user, or other turns), keep the conversation!
    if other_message_count > 0 {
        tx.commit()
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;

        // If this conversation is currently active, resync in-memory history to reflect removed turn artifacts
        if state
            .selection
            .current_conversation_id
            .lock()
            .await
            .as_deref()
            == Some(conversation_id)
        {
            let remaining_rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
                "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
            )
            .bind(conversation_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;

            let max_chars = *state.max_message_chars.lock().await;
            let mut history = state.selection.history.lock().await;
            let new_len =
                crate::ai::context::sync_history_window(&mut history, remaining_rows, max_chars);
            let mut boundary = state.selection.memory_history_boundary.lock().await;
            *boundary = (*boundary).min(new_len);
        }

        tracing::info!(
            target: "chat",
            "[Chat] Preserved temporary conversation '{}' because it contains {} real message(s)",
            conversation_id,
            other_message_count
        );
        return Ok(false);
    }

    // 5. Sanity check: Ensure 0 messages remain in conversation_messages for this conversation
    let remaining_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;

    if remaining_count > 0 {
        tx.commit()
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;
        return Ok(false);
    }

    // 6. Delete the empty conversation record
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    // 7. Reset active conversation state if it matched
    {
        let mut current_id = state.selection.current_conversation_id.lock().await;
        if current_id.as_deref() == Some(conversation_id) {
            *current_id = None;
            state.bump_conversation_generation();
            state.selection.history.lock().await.clear();
            *state.selection.memory_history_boundary.lock().await = 0;
            if state.selection.persist_conversation_selection {
                crate::ai::context::AIOrchestrator::persist_conversation_id(None);
            }
        }
    }

    tracing::info!(
        target: "chat",
        "[Chat] Successfully deleted unused empty temporary conversation '{}'",
        conversation_id
    );
    Ok(true)
}

struct TurnCancellationGuard {
    state: Arc<TurnCancellationState>,
    turn_id: Option<String>,
}

impl TurnCancellationGuard {
    fn new(state: Arc<TurnCancellationState>, turn_id: String) -> Self {
        Self {
            state,
            turn_id: Some(turn_id),
        }
    }

    async fn clear_before_retry(&mut self) {
        if let Some(turn_id) = self.turn_id.take() {
            self.state.clear_turn(&turn_id).await;
        }
    }
}

impl Drop for TurnCancellationGuard {
    fn drop(&mut self) {
        let state = Arc::clone(&self.state);
        let Some(turn_id) = self.turn_id.take() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            state.clear_turn(&turn_id).await;
        });
    }
}

pub struct PendingToolApprovalState {
    pending: Mutex<HashMap<String, PendingToolApproval>>,
    resolved: Mutex<HashSet<String>>,
}

impl Default for PendingToolApprovalState {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingToolApprovalState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            resolved: Mutex::new(HashSet::new()),
        }
    }

    async fn register(
        &self,
        turn_id: String,
        tool_id: String,
        tool_name: String,
        args: HashMap<String, String>,
    ) -> String {
        let approval_request_id = Uuid::new_v4().to_string();
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending.lock().await.insert(
            approval_request_id.clone(),
            PendingToolApproval {
                approval_request_id: approval_request_id.clone(),
                turn_id,
                tool_id,
                tool_name,
                args,
                decision_tx: Some(decision_tx),
                decision_rx: Some(decision_rx),
            },
        );
        approval_request_id
    }

    async fn take_receiver(
        &self,
        approval_request_id: &str,
    ) -> Option<oneshot::Receiver<ToolApprovalDecision>> {
        self.pending
            .lock()
            .await
            .get_mut(approval_request_id)
            .and_then(|entry| entry.decision_rx.take())
    }

    async fn resolve(
        &self,
        approval_request_id: &str,
        decision: ToolApprovalDecision,
    ) -> Result<(), KokoroError> {
        if self.resolved.lock().await.contains(approval_request_id) {
            return Err(KokoroError::Validation(format!(
                "Approval request '{}' already resolved",
                approval_request_id
            )));
        }

        let mut entry = self
            .pending
            .lock()
            .await
            .remove(approval_request_id)
            .ok_or_else(|| {
                KokoroError::Validation(format!(
                    "Unknown approval request '{}'",
                    approval_request_id
                ))
            })?;
        let sender = entry.decision_tx.take().ok_or_else(|| {
            KokoroError::Validation(format!(
                "Approval request '{}' for tool '{}' is no longer pending",
                entry.approval_request_id, entry.tool_name
            ))
        })?;
        let _ = (
            &entry.turn_id,
            &entry.tool_id,
            &entry.args,
            &entry.decision_rx,
        );
        sender.send(decision).map_err(|_| {
            KokoroError::Validation(format!(
                "Approval request '{}' for tool '{}' is no longer pending",
                entry.approval_request_id, entry.tool_name
            ))
        })?;

        self.resolved
            .lock()
            .await
            .insert(approval_request_id.to_string());
        Ok(())
    }

    pub async fn cancel_approval(&self, approval_request_id: &str) {
        let mut pending = self.pending.lock().await;
        if let Some(entry) = pending.remove(approval_request_id) {
            drop(entry);
        }
        self.resolved
            .lock()
            .await
            .insert(approval_request_id.to_string());
    }

    pub async fn cancel_turn(&self, turn_id: &str) {
        let mut pending = self.pending.lock().await;
        let mut resolved = self.resolved.lock().await;
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, entry)| entry.turn_id == turn_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            pending.remove(&id);
            resolved.insert(id);
        }
    }

    #[cfg(test)]
    pub async fn is_pending(&self, approval_request_id: &str) -> bool {
        self.pending.lock().await.contains_key(approval_request_id)
    }
}

pub(crate) async fn approve_tool_approval_inner(
    approval_state: &PendingToolApprovalState,
    approval_request_id: String,
) -> Result<(), KokoroError> {
    approval_state
        .resolve(&approval_request_id, ToolApprovalDecision::Approved)
        .await
}

pub(crate) async fn reject_tool_approval_inner(
    approval_state: &PendingToolApprovalState,
    approval_request_id: String,
    reason: Option<String>,
) -> Result<(), KokoroError> {
    approval_state
        .resolve(
            &approval_request_id,
            ToolApprovalDecision::Rejected { reason },
        )
        .await
}

pub(crate) async fn cancel_chat_turn_inner(
    turn_id: String,
    reason: Option<String>,
    cancel_state: Arc<TurnCancellationState>,
    approval_state: &PendingToolApprovalState,
) -> Result<(), String> {
    cancel_state
        .cancel_turn_and_wait(
            &turn_id,
            reason,
            std::time::Duration::from_millis(1000),
            approval_state,
        )
        .await
}

/// Capability policy selected by trusted adapters, outside the serialized request.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum ChatPolicy {
    Desktop,
    External {
        create_conversation: bool,
        image_output: bool,
        source: &'static str,
    },
}

impl ChatPolicy {
    pub(crate) fn is_desktop(self) -> bool {
        matches!(self, Self::Desktop)
    }

    fn allows_tool(self, tool_id: &str) -> bool {
        if let Self::External { image_output, .. } = self {
            return !matches!(
                tool_id,
                "builtin__capture_screen"
                    | "builtin__play_cue"
                    | "builtin__set_background"
                    | "builtin__send_notification"
            ) && (image_output || tool_id != "builtin__generate_image");
        }
        true
    }

    fn source(self) -> &'static str {
        match self {
            Self::Desktop => "chat",
            Self::External { source, .. } => source,
        }
    }

    async fn emit<S: Serialize + Clone>(
        self,
        app: &tauri::AppHandle,
        name: &str,
        payload: S,
    ) -> Result<(), tauri::Error> {
        if self.is_desktop() {
            return crate::chat::runtime::emit_chat_event(app, name, payload).await;
        }
        if let Ok(value) = serde_json::to_value(payload) {
            app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
                .event(name, &value);
        }
        Ok(())
    }
}

pub(crate) fn channel_id(namespace: &str, owner: &str, id: &str) -> String {
    crate::chat::persistence::external_identity(namespace, &[owner, id])
}

#[derive(Default, serde::Deserialize, serde::Serialize, Clone)]
pub struct ChatRequest {
    #[cfg(test)]
    #[serde(skip)]
    pub(crate) preparation_timeout: Option<std::time::Duration>,
    /// Resolved once by the trusted entry boundary; never accepted over IPC.
    #[serde(skip)]
    pub(crate) resolved_target: Option<ConversationTarget>,
    #[serde(skip)]
    pub(crate) memory_scope: Option<Arc<crate::chat::ownership::MemoryWriteScope>>,
    #[serde(skip)]
    pub(crate) admission_fingerprint: Option<String>,
    pub message: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub allow_image_gen: Option<bool>,
    pub images: Option<Vec<String>>,
    /// Trusted adapters submit the original audio before admission. Include it in
    /// the request fingerprint, but never accept it through desktop IPC.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub(crate) audio: Option<ChatAudioInput>,
    /// Original transport text, separate from fallible media preparation.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub(crate) external_input_text: Option<String>,
    /// Compatibility identities calculated by trusted transport adapters only.
    #[serde(skip)]
    pub(crate) previous_request_ids: Vec<String>,
    /// Journal eviction may reload a committed reply, but must never regenerate
    /// an old request whose persisted history was subsequently deleted.
    #[serde(skip)]
    pub(crate) require_saved_external_reply: bool,
    /// One-shot vision context, excluded from the journal and history previews.
    pub camera_frame: Option<String>,
    pub character_id: Option<String>,
    /// If true, the user instruction is hidden. Non-empty assistant replies may
    /// still be saved, while proactive no-op responses persist nothing.
    /// Used for touch interactions and proactive triggers where the instruction shouldn't appear in chat.
    #[serde(default)]
    pub hidden: bool,
    /// Optional caller correlation echoed on turn lifecycle events.
    #[serde(default)]
    pub client_request_id: Option<String>,
    /// If true, this turn is regenerating an assistant reply for the last user message.
    #[serde(default)]
    pub regenerate: bool,
    /// Validate and truncate the failed turn under this request's execution guard.
    pub retry_failure: Option<FailureRetry>,
    /// Validate and replace a saved assistant reply under the same execution guard.
    pub regenerate_reply: Option<ReplyRegeneration>,
    /// Optional target conversation ID. If specified, backend strictly validates that this request
    /// belongs to this conversation; if unspecified, it binds to the active conversation at entry.
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub conversation_generation: Option<u64>,
}

impl ChatRequest {
    pub(crate) fn fingerprint(&self, policy: ChatPolicy) -> Result<String, String> {
        if let Some(fingerprint) = &self.admission_fingerprint {
            return Ok(fingerprint.clone());
        }
        use sha2::{Digest, Sha256};
        let value = if policy.is_desktop() {
            serde_json::to_value((self, policy))
        } else {
            serde_json::to_value((
                policy.source(),
                &self.character_id,
                &self.client_request_id,
                self.external_input_text.as_ref().unwrap_or(&self.message),
                &self.audio,
                if self.external_input_text.is_some() {
                    None
                } else {
                    self.images.as_ref()
                },
            ))
        };
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&value.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
            )
        ))
    }
}

#[derive(Clone)]
pub(crate) struct ChatAudioInput {
    pub data: Vec<u8>,
    pub format: String,
}

impl Serialize for ChatAudioInput {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use sha2::{Digest, Sha256};
        // Do not expand or retain multi-megabyte audio in JSON while hashing a
        // request. The original bytes live only in the pending worker input.
        (&self.format, format!("{:x}", Sha256::digest(&self.data))).serialize(serializer)
    }
}

fn tool_completion_notice_from_catalog(catalog: &str) -> Result<String, KokoroError> {
    let read_notice = |json: &str| {
        let catalog: serde_json::Value = serde_json::from_str(json).ok()?;
        catalog["chat"]["tool_only_completion"]
            .as_str()
            .filter(|notice| !notice.trim().is_empty())
            .map(str::to_owned)
    };
    // A missing translation must not panic after tools have already executed.
    read_notice(catalog)
        .or_else(|| read_notice(include_str!("../../../src/ui/locales/en.json")))
        .ok_or_else(|| KokoroError::Llm("Tool completion notice unavailable".into()))
}

fn tool_only_completion_notice(language: &str) -> Result<String, KokoroError> {
    let catalog = match language.trim().to_lowercase().as_str() {
        "zh" | "zh-cn" | "中文" | "简体中文" => {
            include_str!("../../../src/ui/locales/zh.json")
        }
        "zh-tw" | "zh-hk" | "繁體中文" => include_str!("../../../src/ui/locales/zh-TW.json"),
        "ja" | "ja-jp" | "日本語" => include_str!("../../../src/ui/locales/ja.json"),
        "ko" | "ko-kr" | "한국어" => include_str!("../../../src/ui/locales/ko.json"),
        "ru" | "ru-ru" | "русский" => include_str!("../../../src/ui/locales/ru.json"),
        _ => include_str!("../../../src/ui/locales/en.json"),
    };
    tool_completion_notice_from_catalog(catalog)
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StreamChatResponse {
    #[serde(skip)]
    pub(crate) speech_snapshot: Option<Arc<crate::tts::manager::TtsExecutionSnapshot>>,
    #[serde(default)]
    pub text: String,
    pub conversation_id: String,
    pub user_message_id: Option<i64>,
    #[serde(default)]
    pub assistant_message_id: Option<i64>,
    #[serde(default)]
    pub client_request_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(skip)]
    pub(crate) translation: Option<String>,
    #[serde(skip)]
    pub(crate) images: Vec<ChatImage>,
    #[serde(skip)]
    pub(crate) output_warnings: Vec<String>,
    #[serde(skip)]
    pub(crate) external_media: persistence::ExternalReplyMedia,
    #[serde(skip)]
    pub(crate) newly_committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatImage {
    pub prompt: String,
    pub path: String,
}

#[derive(Serialize, Clone)]
#[allow(dead_code)]
struct ChatImageGenEvent {
    prompt: String,
}

fn build_chat_error_event(
    stage: &str,
    message: &str,
    trace_id: &str,
    retryable: bool,
) -> ChatErrorEvent {
    ChatErrorEvent {
        code: "CHAT_STREAM_ERROR".to_string(),
        stage: stage.to_string(),
        retryable,
        trace_id: trace_id.to_string(),
        message: message.to_string(),
    }
}

fn build_chat_hook_payload(
    conversation_id: Option<String>,
    character_id: &str,
    turn_id: Option<String>,
    message: Option<String>,
    response: Option<String>,
    tool_round: Option<usize>,
    hidden: bool,
) -> HookPayload {
    HookPayload::Chat(ChatHookPayload {
        conversation_id,
        character_id: character_id.to_string(),
        turn_id,
        message,
        response,
        tool_round,
        hidden,
    })
}

fn vision_context_metadata_value(
    observation: &crate::vision::context::VisionObservation,
) -> serde_json::Value {
    serde_json::json!({
        "type": "vision_observation",
        "observation_id": observation.id,
        "captured_at": observation.captured_at.to_rfc3339(),
        "analyzed_at": observation.analyzed_at.to_rfc3339(),
        "source": observation.source.as_str(),
    })
}

fn insert_vision_context_before_latest_user(
    client_messages: &mut Vec<LlmChatMessage>,
    observation: &crate::vision::context::VisionObservation,
) {
    let metadata = vision_context_metadata_value(observation);
    let rendered = plain_llm_message(render_vision_context_user_message(
        &observation.summary,
        Some(&metadata),
    ));
    let index = client_messages
        .iter()
        .rposition(|message| crate::llm::messages::is_user_message(&message.message))
        .unwrap_or(client_messages.len());
    client_messages.insert(index, rendered);
}

fn vision_context_write(
    observation: &crate::vision::context::VisionObservation,
    turn_id: &str,
) -> MessageWrite {
    let mut metadata = vision_context_metadata_value(observation);
    metadata["turn_id"] = serde_json::Value::String(turn_id.to_string());
    MessageWrite::new(
        "context",
        observation.summary.clone(),
        Some(metadata.to_string()),
    )
    .with_save_id(format!("{turn_id}:vision"))
}

fn is_proactive_noop_response(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("PASS")
}

fn build_before_llm_request_payload(
    conversation_id: Option<String>,
    character_id: &str,
    turn_id: Option<String>,
    request_message: String,
    hidden: bool,
    prompt_messages: &[Message],
) -> BeforeLlmRequestPayload {
    BeforeLlmRequestPayload {
        conversation_id,
        character_id: character_id.to_string(),
        turn_id,
        hidden,
        request_message,
        messages: prompt_messages
            .iter()
            .map(|message| BeforeLlmRequestMessage {
                role: message.role.clone(),
                content: message.content.clone(),
            })
            .collect(),
    }
}

fn apply_before_llm_request_payload(
    payload: BeforeLlmRequestPayload,
    original_prompt_messages: &[Message],
) -> Result<(String, Vec<LlmChatMessage>), String> {
    let request_message = payload.request_message;
    let messages = payload
        .messages
        .into_iter()
        .enumerate()
        .map(|(index, message)| {
            let metadata = original_prompt_messages
                .get(index)
                .filter(|original| original.role == message.role)
                .and_then(|original| original.metadata.as_ref());
            history_message_to_llm_chat_message(&message.role, message.content, metadata)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((request_message, messages))
}

#[cfg(test)]
fn build_effective_before_llm_request(
    conversation_id: Option<String>,
    character_id: &str,
    turn_id: Option<String>,
    request_message: String,
    hidden: bool,
    prompt_messages: &[Message],
) -> Result<(String, Vec<LlmChatMessage>), String> {
    let payload = build_before_llm_request_payload(
        conversation_id,
        character_id,
        turn_id,
        request_message,
        hidden,
        prompt_messages,
    );
    apply_before_llm_request_payload(payload, prompt_messages)
}

#[cfg(debug_assertions)]
fn debug_log_llm_messages(
    label: &str,
    messages: &[async_openai::types::chat::ChatCompletionRequestMessage],
) {
    tracing::info!(target: "llm", "[LLM/Debug] {} ({} messages)", label, messages.len());
    for (index, message) in messages.iter().enumerate() {
        let role = match message {
            async_openai::types::chat::ChatCompletionRequestMessage::Developer(_) => "developer",
            async_openai::types::chat::ChatCompletionRequestMessage::System(_) => "system",
            async_openai::types::chat::ChatCompletionRequestMessage::User(_) => "user",
            async_openai::types::chat::ChatCompletionRequestMessage::Assistant(_) => "assistant",
            async_openai::types::chat::ChatCompletionRequestMessage::Tool(_) => "tool",
            async_openai::types::chat::ChatCompletionRequestMessage::Function(_) => "function",
        };
        let text = extract_message_text(message);
        let compact = text.replace('\n', "\\n");
        let preview = if compact.chars().count() > 300 {
            format!("{}...", compact.chars().take(300).collect::<String>())
        } else {
            compact
        };
        tracing::info!(target: "llm", "[LLM/Debug]   #{} role={} text={}", index, role, preview);
    }
}

#[cfg(debug_assertions)]
fn debug_log_rich_llm_messages(label: &str, messages: &[LlmChatMessage]) {
    let plain_messages = messages
        .iter()
        .map(|message| message.message.clone())
        .collect::<Vec<_>>();
    debug_log_llm_messages(label, &plain_messages);
}

fn plain_llm_message(
    message: async_openai::types::chat::ChatCompletionRequestMessage,
) -> LlmChatMessage {
    message.into()
}

fn deny_kind_for_tool_error(error: &str) -> &'static str {
    if error.starts_with("Denied pending approval:") {
        "pending_approval"
    } else if error.starts_with("Denied by fail-closed policy:") {
        "fail_closed"
    } else if error.starts_with("Denied by policy:") {
        "policy_denied"
    } else if error.starts_with("Denied by hook:") {
        "hook_denied"
    } else {
        "execution_error"
    }
}

fn deny_kind_for_outcome(
    outcome: &crate::actions::ToolExecutionOutcome,
    error: &str,
) -> &'static str {
    if let Some(decision) = outcome.permission_decision.as_ref() {
        if let Some(kind) = crate::actions::permission::deny_kind(decision) {
            return kind;
        }
    }
    deny_kind_for_tool_error(error)
}

#[cfg(test)]
fn tool_error_payload_for_test(tool: &str, turn_id: &str, error: &str) -> serde_json::Value {
    serde_json::json!({
        "turn_id": turn_id,
        "tool": tool,
        "error": error,
        "deny_kind": deny_kind_for_tool_error(error),
    })
}

fn base_tool_trace_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "turn_id": turn_id,
        "tool": outcome.tool_name(),
        "tool_id": outcome.tool_id(),
        "source": outcome.tool_source(),
        "server_name": outcome.tool_server_name(),
        "needs_feedback": outcome.tool_needs_feedback(),
        "permission_level": outcome.tool_permission_level(),
        "risk_tags": outcome.tool_risk_tags(),
    })
}

fn tool_error_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
) -> serde_json::Value {
    let mut payload = base_tool_trace_payload(outcome, turn_id);
    payload["error"] = serde_json::Value::String(error.to_string());
    payload["deny_kind"] =
        serde_json::Value::String(deny_kind_for_outcome(outcome, error).to_string());
    payload
}

fn tool_success_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    result: &crate::actions::ActionResult,
) -> serde_json::Value {
    let mut payload = base_tool_trace_payload(outcome, turn_id);
    payload["result"] = serde_json::to_value(result).expect("action result should serialize");
    payload
}

fn pending_tool_trace_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
    approval_request_id: &str,
) -> serde_json::Value {
    let mut payload = tool_error_payload(outcome, turn_id, error);
    payload["approval_request_id"] = serde_json::Value::String(approval_request_id.to_string());
    payload["approval_status"] = serde_json::Value::String("requested".to_string());
    payload
}

fn approved_tool_trace_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    result: &crate::actions::ActionResult,
    approval_request_id: &str,
) -> serde_json::Value {
    let mut payload = tool_success_payload(outcome, turn_id, result);
    payload["approval_request_id"] = serde_json::Value::String(approval_request_id.to_string());
    payload["approval_status"] = serde_json::Value::String("approved".to_string());
    payload
}

fn rejected_tool_trace_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
    approval_request_id: &str,
) -> serde_json::Value {
    let mut payload = tool_error_payload(outcome, turn_id, error);
    payload["approval_request_id"] = serde_json::Value::String(approval_request_id.to_string());
    payload["approval_status"] = serde_json::Value::String("rejected".to_string());
    payload
}

#[cfg(test)]
fn pending_tool_trace_payload_for_test(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
    approval_request_id: &str,
) -> serde_json::Value {
    pending_tool_trace_payload(outcome, turn_id, error, approval_request_id)
}

#[cfg(test)]
fn approved_tool_trace_payload_for_test(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    result: &crate::actions::ActionResult,
    approval_request_id: &str,
) -> serde_json::Value {
    approved_tool_trace_payload(outcome, turn_id, result, approval_request_id)
}

#[cfg(test)]
fn rejected_tool_trace_payload_for_test(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
    approval_request_id: &str,
) -> serde_json::Value {
    rejected_tool_trace_payload(outcome, turn_id, error, approval_request_id)
}

async fn emit_tool_trace_event(
    app: &tauri::AppHandle,
    policy: ChatPolicy,
    tool_trace_id: &str,
    mut payload: serde_json::Value,
) -> Result<(), tauri::Error> {
    payload["tool_trace_id"] = serde_json::Value::String(tool_trace_id.to_owned());
    policy.emit(app, "chat-turn-tool", payload).await
}

#[allow(clippy::too_many_arguments)]
async fn execute_single_tool_after_approval(
    app: &tauri::AppHandle,
    registry_state: &std::sync::Arc<RwLock<ActionRegistry>>,
    tool_settings_state: &std::sync::Arc<RwLock<ToolSettings>>,
    character_id: &str,
    conversation_id: Option<&str>,
    tool_call: &ToolInvocation,
    expected_registry_generation: u64,
    request_id: &str,
    turn_id: &str,
    cue_names: Option<&Arc<HashSet<String>>>,
    memory_scope: &Arc<crate::chat::ownership::MemoryWriteScope>,
) -> Result<ActionResult, String> {
    let hook_runtime = app.try_state::<HookRuntime>();
    let (registry_generation, resolved) = {
        let registry = registry_state.read().await;
        (
            registry.generation(),
            registry.resolve_action_for_execution(&tool_call.name),
        )
    };
    if registry_generation != expected_registry_generation {
        return Err(
            "Tool changed while approval was pending; approval is no longer valid".to_string(),
        );
    }
    let (action, handler) = resolved.map_err(|error| error.0.clone())?;

    let permission_decision = {
        let settings = tool_settings_state.read().await;
        if !settings.is_enabled(&action.id) {
            return Err(format!("Tool '{}' is disabled", action.id));
        }
        evaluate_permission_decision(&action, &settings)
    };
    if !matches!(permission_decision, PermissionDecision::Allow) {
        return Err(
            crate::actions::permission::decision_reason(&permission_decision)
                .unwrap_or("tool permission policy denied")
                .to_string(),
        );
    }
    let mut args_payload = build_before_action_args_payload(
        conversation_id.map(str::to_owned),
        character_id,
        Some("chat".to_string()),
        tool_call,
        &action,
    );
    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_before_action_args_modify(&mut args_payload, HookModifyPolicy::Strict)
            .await?;
    }
    let effective_args = apply_before_action_args_payload(args_payload);
    let ctx = ActionContext {
        memory_scope: Some(memory_scope.clone()),
        cue_names: cue_names.cloned(),
        request_id: Some(request_id.into()),
        turn_id: Some(turn_id.into()),
        app: app.clone(),
        character_id: character_id.to_string(),
        conversation_id: conversation_id.map(str::to_owned),
        source: Some("chat".to_string()),
    };
    let result = crate::actions::registry::dispatch_handler(handler.as_ref(), effective_args, ctx)
        .await
        .map_err(|e| e.0);
    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_best_effort(
                &HookEvent::AfterActionInvoke,
                &build_action_hook_payload(
                    conversation_id.map(str::to_owned),
                    character_id,
                    Some("chat".to_string()),
                    tool_call,
                    Some(&action),
                    Some(action_result_is_success(&result)),
                    Some(match &result {
                        Ok(value) => value.message.clone(),
                        Err(error) => error.clone(),
                    }),
                ),
            )
            .await;
    }
    result
}

fn rejected_pending_approval_message(reason: Option<String>) -> String {
    match reason {
        Some(reason) if !reason.trim().is_empty() => format!("Denied pending approval: {}", reason),
        _ => "Denied pending approval: rejected by user".to_string(),
    }
}

fn approved_tool_error_payload(
    outcome: &crate::actions::ToolExecutionOutcome,
    turn_id: &str,
    error: &str,
    approval_request_id: &str,
) -> serde_json::Value {
    let mut payload = tool_error_payload(outcome, turn_id, error);
    payload["approval_request_id"] = serde_json::Value::String(approval_request_id.to_string());
    payload["approval_status"] = serde_json::Value::String("approved".to_string());
    payload
}

async fn wait_for_tool_approval_decision(
    approval_state: &PendingToolApprovalState,
    cancel_state: &TurnCancellationState,
    turn_id: &str,
    approval_request_id: &str,
    receiver: oneshot::Receiver<ToolApprovalDecision>,
) -> Result<ToolApprovalDecision, KokoroError> {
    ensure_turn_not_cancelled(cancel_state, turn_id)
        .await
        .map_err(KokoroError::Chat)?;

    let mut cancel_rx = cancel_state.subscribe_cancellation(turn_id).await;

    let decision = tokio::select! {
        biased;
        _ = wait_for_cancel_event(&mut cancel_rx) => {
            tracing::info!(
                target: "chat::tools",
                "[ToolApproval] Approval waiting cancelled for turn {} and request {}",
                turn_id,
                approval_request_id
            );
            approval_state.cancel_approval(approval_request_id).await;
            return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
        }
        res = receiver => {
            match res {
                Ok(decision) => decision,
                Err(_) => {
                    if cancel_state.is_cancelled(turn_id).await {
                        approval_state.cancel_approval(approval_request_id).await;
                        return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
                    }
                    return Err(KokoroError::Validation(format!(
                        "Approval request '{}' was dropped",
                        approval_request_id
                    )));
                }
            }
        }
    };

    ensure_turn_not_cancelled(cancel_state, turn_id)
        .await
        .map_err(KokoroError::Chat)?;

    Ok(decision)
}

struct ToolApprovalExecutionContext<'a> {
    memory_scope: &'a Arc<crate::chat::ownership::MemoryWriteScope>,
    cue_names: Option<&'a Arc<HashSet<String>>>,
    request_id: &'a str,
    app: &'a tauri::AppHandle,
    approval_state: &'a PendingToolApprovalState,
    registry_state: &'a std::sync::Arc<RwLock<ActionRegistry>>,
    tool_settings_state: &'a std::sync::Arc<RwLock<ToolSettings>>,
    character_id: &'a str,
    conversation_id: Option<&'a str>,
    turn_id: &'a str,
    tool_trace_id: &'a str,
    cancel_state: &'a TurnCancellationState,
}

async fn wait_for_tool_approval_and_execute(
    ctx: &ToolApprovalExecutionContext<'_>,
    outcome: &crate::actions::ToolExecutionOutcome,
    pending_error: &str,
) -> Result<(Result<ActionResult, String>, serde_json::Value), KokoroError> {
    ensure_turn_not_cancelled(ctx.cancel_state, ctx.turn_id)
        .await
        .map_err(KokoroError::Chat)?;

    let approval_request_id = ctx
        .approval_state
        .register(
            ctx.turn_id.to_string(),
            outcome.tool_id().to_string(),
            outcome.tool_name().to_string(),
            outcome.invocation.args.clone(),
        )
        .await;
    let requested_payload =
        pending_tool_trace_payload(outcome, ctx.turn_id, pending_error, &approval_request_id);
    let receiver = ctx
        .approval_state
        .take_receiver(&approval_request_id)
        .await
        .ok_or_else(|| {
            KokoroError::Internal("Missing approval receiver after registration".to_string())
        })?;

    if let Err(e) = ensure_turn_not_cancelled(ctx.cancel_state, ctx.turn_id).await {
        ctx.approval_state
            .cancel_approval(&approval_request_id)
            .await;
        return Err(KokoroError::Chat(e));
    }

    emit_tool_trace_event(
        ctx.app,
        ChatPolicy::Desktop,
        ctx.tool_trace_id,
        requested_payload,
    )
    .await
    .map_err(|e| KokoroError::Chat(e.to_string()))?;

    let decision = wait_for_tool_approval_decision(
        ctx.approval_state,
        ctx.cancel_state,
        ctx.turn_id,
        &approval_request_id,
        receiver,
    )
    .await?;

    match decision {
        ToolApprovalDecision::Approved => {
            // Guard: turn MUST NOT be cancelled before executing approved tool
            ensure_turn_not_cancelled(ctx.cancel_state, ctx.turn_id)
                .await
                .map_err(KokoroError::Chat)?;

            let mut tool_cancel_rx = ctx.cancel_state.subscribe_cancellation(ctx.turn_id).await;
            let timeout_duration = tool_execution_timeout();

            let result = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut tool_cancel_rx) => {
                    tracing::info!(
                        target: "chat::tools",
                        "[ToolApproval] Approved tool execution interrupted by cancellation for turn {}",
                        ctx.turn_id
                    );
                    return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
                }
                res = tokio::time::timeout(timeout_duration, execute_single_tool_after_approval(
                    ctx.app,
                    ctx.registry_state,
                    ctx.tool_settings_state,
                    ctx.character_id,
                    ctx.conversation_id,
                    &outcome.invocation,
                    outcome.registry_generation,
                    ctx.request_id,
                    ctx.turn_id,
                    ctx.cue_names,
                    ctx.memory_scope,
                )) => {
                    match res {
                        Ok(exec_res) => exec_res,
                        Err(_) => Err(format!(
                            "Tool '{}' execution timed out after {}s",
                            outcome.tool_name(),
                            timeout_duration.as_secs()
                        )),
                    }
                }
            };

            // Guard: turn MUST NOT be cancelled after tool execution
            ensure_turn_not_cancelled(ctx.cancel_state, ctx.turn_id)
                .await
                .map_err(KokoroError::Chat)?;

            let payload = match &result {
                Ok(value) => {
                    approved_tool_trace_payload(outcome, ctx.turn_id, value, &approval_request_id)
                }
                Err(error) => {
                    approved_tool_error_payload(outcome, ctx.turn_id, error, &approval_request_id)
                }
            };
            Ok((result, payload))
        }
        ToolApprovalDecision::Rejected { reason } => {
            ensure_turn_not_cancelled(ctx.cancel_state, ctx.turn_id)
                .await
                .map_err(KokoroError::Chat)?;

            let rejected_message = rejected_pending_approval_message(reason);
            let payload = rejected_tool_trace_payload(
                outcome,
                ctx.turn_id,
                &rejected_message,
                &approval_request_id,
            );
            Ok((Err(rejected_message), payload))
        }
    }
}

#[cfg(test)]
fn sample_action_result(message: &str) -> crate::actions::ActionResult {
    crate::actions::ActionResult {
        success: true,
        message: message.to_string(),
        data: None,
    }
}

#[cfg(test)]
fn tool_trace_error_deny_kind(error: &str) -> Option<String> {
    tool_error_payload_for_test("tool", "turn-1", error)
        .get("deny_kind")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

#[cfg(test)]
fn tool_trace_error_message(error: &str) -> Option<String> {
    tool_error_payload_for_test("tool", "turn-1", error)
        .get("error")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

#[cfg(test)]
fn sample_tool_trace_outcome_for_test() -> crate::actions::ToolExecutionOutcome {
    crate::actions::ToolExecutionOutcome {
        invocation: crate::actions::ToolInvocation {
            tool_call_id: Some("call-1".to_string()),
            name: "read_file".to_string(),
            args: HashMap::from([("path".to_string(), "README.md".to_string())]),
        },
        action: Some(crate::actions::ActionInfo {
            id: "mcp__filesystem__read_file".to_string(),
            name: "read_file".to_string(),
            source: crate::actions::ActionSource::Mcp,
            server_name: Some("filesystem".to_string()),
            description: "Read file".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![crate::actions::registry::ActionRiskTag::Read],
            permission_level: crate::actions::registry::ActionPermissionLevel::Safe,
        }),
        registry_generation: 0,
        result: Ok(sample_action_result("ok")),
        needs_feedback: true,
        permission_decision: Some(crate::actions::PermissionDecision::Allow),
    }
}

#[cfg(test)]
fn sample_tool_outcome_with_decision(
    permission_decision: crate::actions::PermissionDecision,
    result: Result<crate::actions::ActionResult, String>,
) -> crate::actions::ToolExecutionOutcome {
    crate::actions::ToolExecutionOutcome {
        invocation: crate::actions::ToolInvocation {
            tool_call_id: Some("call-1".to_string()),
            name: "read_file".to_string(),
            args: HashMap::new(),
        },
        action: Some(crate::actions::ActionInfo {
            id: "mcp__filesystem__read_file".to_string(),
            name: "read_file".to_string(),
            source: crate::actions::ActionSource::Mcp,
            server_name: Some("filesystem".to_string()),
            description: "Read file".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![crate::actions::registry::ActionRiskTag::Read],
            permission_level: crate::actions::registry::ActionPermissionLevel::Safe,
        }),
        registry_generation: 0,
        result,
        needs_feedback: true,
        permission_decision: Some(permission_decision),
    }
}

#[cfg(test)]
fn tool_trace_success_has_no_deny_kind() -> bool {
    tool_success_payload(
        &sample_tool_trace_outcome_for_test(),
        "turn-1",
        &sample_action_result("ok"),
    )
    .get("deny_kind")
    .is_none()
}

#[cfg(test)]
fn tool_trace_success_message() -> Option<String> {
    tool_success_payload(
        &sample_tool_trace_outcome_for_test(),
        "turn-1",
        &sample_action_result("ok"),
    )
    .get("result")
    .and_then(|value| value.get("message"))
    .and_then(|value| value.as_str())
    .map(ToString::to_string)
}

#[inline]
pub(crate) fn should_insert_user_message_for_request(
    hidden: bool,
    regenerate: bool,
    has_trailing_user_message: bool,
) -> bool {
    if hidden {
        return false;
    }
    if regenerate {
        !has_trailing_user_message
    } else {
        true
    }
}

pub(crate) async fn resolve_trailing_visible_user_message(
    db: &sqlx::SqlitePool,
    conversation_id: &str,
) -> Result<Option<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>)>(
        "SELECT id, role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id DESC LIMIT 50"
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await?;

    for (id, role, content, metadata) in rows {
        if !crate::chat::tags::is_visible_message_raw(&role, metadata.as_deref()) {
            continue;
        }

        if role == "user" {
            return Ok(Some((id, content)));
        } else {
            return Ok(None);
        }
    }
    Ok(None)
}

pub(crate) fn ensure_client_request_id(client_request_id: &mut Option<String>) {
    if client_request_id
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        *client_request_id = Some(format!("req_backend_{}", uuid::Uuid::new_v4()));
    }
}

pub(crate) fn resolve_turn_user_message_id(
    hidden: bool,
    trailing_visible_user: Option<i64>,
) -> Option<i64> {
    if hidden {
        None
    } else {
        trailing_visible_user
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatDraftSubmissionStatus {
    Pending,
    Persisted,
    Unsent,
}

pub(crate) async fn recover_chat_draft_submission_inner(
    db: &sqlx::SqlitePool,
    runtime: &crate::chat::runtime::ChatRuntimeState,
    character_id: &str,
    client_request_id: &str,
) -> Result<ChatDraftSubmissionStatus, KokoroError> {
    if runtime.turn_scope(client_request_id).is_some_and(|turn| {
        turn.character_id.as_deref() == Some(character_id) && turn.user_save_failed
    }) {
        return Ok(ChatDraftSubmissionStatus::Unsent);
    }
    let pending = runtime.recover_draft_admission(character_id, client_request_id);
    let persisted: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM conversation_messages AS message
            JOIN conversations AS conversation ON conversation.id = message.conversation_id
            WHERE conversation.character_id = ? AND message.role = 'user' AND message.metadata IS NOT NULL
              AND (CASE WHEN json_valid(message.metadata)
                        THEN json_extract(message.metadata, '$.client_request_id') END) = ?
        )",
    )
    .bind(character_id)
    .bind(client_request_id)
    .fetch_one(db)
    .await?;
    Ok(if persisted {
        ChatDraftSubmissionStatus::Persisted
    } else if pending {
        ChatDraftSubmissionStatus::Pending
    } else {
        ChatDraftSubmissionStatus::Unsent
    })
}

// ── Stream Chat Command ────────────────────────────────────

#[cfg(test)]
pub(super) async fn external_request_exists(
    db: &sqlx::SqlitePool,
    source: &str,
    request_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM conversation_messages
         WHERE role = 'user' AND metadata IS NOT NULL
           AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.client_request_id') END = ?
           AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.source') END = ?)",
    )
    .bind(request_id)
    .bind(source)
    .fetch_one(db)
    .await
}

async fn validate_conversation_character_id(
    db: &sqlx::SqlitePool,
    conversation_id: &str,
    character_id: &str,
) -> Result<(), KokoroError> {
    let owner =
        sqlx::query_scalar::<_, String>("SELECT character_id FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(db)
            .await
            .map_err(|error| KokoroError::Database(format!("{error:#}")))?;

    if owner.as_deref() != Some(character_id) {
        return Err(KokoroError::Validation(
            "Conversation is not available for this character".to_string(),
        ));
    }

    Ok(())
}

struct PreparedChatTurn {
    prompt_snapshot: crate::chat::ownership::PromptSnapshot,
    speech: Option<crate::characters::activation::ResolvedTts>,
    speech_snapshot: Option<Arc<crate::tts::manager::TtsExecutionSnapshot>>,
    char_id: String,
    conversation_id: Option<String>,
    user_message_id: Option<i64>,
    bound_generation: u64,
    memory_epoch: u64,
    regeneration: Option<RegenerationPlan>,
    selected_vision_observation: Option<crate::vision::context::VisionObservation>,
    prompt_messages: Vec<crate::ai::context::Message>,
    llm_config: crate::llm::llm_config::LlmConfig,
    chat_provider: std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
    effective_provider_id: String,
    native_tools_enabled: bool,
    native_tools: Vec<crate::llm::provider::LlmToolDefinition>,
    system_provider: std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
    memory_target_language: String,
    external_turn: Option<persistence::ExternalTurn>,
}

/// Compatibility lives here: UI defaults are read once, before journal admission.
async fn resolve_request_scope(
    state: &AIOrchestrator,
    request: &mut ChatRequest,
    policy: ChatPolicy,
) -> Result<(), KokoroError> {
    if request.resolved_target.is_some() {
        return Ok(());
    }
    ensure_client_request_id(&mut request.client_request_id);
    request.admission_fingerprint = Some(request.fingerprint(policy).map_err(KokoroError::Chat)?);
    let _activation = state.enter_chat_turn().map_err(KokoroError::Chat)?;
    let _selection = state.selection.conversation_switch_lock.lock().await;
    let selected_character = state.get_character_id().await;
    let character = request
        .character_id
        .clone()
        .or_else(|| policy.is_desktop().then_some(selected_character.clone()))
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| KokoroError::Validation("Chat character is missing".into()))?;
    if policy.is_desktop()
        && (character != selected_character
            || request
                .conversation_generation
                .is_some_and(|generation| generation != state.current_conversation_generation()))
    {
        return Err(KokoroError::Validation(
            "Desktop selection changed before admission".into(),
        ));
    }
    let character_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM characters WHERE id=?)")
            .bind(&character)
            .fetch_one(&state.db)
            .await?;
    if !character_exists {
        return Err(KokoroError::Validation(
            "Chat character no longer exists".into(),
        ));
    }
    let mut conversation = request.conversation_id.clone();
    if policy.is_desktop() && conversation.is_none() {
        conversation = state.selection.current_conversation_id.lock().await.clone();
    }
    if !policy.is_desktop() {
        let saved: Option<String> = sqlx::query_scalar("SELECT m.conversation_id FROM conversation_messages m JOIN conversations c ON c.id=m.conversation_id WHERE c.character_id=? AND m.role='user' AND CASE WHEN json_valid(m.metadata) THEN json_extract(m.metadata,'$.chat_save_id') END=?")
            .bind(&character).bind(format!("{}:user", request.client_request_id.as_deref().unwrap())).fetch_optional(&state.db).await.map_err(KokoroError::chat_persistence)?;
        if saved.is_some() {
            conversation = saved;
        }
    }
    let conversation = conversation
        .or_else(|| policy.is_desktop().then(|| Uuid::new_v4().to_string()))
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| KokoroError::Validation("Chat conversation is missing".into()))?;
    let owner: Option<String> =
        sqlx::query_scalar("SELECT character_id FROM conversations WHERE id=?")
            .bind(&conversation)
            .fetch_optional(&state.db)
            .await?;
    let create = owner.is_none();
    if let Some(owner) = owner {
        if owner != character {
            return Err(KokoroError::Validation(
                "Conversation is not available for this character".into(),
            ));
        }
    } else if request.conversation_id.is_some()
        && (policy.is_desktop()
            || matches!(
                policy,
                ChatPolicy::External {
                    create_conversation: false,
                    ..
                }
            ))
    {
        return Err(KokoroError::Validation(
            "Chat conversation no longer exists".into(),
        ));
    }
    let target = ConversationTarget {
        require_character: true,
        character_id: character.clone(),
        conversation_id: conversation.clone(),
        create,
    };
    crate::chat::ownership::TurnIdentity::new(
        &character,
        &conversation,
        request.client_request_id.as_deref().unwrap(),
    )?;
    request.character_id = Some(character);
    request.conversation_id = Some(conversation);
    if policy.is_desktop() {
        request.conversation_generation = Some(state.current_conversation_generation());
    }
    let mut memory_scope = state.memory_write_scope(
        &target.character_id,
        &target.conversation_id,
        request.client_request_id.as_deref().unwrap(),
        state.external_memory.epoch(),
    );
    // A hidden turn may run role-memory tools before its first message commit.
    // Grant absence only for the target just validated under this selection lock.
    Arc::get_mut(&mut memory_scope)
        .unwrap()
        .allow_reserved_conversation = target.create;
    request.memory_scope = Some(memory_scope);
    request.resolved_target = Some(target);
    Ok(())
}

pub(crate) async fn submit_chat(
    app: tauri::AppHandle,
    mut request: ChatRequest,
    policy: ChatPolicy,
) -> Result<StreamChatResponse, KokoroError> {
    // Own the gate through journal completion, including before the worker is
    // polled. Import must not invalidate a turn which can still write or finish.
    let database_operation_guard = crate::ai::context::try_acquire_database_operation_read_guard()
        .map_err(|error| KokoroError::Chat(error.into()))?;
    if policy.is_desktop() {
        if let Some(outcome) = app
            .state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
            .desktop_replay(&request)
            .map_err(KokoroError::Chat)?
        {
            let mut response = wait_chat_outcome(outcome).await?;
            response.newly_committed = false;
            return Ok(response);
        }
    }
    resolve_request_scope(app.state::<AIOrchestrator>().inner(), &mut request, policy).await?;
    let request_id = request.client_request_id.clone().unwrap();
    let runtime = app
        .state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
        .inner()
        .clone();
    let (created, outcome, replay_only) = runtime
        .begin_with_policy(&request, policy)
        .map_err(KokoroError::Chat)?;
    request.require_saved_external_reply = replay_only;
    if created {
        let execution_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _database_operation_guard = database_operation_guard;
            run_chat_worker(execution_app, runtime, request_id, request, policy, None).await;
        });
    }
    let mut response = wait_chat_outcome(outcome).await?;
    if !created {
        response.newly_committed = false;
    }
    Ok(response)
}

async fn wait_chat_outcome(
    mut outcome: tokio::sync::watch::Receiver<Option<Result<StreamChatResponse, KokoroError>>>,
) -> Result<StreamChatResponse, KokoroError> {
    loop {
        if let Some(result) = outcome.borrow().clone() {
            return result;
        }
        outcome
            .changed()
            .await
            .map_err(|_| KokoroError::Chat("chat worker stopped".into()))?;
    }
}

async fn run_chat_worker(
    app: tauri::AppHandle,
    runtime: Arc<crate::chat::runtime::ChatRuntimeState>,
    request_id: String,
    request: ChatRequest,
    policy: ChatPolicy,
    pending_user_save: Option<Box<PendingUserSave>>,
) {
    // Catch panics as an explicit terminal error, so reconnect never leaves
    // a request permanently pending after a worker failure.
    let execution = execute_chat_with_pending(
        &app,
        request,
        policy,
        ChatDependencies::managed(&app),
        pending_user_save,
    );
    let execution = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(execution))
        .await
        .unwrap_or_else(|_| Err(KokoroError::Chat("chat worker panicked".into())));
    let mut result = match execution {
        Err(KokoroError::Chat(ref message)) if is_turn_cancelled_error_message(message) => runtime
            .cancelled_response(&request_id)
            .ok_or_else(|| KokoroError::Chat(message.clone())),
        other => other,
    };
    if let Ok(response) = &mut result {
        if response.status.as_deref() != Some("cancelled") {
            if let Some(text) = runtime.turn_text(&request_id) {
                response.text = text;
            }
        }
    }
    if let Err(KokoroError::ChatPersistence(message)) = &result {
        if let Some(scope) = runtime.turn_scope(&request_id) {
            let failure = ChatErrorEvent {
                code: KokoroError::CHAT_PERSISTENCE_CODE.into(),
                stage: "persistence".into(),
                retryable: false,
                trace_id: scope.turn_id.clone().unwrap_or_else(|| request_id.clone()),
                message: message.clone(),
            }
            .into_failure_event(
                scope.conversation_id.clone(),
                scope.turn_id.clone(),
                scope.character_id,
                None,
            );
            let _ = policy.emit(&app, "chat-failure", failure).await;
            let _ = policy
                .emit(
                    &app,
                    "chat-turn-finish",
                    serde_json::json!({
                        "turn_id": scope.turn_id,
                        "client_request_id": request_id,
                        "conversation_id": scope.conversation_id,
                        "status": "error",
                        "stage": "persistence",
                        "code": KokoroError::CHAT_PERSISTENCE_CODE,
                    }),
                )
                .await;
        }
    }
    if let Some(snapshot) = runtime.complete(&request_id, result) {
        let _ = app.emit("chat-runtime-changed", snapshot);
    }
}

pub async fn retry_chat_persistence(
    app: tauri::AppHandle,
    client_request_id: String,
) -> Result<StreamChatResponse, KokoroError> {
    let runtime = app
        .state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
        .inner()
        .clone();
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let app = worker_app;
        let _retry = runtime.lock_save_retry().await;
        let _database_guard = crate::ai::context::try_acquire_database_operation_read_guard()
            .map_err(|error| KokoroError::Chat(error.into()))?;
        let state = app.state::<AIOrchestrator>();
        let pending = match runtime.save_retry(&client_request_id)? {
            crate::chat::runtime::SaveRetry::Saved(response) => return Ok(*response),
            crate::chat::runtime::SaveRetry::Pending(pending) => pending,
            crate::chat::runtime::SaveRetry::InFlight(outcome) => {
                drop(_retry);
                return wait_chat_outcome(outcome).await;
            }
            crate::chat::runtime::SaveRetry::User(pending) => {
                if state.is_chat_busy() {
                    return Err(KokoroError::Chat(
                        "chat_turn_busy: A chat turn is already in progress".into(),
                    ));
                }
                let snapshot = runtime
                    .start_user_save_retry(&client_request_id)
                    .ok_or_else(|| {
                        KokoroError::Validation("No failed chat save is available to retry".into())
                    })?;
                let outcome = runtime
                    .outcome(&client_request_id)
                    .expect("the retried turn is active");
                let _ = app.emit("chat-runtime-changed", snapshot);
                drop(_retry);
                let request = pending.request.clone();
                run_chat_worker(
                    app.clone(),
                    runtime,
                    client_request_id,
                    request,
                    ChatPolicy::Desktop,
                    Some(pending),
                )
                .await;
                return wait_chat_outcome(outcome).await;
            }
        };
        let _chat_guard = state.enter_chat_turn().map_err(KokoroError::Chat)?;
        let _execution = state
            .try_acquire_chat_turn(&client_request_id)
            .await
            .map_err(KokoroError::Chat)?;
        let _selection = state.selection.conversation_switch_lock.lock().await;
        let select_on_commit = pending.target.create
            && pending.creation_generation == Some(state.current_conversation_generation())
            && state.get_character_id().await == pending.target.character_id
            && state
                .selection
                .current_conversation_id
                .lock()
                .await
                .is_none();
        let result = if pending
            .post_commit
            .scope
            .as_ref()
            .is_some_and(|scope| scope.owner_is_current())
        {
            state
                .save_messages_expected_locked(
                    &pending.target,
                    &pending.messages,
                    select_on_commit,
                    None,
                    Some(pending.expected_revision),
                )
                .await
        } else {
            Err(KokoroError::Validation(
                "Chat owner was deleted or replaced".into(),
            ))
        };
        let saved = match result {
            Ok(saved) => saved,
            Err(error @ KokoroError::Validation(_)) => {
                if let Some(snapshot) =
                    runtime.retire_save_retry(&client_request_id, error.to_string())
                {
                    let _ = app.emit("chat-runtime-changed", snapshot);
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let assistant_message_id = if pending.has_assistant_text {
            saved.last().map(|message| message.id)
        } else {
            None
        };
        let response = StreamChatResponse {
            text: pending.text.clone(),
            conversation_id: pending.target.conversation_id,
            user_message_id: pending.user_message_id,
            assistant_message_id,
            client_request_id: Some(client_request_id.clone()),
            status: Some("completed".into()),
            translation: None,
            images: Vec::new(),
            output_warnings: Vec::new(),
            ..Default::default()
        };
        let snapshot = runtime.complete_save_retry(
            &client_request_id,
            response.clone(),
            select_on_commit.then(|| state.current_conversation_generation()),
        );
        drop(_selection);
        if let Some(snapshot) = snapshot {
            pending.post_commit.dispatch(app.clone());
            let _ = app.emit("chat-runtime-changed", snapshot);
            let _ = ChatPolicy::Desktop
                .emit(
                    &app,
                    "chat-turn-finish",
                    serde_json::json!({
                        "turn_id": pending.turn_id,
                        "client_request_id": client_request_id,
                        "conversation_id": response.conversation_id,
                        "assistant_message_id": response.assistant_message_id,
                        "status": "completed",
                        "text": response.text,
                    }),
                )
                .await;
            if state.get_character_id().await == pending.target.character_id
                && state
                    .selection
                    .current_conversation_id
                    .lock()
                    .await
                    .as_deref()
                    == Some(response.conversation_id.as_str())
            {
                if let Some(speech) = pending.speech {
                    crate::chat::runtime::speak_completed_turn(
                        app.clone(),
                        client_request_id,
                        pending.turn_id,
                        response.text.clone(),
                        speech,
                        pending.speech_snapshot.clone(),
                    );
                }
            }
        }
        Ok(response)
    })
    .await
    .map_err(|error| KokoroError::Chat(error.to_string()))?
}

pub(crate) fn submit_proactive_bound(
    app: tauri::AppHandle,
    instruction: String,
    scope: (String, Option<String>, u64),
) -> Option<String> {
    if app
        .state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
        .is_playing(&app)
    {
        return None;
    }
    let request_id = format!("proactive_{}", Uuid::new_v4());
    let request = ChatRequest {
        message: instruction,
        hidden: true,
        character_id: Some(scope.0),
        conversation_id: scope.1,
        conversation_generation: Some(scope.2),
        client_request_id: Some(request_id.clone()),
        ..Default::default()
    };
    tauri::async_runtime::spawn(async move {
        if let Err(error) = submit_chat(app, request, ChatPolicy::Desktop).await {
            tracing::debug!(target:"chat","proactive request ended: {error}");
        }
    });
    Some(request_id)
}

struct ChatDependencies<'a> {
    state: &'a AIOrchestrator,
    imagegen: &'a ImageGenService,
    llm: &'a LlmService,
    registry: &'a Arc<RwLock<ActionRegistry>>,
    tool_settings: &'a Arc<RwLock<ToolSettings>>,
    approval: &'a Arc<PendingToolApprovalState>,
    cancellation: &'a Arc<TurnCancellationState>,
    vision: &'a crate::vision::watcher::VisionWatcher,
    window_size: &'a WindowSizeState,
    vision_server: &'a Arc<Mutex<crate::vision::server::VisionServer>>,
}

impl<'a> ChatDependencies<'a> {
    fn managed(app: &'a tauri::AppHandle) -> Self {
        Self {
            state: app.state::<AIOrchestrator>().inner(),
            imagegen: app.state::<ImageGenService>().inner(),
            llm: app.state::<LlmService>().inner(),
            registry: app.state::<Arc<RwLock<ActionRegistry>>>().inner(),
            tool_settings: app.state::<Arc<RwLock<ToolSettings>>>().inner(),
            approval: app.state::<Arc<PendingToolApprovalState>>().inner(),
            cancellation: app.state::<Arc<TurnCancellationState>>().inner(),
            vision: app.state::<crate::vision::watcher::VisionWatcher>().inner(),
            window_size: app.state::<WindowSizeState>().inner(),
            vision_server: app
                .state::<Arc<Mutex<crate::vision::server::VisionServer>>>()
                .inner(),
        }
    }
}

async fn execute_chat_with_pending(
    app: &tauri::AppHandle,
    request: ChatRequest,
    policy: ChatPolicy,
    dependencies: ChatDependencies<'_>,
    pending_user_save: Option<Box<PendingUserSave>>,
) -> Result<StreamChatResponse, KokoroError> {
    let ChatDependencies {
        state,
        imagegen: imagegen_state,
        llm: llm_state,
        registry: action_registry,
        tool_settings: tool_settings_state,
        approval: approval_state,
        cancellation: cancel_state,
        vision: vision_watcher,
        window_size: window_size_state,
        vision_server,
    } = dependencies;
    let desktop = policy.is_desktop();
    // Gate chat if runtime is degraded (e.g. startup recovery failure)
    if let Some(reason) = state.get_runtime_degraded().await {
        return Err(KokoroError::Chat(format!(
            "Character runtime is degraded ({reason}). Please re-activate or select a character in Character Settings before sending messages."
        )));
    }

    let mut request = request;
    persistence::validate_user_input(&request.message)?;
    let target = request
        .resolved_target
        .as_ref()
        .ok_or_else(|| KokoroError::Validation("Unresolved chat identity".into()))?
        .clone();
    let identity = crate::chat::ownership::TurnIdentity::new(
        &target.character_id,
        &target.conversation_id,
        request
            .client_request_id
            .as_deref()
            .ok_or_else(|| KokoroError::Validation("Chat request identity is missing".into()))?,
    )?;
    let memory_scope = request
        .memory_scope
        .clone()
        .ok_or_else(|| KokoroError::Validation("Unresolved chat lifetime".into()))?;
    memory_scope.ensure_owner_current()?;
    let client_request_id = identity.request_id.clone();
    let char_id = identity.character_id.clone();
    let _turn_execution_guard = state
        .try_acquire_chat_turn(&client_request_id)
        .await
        .map_err(KokoroError::Chat)?;
    let assistant_turn_id = pending_user_save.as_ref().map_or_else(
        || Uuid::new_v4().to_string(),
        |pending| pending.turn_id.clone(),
    );
    cancel_state
        .register_turn_with_request(&assistant_turn_id, Some(&client_request_id))
        .await;
    let mut turn_guard =
        TurnCancellationGuard::new(cancel_state.clone(), assistant_turn_id.clone());
    let mut cancel_rx = cancel_state
        .subscribe_cancellation(&assistant_turn_id)
        .await;

    if cancel_state.is_cancelled(&assistant_turn_id).await {
        tracing::info!(
            target: "chat",
            "[stream_chat] Turn {} (request {}) cancelled immediately after turn acquisition",
            assistant_turn_id,
            client_request_id
        );
        return Err(KokoroError::Chat(
            TURN_CANCELLED_BY_USER_MESSAGE.to_string(),
        ));
    }

    policy
        .emit(
            app,
            "chat-turn-acknowledged",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "client_request_id": client_request_id,
                "character_id": char_id,
                "conversation_id": request.conversation_id,
            }),
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(
                target: "chat",
                "[stream_chat] Failed to emit chat-turn-acknowledged: {}",
                e
            );
        });

    let active_conversation_id_holder = std::sync::Arc::new(std::sync::Mutex::new(None));
    let active_conv_id_for_prep = std::sync::Arc::clone(&active_conversation_id_holder);
    let hook_runtime = desktop.then(|| app.try_state::<HookRuntime>()).flatten();

    let preparation_timeout = chat_turn_preparation_timeout();
    #[cfg(test)]
    let preparation_timeout = request.preparation_timeout.unwrap_or(preparation_timeout);

    let prep_fut = async {
        // Activation protects only the snapshot. No activation lock is held across
        // model, tool, media or memory I/O.
        let (prompt_snapshot, llm_snapshot, speech, speech_snapshot) = {
            let _activation = state.enter_chat_turn().map_err(KokoroError::Chat)?;
            let mut prompt = state
                .prompt_snapshot(&char_id)
                .await
                .map_err(|error| KokoroError::Chat(error.to_string()))?;
            prompt.vision_context_history_mode = vision_watcher
                .config
                .read()
                .await
                .vision_context_history_mode
                .clone();
            let providers = llm_state.execution_snapshot().await;
            let committed = app
                .state::<crate::characters::activation::ActivationCoordinator>()
                .get_committed(&state.db)
                .await
                .ok()
                .flatten()
                .filter(|committed| committed.runtime.character_id == char_id);
            if let Some(cues) = committed
                .as_ref()
                .and_then(|committed| crate::live2d::committed_cue_map(&committed.runtime))
            {
                prompt.cue_map = Some(cues);
            }
            let speech = committed.map(|committed| committed.runtime.tts);
            let profile = if !desktop && speech.is_none() {
                let profile: Option<String> =
                    sqlx::query_scalar("SELECT runtime_profile_json FROM characters WHERE id=?")
                        .bind(&char_id)
                        .fetch_optional(&state.db)
                        .await?;
                profile
                    .and_then(|profile| {
                        serde_json::from_str::<crate::characters::manifest::CharacterRuntimeProfile>(
                            &profile,
                        )
                        .ok()
                    })
                    .and_then(|profile| profile.tts)
            } else {
                None
            };
            let speech_snapshot = if let Some(tts) = app.try_state::<crate::tts::TtsService>() {
                tts.execution_snapshot(speech.as_ref(), profile.as_ref())
                    .await
            } else {
                None
            };
            (prompt, providers, speech, speech_snapshot)
        };

        // 0. Resolve character ID for this request (not stored in shared state)
        let char_id = identity.character_id.clone();
        let requested_conversation_id = Some(identity.conversation_id.clone());
        let initial_generation = request.conversation_generation.unwrap_or(0);
        #[cfg(debug_assertions)]
        let lifecycle_hold = if std::env::var_os("KOKORO_TEST_DATA_ROOT").is_some()
            && std::env::var_os("KOKORO_CHAT_LIFECYCLE_TEST_PREPARATION").is_some()
        {
            request
                .message
                .strip_prefix("[case:lifecycle-hold-preparation-")
                .and_then(|phase| phase.strip_suffix(']'))
                .map(str::to_owned)
        } else {
            None
        };
        #[cfg(debug_assertions)]
        if lifecycle_hold
            .as_deref()
            .is_some_and(|phase| phase != "after-selection")
        {
            std::future::pending::<()>().await;
        }
        // Record user activity
        if desktop {
            state.touch_character_activity(&char_id).await;
        }

        // Typing simulation
        if desktop {
            let is_question = request.message.contains('?') || request.message.contains('？');
            let typing_params = crate::ai::typing_sim::calculate_typing_delay(
                "neutral",
                0.6,
                request.message.chars().count(),
                is_question,
            );
            let mut typing_event = serde_json::to_value(&typing_params).unwrap_or_default();
            typing_event["client_request_id"] = client_request_id.clone().into();
            typing_event["turn_id"] = assistant_turn_id.clone().into();
            let _ = policy.emit(app, "chat-typing", typing_event).await;
        }

        // 1. Select current-turn vision context before any user-message persistence.
        let selected_vision_observation = if let Some(pending) = &pending_user_save {
            pending.vision.clone()
        } else if desktop {
            vision_watcher
                .context
                .latest_completed_observation(chrono::Utc::now())
                .await
        } else {
            None
        };

        // 2. Update History with User Message under conversation_switch_lock
        let (llm_config, chat_provider, system_provider) = llm_snapshot.clone();
        let fingerprint = request.fingerprint(policy).map_err(KokoroError::Chat)?;
        let original_input = request
            .external_input_text
            .clone()
            .or_else(|| request.audio.as_ref().map(|_| request.message.clone()));
        if !desktop {
            persistence::validate_external_request_owner(
                &state.db,
                &char_id,
                policy.source(),
                &client_request_id,
                &request.previous_request_ids,
            )
            .await?;
            let saved: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT m.content,m.metadata FROM conversation_messages m JOIN conversations c ON c.id=m.conversation_id WHERE c.character_id=? AND m.role='user' AND CASE WHEN json_valid(m.metadata) THEN json_extract(m.metadata,'$.chat_save_id') END=?"
            ).bind(&char_id).bind(format!("{client_request_id}:user")).fetch_optional(&state.db).await.map_err(KokoroError::chat_persistence)?;
            if request.require_saved_external_reply && saved.is_none() {
                return Err(KokoroError::Chat(
                    "chat_request_conflict: the committed reply is no longer available".into(),
                ));
            }
            if let Some((text, metadata)) = saved {
                let metadata = metadata
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .unwrap_or_default();
                if metadata["external_request_fingerprint"]
                    .as_str()
                    .is_some_and(|old| old != fingerprint)
                {
                    return Err(KokoroError::Chat(
                        "chat_request_conflict: request identity was reused with different input"
                            .into(),
                    ));
                }
                if let Some(original) = original_input.as_deref() {
                    let matches = metadata["external_input_text"].as_str().map_or_else(
                        || {
                            original.trim().is_empty()
                                || text == original
                                || text.starts_with(&format!(
                                    "{original}\n\nVoice message transcript: "
                                ))
                        },
                        |previous| previous == original,
                    );
                    if !matches {
                        return Err(KokoroError::Validation(
                            "Request identity was reused with different text".into(),
                        ));
                    }
                    request.message = text;
                    request.audio = None;
                }
            }
            if let Some(audio) = request.audio.take() {
                let stt = app
                    .try_state::<crate::stt::SttService>()
                    .ok_or_else(|| KokoroError::Chat("STT service not available".into()))?;
                let transcription = stt
                    .transcribe(
                        &crate::stt::AudioSource::Encoded {
                            data: audio.data,
                            format: audio.format,
                        },
                        None,
                    )
                    .await
                    .map_err(|error| {
                        KokoroError::Chat(format!("Failed to transcribe audio: {error}"))
                    })?;
                if !transcription.text.trim().is_empty() {
                    request.message = if request.message.trim().is_empty() {
                        transcription.text.trim().to_owned()
                    } else {
                        format!(
                            "{}\n\nVoice message transcript: {}",
                            request.message,
                            transcription.text.trim()
                        )
                    };
                }
                if request.message.trim().is_empty() {
                    return Err(KokoroError::Validation("Missing text or media".into()));
                }
            }
        }
        let mut external_turn = None;
        let (
            conversation_id,
            user_message_id,
            history_snapshot,
            bound_generation,
            _completion_generation,
            memory_epoch,
            regeneration,
        ) = 'bind: {
            let _switch_guard = state.selection.conversation_switch_lock.lock().await;
            memory_scope.ensure_owner_current()?;
            let current_conv_id = (!target.create).then(|| identity.conversation_id.clone());
            let current_generation = initial_generation;
            if let ChatPolicy::External {
                create_conversation,
                ..
            } = policy
            {
                drop(_switch_guard);
                let cid = requested_conversation_id.as_deref().ok_or_else(|| {
                    KokoroError::Validation("External conversation is missing".into())
                })?;
                if !create_conversation {
                    validate_conversation_character_id(&state.db, cid, &char_id).await?;
                }
                let mut metadata = serde_json::json!({"source":policy.source(), "client_request_id":client_request_id, "external_request_fingerprint":fingerprint});
                if let Some(original) = original_input {
                    metadata["external_input_text"] = original.into();
                }
                let turn = persistence::ExternalTurn::begin_with_input_metadata(
                    state,
                    target.clone(),
                    client_request_id.clone(),
                    &request.message,
                    Some(metadata),
                    Some(memory_scope.clone()),
                )
                .await?;
                let cid = turn.target.conversation_id.clone();
                let mid = turn.user_message_id;
                let history = turn.history.clone();
                if let Ok(mut guard) = active_conv_id_for_prep.lock() {
                    *guard = Some(cid.clone());
                }
                let _ = policy.emit(app, "chat-turn-bound", serde_json::json!({
                    "client_request_id":client_request_id,"turn_id":assistant_turn_id,"character_id":char_id,
                    "conversation_id":cid,"user_message_id":mid,"message":request.message,
                })).await;
                external_turn = Some(turn);
                break 'bind (
                    Some(cid),
                    Some(mid),
                    history,
                    current_generation,
                    current_generation,
                    state.external_memory.epoch(),
                    None,
                );
            }

            if let Some(conversation_id) = current_conv_id.as_deref() {
                validate_conversation_character_id(&state.db, conversation_id, &char_id).await?;
            }
            if let Ok(mut recorded) = active_conv_id_for_prep.lock() {
                *recorded = Some(identity.conversation_id.clone());
            }
            let bound_generation = initial_generation;

            let regeneration = match (&request.retry_failure, &request.regenerate_reply) {
                (Some(retry), None) => Some(Regeneration::Failure(retry)),
                (None, Some(reply)) => Some(Regeneration::Reply(reply)),
                (None, None) => None,
                (Some(_), Some(_)) => {
                    return Err(KokoroError::Validation(
                        "Conflicting regeneration targets".into(),
                    ))
                }
            };
            if let Some(regeneration) = regeneration {
                let conversation = current_conv_id
                    .as_deref()
                    .filter(|id| requested_conversation_id.as_deref() == Some(*id))
                    .filter(|_| {
                        request.conversation_generation == Some(current_generation)
                            && request.regenerate
                            && !request.hidden
                    })
                    .ok_or_else(|| KokoroError::Validation("Invalid regeneration scope".into()))?;
                let runtime = app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>();
                let plan = RegenerationPlan::prepare(
                    state,
                    &runtime,
                    regeneration,
                    &char_id,
                    conversation,
                    &request.message,
                )
                .await?;
                let history = plan.prompt_history(&request.message);
                break 'bind (
                    current_conv_id,
                    Some(plan.user_message_id),
                    history,
                    bound_generation,
                    current_generation,
                    state.external_memory.epoch(),
                    Some(plan),
                );
            }

            let trailing_visible_user = if request.regenerate {
                if let Some(cid) = current_conv_id.as_deref() {
                    match resolve_trailing_visible_user_message(&state.db, cid).await {
                        Ok(res) => res,
                        Err(e) => {
                            tracing::warn!(
                                "[stream_chat] Failed to query trailing visible message for '{}': {}",
                                cid,
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            let has_trailing_user = trailing_visible_user.is_some();
            let should_insert = should_insert_user_message_for_request(
                request.hidden,
                request.regenerate,
                has_trailing_user,
            );

            // A reserved conversation belongs to its admitted empty view until
            // this turn actually selects the newly committed conversation.
            let mut projection_generation = initial_generation;
            let (cid, mid) = if should_insert {
                let already_saved: bool = sqlx::query_scalar(
                    "SELECT EXISTS (
                         SELECT 1 FROM conversation_messages
                         WHERE role = 'user' AND metadata IS NOT NULL
                         AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.client_request_id') END = ?
                     )",
                )
                .bind(&client_request_id)
                .fetch_one(&state.db)
                .await
                .map_err(KokoroError::chat_persistence)?;
                if already_saved {
                    return Err(KokoroError::Validation(
                        "This request already has a saved user message.".into(),
                    ));
                }
                let create = current_conv_id.is_none();
                let (target, messages, original_revision) =
                    if let Some(pending) = &pending_user_save {
                        (
                            pending.target.clone(),
                            pending.messages.clone(),
                            pending.expected_revision,
                        )
                    } else {
                        let target = ConversationTarget {
                            require_character: true,
                            character_id: char_id.clone(),
                            conversation_id: identity.conversation_id.clone(),
                            create,
                        };
                        #[cfg(debug_assertions)]
                        if create && lifecycle_hold.as_deref() == Some("after-selection") {
                            std::future::pending::<()>().await;
                        }
                        let mut messages = Vec::with_capacity(2);
                        if let Some(observation) = selected_vision_observation.as_ref() {
                            messages.push(vision_context_write(observation, &assistant_turn_id));
                        }
                        messages.push(
                            MessageWrite::new(
                                "user",
                                request.message.clone(),
                                Some(
                                    serde_json::json!({ "client_request_id": client_request_id })
                                        .to_string(),
                                ),
                            )
                            .with_save_id(format!("{client_request_id}:user")),
                        );
                        let revision = if create {
                            Some(0)
                        } else {
                            persistence::current_revision(&state.db, &target)
                                .await
                                .ok()
                                .flatten()
                        };
                        (target, messages, revision)
                    };
                let select_on_commit = create
                    && state.current_conversation_generation() == initial_generation
                    && state.get_character_id().await == char_id
                    && state
                        .selection
                        .current_conversation_id
                        .lock()
                        .await
                        .is_none();
                let saved = state
                    .save_messages_expected_locked(
                        &target,
                        &messages,
                        select_on_commit,
                        None,
                        pending_user_save.as_ref().and(original_revision),
                    )
                    .await;
                if matches!(saved, Err(KokoroError::ChatPersistence(_))) {
                    let mut request = request.clone();
                    request.conversation_id = current_conv_id;
                    request.conversation_generation = Some(current_generation);
                    let runtime = app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>();
                    runtime.prepare_user_save_retry(
                        &client_request_id,
                        original_revision.map(|revision| PendingUserSave {
                            request,
                            target: target.clone(),
                            messages,
                            expected_revision: Some(revision),
                            turn_id: assistant_turn_id.clone(),
                            vision: selected_vision_observation.clone(),
                        }),
                    );
                }
                let saved = saved?;
                if select_on_commit {
                    projection_generation = state.current_conversation_generation();
                }
                if saved.iter().any(|row| row.changed) {
                    state
                        .schedule_conversation_summary(
                            &target,
                            &client_request_id,
                            system_provider.clone(),
                            prompt_snapshot.response_language.clone(),
                            Some(memory_scope.clone()),
                        )
                        .await;
                }
                (
                    Some(target.conversation_id),
                    saved.last().map(|message| message.id),
                )
            } else {
                let cid = current_conv_id;
                let mid = resolve_turn_user_message_id(
                    request.hidden,
                    trailing_visible_user.map(|(id, _)| id),
                );

                (cid, mid)
            };

            // Bind persisted identity before releasing the same lock used by
            // get_chat_view. A subscriber joining during prompt preparation must
            // not project a second optimistic user beside this SQLite row.
            let _ = policy
                .emit(
                    app,
                    "chat-turn-bound",
                    serde_json::json!({
                        "client_request_id": client_request_id,
                        "turn_id": assistant_turn_id,
                        "character_id": char_id,
                        "conversation_id": identity.conversation_id,
                        "conversation_generation": projection_generation,
                        "user_message_id": mid,
                    }),
                )
                .await;

            let history_snapshot = if cid.is_some() {
                state.saved_history_for(&target, None).await?
            } else {
                Vec::new()
            };

            (
                cid,
                mid,
                history_snapshot,
                bound_generation,
                state.current_conversation_generation(),
                state.external_memory.epoch(),
                None,
            )
        };

        if let Ok(mut guard) = active_conv_id_for_prep.lock() {
            *guard = Some(identity.conversation_id.clone());
        }

        if let Some(hooks) = hook_runtime.as_ref() {
            let hook_payload = build_chat_hook_payload(
                Some(identity.conversation_id.clone()),
                &char_id,
                None,
                Some(request.message.clone()),
                None,
                None,
                request.hidden,
            );
            let hook_fut = hooks.emit_best_effort(&HookEvent::BeforeUserMessage, &hook_payload);
            tokio::select! {
                _ = tokio::time::sleep(chat_hook_execution_timeout()) => {
                    tracing::warn!(
                        target: "chat",
                        "[stream_chat] BeforeUserMessage hook timed out after {}s",
                        chat_hook_execution_timeout().as_secs()
                    );
                }
                _ = hook_fut => {}
            }
        }

        if let Some(hooks) = hook_runtime
            .as_ref()
            .filter(|_| user_message_id.is_some() && regeneration.is_none())
        {
            let hook_payload = build_chat_hook_payload(
                Some(identity.conversation_id.clone()),
                &char_id,
                None,
                Some(request.message.clone()),
                None,
                None,
                request.hidden,
            );
            let hook_fut =
                hooks.emit_best_effort(&HookEvent::AfterUserMessagePersisted, &hook_payload);
            tokio::select! {
                _ = tokio::time::sleep(chat_hook_execution_timeout()) => {
                    tracing::warn!(
                        target: "chat",
                        "[stream_chat] AfterUserMessagePersisted hook timed out after {}s",
                        chat_hook_execution_timeout().as_secs()
                    );
                }
                _ = hook_fut => {}
            }
        }

        // ── LAYER 1 & 2: SYSTEM SETUP ───────────────────────────────

        // ── EXECUTION & STATE UPDATE ────────────────────────────────

        // ── LAYER 3: PERSONA GENERATION ─────────────────────────────

        let effective_provider_id = chat_provider.id().to_string();
        let native_tools_enabled = llm_config
            .providers
            .iter()
            .find(|provider| provider.id == effective_provider_id)
            .map(|provider| provider.supports_native_tools)
            .unwrap_or_else(|| chat_provider.supports_native_tools());
        tracing::info!(
            target: "chat",
            "[Chat] configured_active_provider={}, effective_active_provider={}, native_tools_enabled={}",
            llm_config.active_provider, effective_provider_id, native_tools_enabled
        );
        let vision_config = vision_watcher.config.read().await.clone();
        let vision_enabled = desktop && vision_config.vlm_enabled;

        let (tool_prompt, native_tools) = {
            let registry = action_registry.read().await;
            let settings = tool_settings_state.read().await;
            let actions = registry
                .list_actions_for_prompt_with_settings_and_availability(
                    state.is_memory_enabled(),
                    vision_enabled,
                    &settings,
                )
                .into_iter()
                .filter(|action| policy.allows_tool(&action.id))
                .collect::<Vec<_>>();
            let prompt = if native_tools_enabled {
                String::new()
            } else {
                registry.generate_tool_prompt_from_actions(actions.clone())
            };
            let definitions = ActionRegistry::tool_definitions(actions);
            ((!prompt.is_empty()).then_some(prompt), definitions)
        };
        let memory_target_language = prompt_snapshot.response_language.clone();

        if cancel_state.is_cancelled(&assistant_turn_id).await {
            tracing::info!(
                target: "chat",
                "[stream_chat] Turn {} (request {}) cancelled before prompt composition",
                assistant_turn_id,
                client_request_id
            );
            return Err(KokoroError::Chat(
                TURN_CANCELLED_BY_USER_MESSAGE.to_string(),
            ));
        }

        // Compose Persona Prompt with immutable history snapshot and explicit conversation scoping
        let (prompt_messages, compose_warnings) = if external_turn
            .as_ref()
            .is_some_and(|turn| turn.previous_reply.is_some())
        {
            (Vec::new(), Vec::new())
        } else {
            state
                .compose_chat_prompt(
                    &request.message,
                    request.allow_image_gen.unwrap_or(false),
                    tool_prompt,
                    native_tools_enabled,
                    &char_id,
                    Some(&identity.conversation_id),
                    Some(history_snapshot),
                    desktop,
                    &prompt_snapshot,
                )
                .await
                .map_err(|e| KokoroError::Chat(e.to_string()))?
        };

        // 将构建过程中产生的非致命警告（如记忆检索失败）通知前端
        for warning in compose_warnings {
            tracing::warn!("[compose_prompt] {}", warning);
            let _ = policy.emit(app, "chat-warning", serde_json::json!({"message":warning,"client_request_id":client_request_id,"turn_id":assistant_turn_id,"character_id":identity.character_id,"conversation_id":identity.conversation_id,"conversation_generation":request.conversation_generation})).await;
        }

        Ok(PreparedChatTurn {
            prompt_snapshot,
            speech,
            speech_snapshot,
            char_id,
            conversation_id,
            user_message_id,
            bound_generation,
            memory_epoch,
            regeneration,
            selected_vision_observation,
            prompt_messages,
            llm_config,
            chat_provider,
            effective_provider_id,
            native_tools_enabled,
            native_tools,
            system_provider,
            memory_target_language,
            external_turn,
        })
    };

    // One budget covers snapshot acquisition and the rest of preparation.
    let prepared_turn = tokio::select! {
        biased;
        _ = wait_for_cancel_event(&mut cancel_rx) => {
            tracing::info!(
                target: "chat",
                "[stream_chat] Turn {} (request {}) cancelled during preparation phase",
                assistant_turn_id,
                client_request_id
            );
            let recorded_conv_id = active_conversation_id_holder
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let _ = policy.emit(app,
                "chat-turn-finish",
                serde_json::json!({
                    "turn_id": assistant_turn_id,
                    "status": "cancelled",
                    "client_request_id": client_request_id,
                    "conversation_id": recorded_conv_id,
                    "assistant_message_id": serde_json::Value::Null,
                }),
            ).await;
            return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
        }
        _ = tokio::time::sleep(preparation_timeout) => {
            tracing::error!(
                target: "chat",
                "[stream_chat] Turn {} (request {}) timed out during preparation phase after {}s",
                assistant_turn_id,
                client_request_id,
                preparation_timeout.as_secs()
            );
            let recorded_conv_id = active_conversation_id_holder
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let _ = policy.emit(app,
                "chat-turn-finish",
                serde_json::json!({
                    "turn_id": assistant_turn_id,
                    "status": "error",
                    "stage": "preparation_timeout",
                    "client_request_id": client_request_id,
                    "conversation_id": recorded_conv_id,
                    "assistant_message_id": serde_json::Value::Null,
                }),
            ).await;
            return Err(KokoroError::Chat(format!(
                "Turn preparation timed out after {}s",
                preparation_timeout.as_secs()
            )));
        }
        res = prep_fut => {
            if matches!(res, Err(KokoroError::ChatPersistence(_))) {
                // The explicit retry will register this same turn ID. Finish
                // removing the old cancellation registration before publishing
                // the save failure and letting the retry reach admission.
                turn_guard.clear_before_retry().await;
            }
            res?
        }
    };

    let PreparedChatTurn {
        prompt_snapshot,
        speech,
        speech_snapshot,
        char_id,
        mut conversation_id,
        user_message_id,
        bound_generation,
        memory_epoch,
        regeneration,
        selected_vision_observation,
        prompt_messages,
        llm_config,
        chat_provider,
        effective_provider_id,
        native_tools_enabled,
        native_tools,
        system_provider,
        memory_target_language,
        external_turn,
    } = prepared_turn;

    let cue_names = prompt_snapshot
        .cue_map
        .as_ref()
        .map(|cues| Arc::new(cues.keys().cloned().collect::<HashSet<_>>()));

    if let Some(turn) = external_turn.as_ref() {
        if let Some((text, translation)) = &turn.previous_reply {
            let displayed_text = strip_leaked_tags(text);
            if let Some(execution) = &turn.previous_execution {
                super::follow_up::schedule_external_memory_tasks(
                    policy.source(),
                    &request.message,
                    &char_id,
                    turn,
                    state,
                    execution.memory_provider.clone(),
                    &execution.memory_language,
                    turn.previous_memory_work,
                    execution.memory_scope.clone(),
                )
                .await;
            }
            let _ = policy.emit(app, "chat-turn-text-complete", serde_json::json!({"turn_id":assistant_turn_id,"text":displayed_text,"translation":translation})).await;
            let assistant_message_id = sqlx::query_scalar("SELECT id FROM conversation_messages WHERE conversation_id=? AND role='assistant' AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.chat_save_id') END=?")
                .bind(&turn.target.conversation_id).bind(format!("{client_request_id}:assistant")).fetch_optional(&state.db).await.map_err(KokoroError::chat_persistence)?;
            return Ok(StreamChatResponse {
                speech_snapshot: turn
                    .previous_execution
                    .as_ref()
                    .and_then(|execution| execution.speech.clone()),
                text: displayed_text,
                translation: translation.clone(),
                conversation_id: turn.target.conversation_id.clone(),
                user_message_id,
                assistant_message_id,
                client_request_id: Some(client_request_id),
                status: Some("completed".into()),
                output_warnings: turn.previous_reply_media.output_warnings.clone(),
                external_media: turn.previous_reply_media.clone(),
                newly_committed: turn.previous_reply_newly_committed,
                ..Default::default()
            });
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (llm_config, effective_provider_id);

    let mut legacy_image_prompts = HashSet::new();
    let mut media_delivery_failures = HashSet::new();
    let mut images = Vec::new();
    let mut output_warnings = Vec::new();
    let mut final_translation = None;
    let mut external_media = persistence::ExternalReplyMedia::default();
    let is_newly_created_for_hidden = false;
    let mut pending_tool_messages = Vec::new();
    let draft_row_id_holder = std::sync::Arc::new(tokio::sync::Mutex::new(None));
    let draft_row_id_for_stream = std::sync::Arc::clone(&draft_row_id_holder);

    let stream_result: Result<Option<i64>, KokoroError> = async {
    let mut before_llm_request_payload = build_before_llm_request_payload(
        conversation_id.clone(),
        &char_id,
        Some(assistant_turn_id.clone()),
        request.message.clone(),
        request.hidden,
        &prompt_messages,
    );

    if let Some(hooks) = hook_runtime.as_ref() {
        let hook_fut = hooks.emit_before_llm_request_modify(
            &mut before_llm_request_payload,
            HookModifyPolicy::Strict,
        );
        tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut cancel_rx) => {
                tracing::info!(
                    target: "chat",
                    "[stream_chat] Turn {} (request {}) cancelled during BeforeLlmRequest modify hook",
                    assistant_turn_id,
                    client_request_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
            _ = tokio::time::sleep(chat_hook_execution_timeout()) => {
                tracing::error!(
                    target: "chat",
                    "[stream_chat] BeforeLlmRequest modify hook timed out after {}s",
                    chat_hook_execution_timeout().as_secs()
                );
                return Err(KokoroError::Chat(format!(
                    "BeforeLlmRequest hook timed out after {}s",
                    chat_hook_execution_timeout().as_secs()
                )));
            }
            res = hook_fut => {
                res.map_err(KokoroError::Chat)?;
            }
        }
    }

    let (effective_request_message, mut client_messages) =
        apply_before_llm_request_payload(before_llm_request_payload, &prompt_messages)
            .map_err(KokoroError::Chat)?;

    if let Some(hooks) = hook_runtime.as_ref() {
        let hook_payload = build_chat_hook_payload(
            Some(identity.conversation_id.clone()),
            &char_id,
            Some(assistant_turn_id.clone()),
            Some(effective_request_message.clone()),
            None,
            None,
            request.hidden,
        );
        let hook_fut = hooks.emit_best_effort(&HookEvent::BeforeLlmRequest, &hook_payload);
        tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut cancel_rx) => {
                tracing::info!(
                    target: "chat",
                    "[stream_chat] Turn {} (request {}) cancelled during BeforeLlmRequest best-effort hook",
                    assistant_turn_id,
                    client_request_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
            _ = tokio::time::sleep(chat_hook_execution_timeout()) => {
                tracing::warn!(
                    target: "chat",
                    "[stream_chat] BeforeLlmRequest best-effort hook timed out after {}s",
                    chat_hook_execution_timeout().as_secs()
                );
            }
            _ = hook_fut => {}
        }
    }

    if cancel_state.is_cancelled(&assistant_turn_id).await {
        tracing::info!(
            target: "chat",
            "[stream_chat] Turn {} (request {}) cancelled before chat-turn-start emit",
            assistant_turn_id,
            client_request_id
        );
        return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
    }

    let keep_through_message_id = regeneration.as_ref()
        .filter(|plan| plan.keep_through_message_id != plan.user_message_id)
        .map(|plan| plan.keep_through_message_id);
    if let Some(regeneration) = regeneration {
        let conversation = conversation_id.as_deref().ok_or_else(|| KokoroError::Validation("Missing regeneration conversation".into()))?;
        regeneration.commit(state, conversation, bound_generation, cancel_rx.as_ref()).await?;
    }

    policy.emit(app,
        "chat-turn-start",
        serde_json::json!({
            "turn_id": assistant_turn_id,
            "client_request_id": request.client_request_id,
            "conversation_id": identity.conversation_id,
            "user_message_id": user_message_id,
        }),
    ).await
    .map_err(|e| KokoroError::Chat(e.to_string()))?;

    // For hidden messages (touch/proactive interactions), the user message
    // wasn't added to history, so include it before dedicated vision rendering.
    // This keeps screen context immediately before the current hidden user turn.
    if request.hidden {
        client_messages.push(plain_llm_message(user_text_message(
            effective_request_message.clone(),
        )));
    }

    // Dedicated current-turn vision context rendering happens after ordinary
    // request hooks. Visible turns already persisted the selected observation
    // before prompt composition, so only hidden turns need an in-flight insert.
    if request.hidden {
        if let Some(observation) = selected_vision_observation.as_ref() {
            insert_vision_context_before_latest_user(&mut client_messages, observation);
        }
    }

    // Attach the selected images and one-shot camera context to the current user message.
    let input_images = request.images.as_deref().unwrap_or_default();
    let camera_frame = request.camera_frame.take().filter(|_| desktop);
    let image_count = input_images.len() + usize::from(camera_frame.is_some());
    if image_count > 0 {
        if let Some(last_user_msg) = client_messages
            .iter_mut()
            .rfind(|m| crate::llm::messages::is_user_message(&m.message))
        {
            let text_content = extract_message_text(&last_user_msg.message);
            let mut processed_images = Vec::with_capacity(image_count);

            if !desktop {
                processed_images.extend_from_slice(input_images);
            } else if !input_images.is_empty() {
                let vision_server_guard = vision_server.lock().await;
                let port = vision_server_guard.port;
                let upload_dir = vision_server_guard.upload_dir.clone();
                drop(vision_server_guard);
                for img_url in input_images {
                    let mut final_url = img_url.clone();
                    if img_url.contains(&format!("http://127.0.0.1:{}", port)) {
                        if let Some(filename) = img_url.split("/vision/").nth(1) {
                            let file_path = upload_dir.join(filename);
                            if let Ok(file_content) = tokio::fs::read(&file_path).await {
                                use base64::Engine as _;
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(&file_content);
                                let mime = crate::vision::server::detect_image_mime(&file_content)
                                    .unwrap_or("image/png".to_string());
                                final_url = format!("data:{};base64,{}", mime, b64);
                            }
                        }
                    }
                    processed_images.push(final_url);
                }
            }

            processed_images.extend(camera_frame);
            replace_user_message_with_images(
                &mut last_user_msg.message,
                text_content,
                processed_images,
            )
            .map_err(KokoroError::Chat)?;
            tracing::info!(target: "chat", "[Chat] Attached {} images to user message", image_count);
        }
    }

    #[cfg(debug_assertions)]
    {
        tracing::info!(
            target: "llm",
            "[LLM/Debug] configured_active_provider={} effective_active_provider={} native_tools_enabled={} tool_count={}",
            llm_config.active_provider,
            effective_provider_id,
            native_tools_enabled,
            native_tools.len()
        );
        debug_log_rich_llm_messages("initial chat request", &client_messages);
    }

    // Stream Response with Tool Call Feedback Loop
    let max_tool_rounds = {
        let tool_settings = tool_settings_state.read().await;
        tool_settings.max_tool_rounds.max(1)
    };
    let mut all_cleaned_text = String::new();
    let mut executed_any_tool = false;
    // Tools can change state even when the handler eventually returns an error.
    let mut dispatched_tools = false;
    let mut all_translations = Vec::new();
    let mut bg_generated_by_tool = false;
    let mut cue_set_by_tool = false;
    let mut draft_row_id: Option<i64> = None;
    let mut stream_failed = false;
    let mut stream_timed_out = false;
    let mut stream_failure_message: Option<String> = None;
    let mut all_reasoning_content = String::new();
    let mut final_provider_data = Vec::new();

    let mut cancel_rx = cancel_state.subscribe_cancellation(&assistant_turn_id).await;

    for round in 0..max_tool_rounds {
        memory_scope.ensure_owner_current()?;
        tracing::info!(target: "chat", "[Chat] Tool loop round {}", round + 1);
        ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
            .await
            .map_err(KokoroError::Chat)?;

        let round_output = super::model_round::run(
            chat_provider.as_ref(),
            client_messages.clone(),
            native_tools_enabled.then_some(native_tools.as_slice()),
            &mut cancel_rx,
            |text| async {
                let payload = build_turn_delta_payload_if_not_cancelled(
                    cancel_state.as_ref(), &assistant_turn_id, text, request.client_request_id.as_deref(),
                ).await.map_err(KokoroError::Chat)?;
                policy.emit(app, "chat-turn-delta", payload).await.map_err(|error| KokoroError::Chat(error.to_string()))
            },
        ).await?;
        memory_scope.ensure_owner_current()?;
        let super::model_round::RoundOutput {
            text: round_response,
            reasoning: round_reasoning_content,
            provider_data: round_provider_data,
            native_tool_calls,
            remainder: emit_buffer,
            failure,
        } = round_output;
        if let Some(failure) = failure {
            let retryable = !dispatched_tools;
            let failure_event = build_chat_error_event(failure.stage, &failure.message, &assistant_turn_id, retryable)
                .into_failure_event(Some(identity.conversation_id.clone()), Some(assistant_turn_id.clone()), Some(char_id.clone()), None);
            emit_and_persist_failure_event(
                app, policy, state, failure_event,
                user_message_id, keep_through_message_id, &memory_scope,
            ).await?;
            stream_failure_message = Some(failure.message);
            stream_timed_out = failure.timed_out;
            stream_failed = true;
        }

        // If stream failed or timed out, terminate round and outer loop immediately.
        // Never execute any tool calls that arrived before or during the failure.
        if stream_failed || stream_timed_out {
            tracing::warn!(
                target: "chat",
                "[Chat] Terminating round {} and turn {} due to stream failure/timeout (timed_out={}); suppressing {} native tool calls",
                round + 1,
                assistant_turn_id,
                stream_timed_out,
                native_tool_calls.len()
            );
            if !round_response.is_empty() {
                let (cleaned_text, _) = parse_tool_call_tags(&round_response);
                let (cleaned_text, _) = extract_translate_tags(&cleaned_text);
                merge_continuation_text(&mut all_cleaned_text, &cleaned_text);
                if desktop && !request.hidden && !all_cleaned_text.is_empty() && !cancel_state.is_cancelled(&assistant_turn_id).await {
                    let draft_content = strip_leaked_tags(&all_cleaned_text);
                    if !draft_content.is_empty() {
                        if let Some(cid) = &conversation_id {
                            draft_row_id = save_visible_draft(
                                state, &char_id, cid, &assistant_turn_id, draft_row_id, &draft_content, &memory_scope,
                            ).await;
                            *draft_row_id_for_stream.lock().await = draft_row_id;
                        }
                    }
                }
            }
            break;
        }

        // Flush remaining buffer — strip any complete tags before emitting
        if !emit_buffer.is_empty() {
            let (cleaned_remainder, _) = parse_tool_call_tags(&emit_buffer);
            let cleaned_remainder = strip_translate_tags(&cleaned_remainder);
            if !cleaned_remainder.is_empty() {
                let payload = build_turn_delta_payload_if_not_cancelled(
                    cancel_state.as_ref(),
                    &assistant_turn_id,
                    cleaned_remainder,
                    request.client_request_id.as_deref(),
                )
                .await
                .map_err(KokoroError::Chat)?;
                policy.emit(app, "chat-turn-delta", payload).await
                    .map_err(|e| KokoroError::Chat(e.to_string()))?;
            }
        }

        let (cleaned_text, mut parsed_tool_calls) = parse_tool_call_tags(&round_response);
        let (cleaned_text, round_translation) = extract_translate_tags(&cleaned_text);
        let cleaned_text = if desktop {
            cleaned_text
        } else {
            let (cleaned, prompts) = crate::chat::tags::extract_image_prompt_tags(&cleaned_text);
            if matches!(policy, ChatPolicy::External { image_output: true, .. }) {
                parsed_tool_calls.extend(prompts.into_iter()
                    .filter(|prompt| legacy_image_prompts.insert(prompt.clone()))
                    .map(|prompt| ToolCall {
                        tool_call_id: None,
                        name: builtin_tool_id("generate_image"),
                        args: HashMap::from([("prompt".into(), prompt)]),
                    }));
            } else if !prompts.is_empty() {
                output_warnings.push("Image output is unavailable on this channel.".to_string());
            }
            crate::chat::tags::strip_external_control_tags(&cleaned)
        };
        let (tool_calls, deduped_textual_tool_call_count) =
            merge_round_tool_calls(parsed_tool_calls, native_tool_calls);

        tracing::info!(
            target: "chat",
            "[Chat] Round {} raw response ({} chars): ...{}",
            round + 1,
            round_response.len(),
            round_response
                .chars()
                .rev()
                .take(100)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        tracing::info!(
            target: "chat",
            "[Chat] Round {} translation: {:?}",
            round + 1,
            round_translation
        );
        tracing::info!(
            target: "chat::tools",
            "[Chat] Round {} tool_calls: {}",
            round + 1,
            tool_calls.len()
        );
        if deduped_textual_tool_call_count > 0 {
            tracing::warn!(
                target: "chat::tools",
                "[Chat] Round {} dropped {} duplicate textual tool call(s) because matching native tool call(s) were present",
                round + 1,
                deduped_textual_tool_call_count
            );
        }

        // Collect translation from this round
        if let Some(t) = round_translation {
            all_translations.push(t);
        }

        // Accumulate cleaned text for history
        merge_continuation_text(&mut all_cleaned_text, &cleaned_text);
        all_reasoning_content.push_str(&round_reasoning_content);

        // Persist visible assistant drafts incrementally. Hidden proactive/touch
        // turns are persisted only after final no-op filtering, so PASS/empty
        // proactive vision responses leave no DB rows.
        if desktop && !request.hidden && !all_cleaned_text.is_empty() && !cancel_state.is_cancelled(&assistant_turn_id).await {
            let draft_content = strip_leaked_tags(&all_cleaned_text);
            if !draft_content.is_empty() {
                if let Some(cid) = &conversation_id {
                    draft_row_id = save_visible_draft(
                        state, &char_id, cid, &assistant_turn_id, draft_row_id, &draft_content, &memory_scope,
                    ).await;
                    *draft_row_id_for_stream.lock().await = draft_row_id;
                }
            }
        }

        // No tool calls → final round
        if tool_calls.is_empty() {
            final_provider_data = round_provider_data;
            break;
        }

        // Execute tool calls and collect results
        let tool_invocations = {
            let registry = action_registry.read().await;
            tool_calls
                .iter()
                .map(|tool_call| {
                    crate::actions::executor::build_tool_invocation_from_input(
                        &registry,
                        &tool_call.name,
                        tool_call.args.clone(),
                        tool_call.tool_call_id.clone(),
                    )
                    .map_err(|error| KokoroError::Validation(error.0))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
            .await
            .map_err(KokoroError::Chat)?;
        let allowed_tool_ids = (!desktop).then(|| native_tools.iter().map(|tool| tool.name.clone()).collect());
        let tool_scope = ToolExecutionScope {
            memory_scope: Some(&memory_scope),
            cue_names: cue_names.as_ref(),
            request_id: &client_request_id,
            turn_id: &assistant_turn_id,
            character_id: &char_id,
            conversation_id: Some(&identity.conversation_id),
            source: policy.source(),
            allow_hooks: desktop,
            allowed_tool_ids: allowed_tool_ids.as_ref(),
        };
        dispatched_tools = true;
        let execution_outcomes = match execute_tool_calls_with_cancellation(
            app,
            action_registry,
            tool_settings_state,
            &tool_scope,
            &tool_invocations,
            cancel_rx.clone(),
            Some(tool_execution_timeout()),
        )
        .await
        {
            Ok(outcomes) => outcomes,
            Err(ToolCancellationError) => {
                tracing::info!(
                    target: "chat::tools",
                    "[ToolCall] Tool execution cancelled by user for turn {}",
                    assistant_turn_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
        };
        ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
            .await
            .map_err(KokoroError::Chat)?;
        let mut tool_results = Vec::new();
        let mut tool_result_messages = Vec::new();
        let mut continuation_tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut continuation_tool_call_messages = Vec::new();
        let mut persisted_native_tool_results: Vec<(
            serde_json::Value,
            async_openai::types::chat::ChatCompletionRequestMessage,
        )> = Vec::new();
        let any_needs_feedback = execution_outcomes
            .iter()
            .any(|outcome| outcome.needs_feedback);
        let has_native_tool_calls = tool_calls.iter().any(|tc| tc.tool_call_id.is_some());

        executed_any_tool |= execution_outcomes.iter().any(|outcome| action_result_is_success(&outcome.result));
        for (index, outcome) in execution_outcomes.into_iter().enumerate() {
            // Scope the trace to this turn. Providers can omit or reuse their call IDs.
            let tool_trace_id = format!("{round}:{index}");
            ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
                .await
                .map_err(KokoroError::Chat)?;
            tracing::info!(
                target: "tools",
                "[ToolCall] Executing: {} with args {:?}",
                outcome.invocation.name, outcome.invocation.args
            );
            if outcome.tool_id() == builtin_tool_id("set_background") {
                bg_generated_by_tool = true;
            }
            if outcome.tool_id() == builtin_tool_id("play_cue") {
                cue_set_by_tool = true;
            }

            let audit_event = build_tool_audit_event(ToolAuditInput {
                tool_id: outcome.tool_id(),
                tool_name: outcome.tool_name(),
                source: outcome.tool_source().unwrap_or("builtin"),
                server_name: outcome.tool_server_name(),
                invocation_source: policy.source(),
                risk_tags: &outcome.tool_risk_tags(),
                permission_level: outcome.tool_permission_level().unwrap_or("safe"),
                decision: outcome
                    .permission_decision
                    .as_ref()
                    .unwrap_or(&PermissionDecision::Allow),
                approved_by_user: None,
                conversation_id: Some(&identity.conversation_id),
                character_id: Some(&char_id),
            });
            tracing::info!(target: "tools", "[ToolAudit] {:?}", audit_event);

            let result = if let Err(error) = &outcome.result {
                if desktop && matches!(
                    outcome.permission_decision,
                    Some(PermissionDecision::DenyPendingApproval { .. })
                ) {
                    let approval_ctx = ToolApprovalExecutionContext {
                        memory_scope: &memory_scope,
                        cue_names: cue_names.as_ref(),
                        request_id: &client_request_id,
                        app,
                        approval_state: approval_state.as_ref(),
                        registry_state: action_registry,
                        tool_settings_state,
                        character_id: &char_id,
                        conversation_id: Some(&identity.conversation_id),
                        turn_id: &assistant_turn_id,
                        tool_trace_id: &tool_trace_id,
                        cancel_state: cancel_state.as_ref(),
                    };
                    let (resolved_result, resolved_payload) = wait_for_tool_approval_and_execute(
                        &approval_ctx,
                        &outcome,
                        error,
                    )
                    .await?;
                    match &resolved_result {
                        Ok(result) => {
                            tracing::info!(target: "tools", "[ToolCall] {} approved => {}", outcome.tool_name(), result.message);
                        }
                        Err(error) => {
                            tracing::error!(target: "tools", "[ToolCall] {} rejected/failed after approval flow: {}", outcome.tool_name(), error);
                        }
                    }
                    ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
                        .await
                        .map_err(KokoroError::Chat)?;
                    emit_tool_trace_event(app, policy, &tool_trace_id, resolved_payload).await
                        .map_err(|e| KokoroError::Chat(e.to_string()))?;
                    ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
                        .await
                        .map_err(KokoroError::Chat)?;
                    resolved_result
                } else {
                    tracing::error!(target: "tools", "[ToolCall] {} failed: {}", outcome.tool_name(), error);
                    let payload = tool_error_payload(&outcome, &assistant_turn_id, error);
                    let _ = emit_tool_trace_event(app, policy, &tool_trace_id, payload).await;
                    outcome.result.clone()
                }
            } else {
                if let Ok(success) = &outcome.result {
                    tracing::info!(target: "tools", "[ToolCall] {} => {}", outcome.tool_name(), success.message);
                    let payload = tool_success_payload(&outcome, &assistant_turn_id, success);
                    let _ = emit_tool_trace_event(app, policy, &tool_trace_id, payload).await;
                }
                outcome.result.clone()
            };

            ensure_turn_not_cancelled(cancel_state.as_ref(), &assistant_turn_id)
                .await
                .map_err(KokoroError::Chat)?;

            if !desktop && outcome.tool_id() == builtin_tool_id("generate_image") {
                let image = result.as_ref().ok().and_then(|result| result.data.as_ref())
                    .and_then(|data| {
                        Some(ChatImage {
                            path: data.get("image_url")?.as_str()?.to_owned(),
                            prompt: data.get("prompt").and_then(serde_json::Value::as_str)
                                .unwrap_or_default().to_owned(),
                        })
                    });
                if let Some(image) = image {
                    images.push(image);
                } else {
                    media_delivery_failures.insert("Kokoro Engine could not generate the image.");
                }
            }

            tool_results.push(match &result {
                Ok(value) => format!("- {}: {}", outcome.tool_id(), value.message),
                Err(error) => format!("- {}: Error: {}", outcome.tool_id(), error),
            });

            if let Some(tool_call_id) = &outcome.invocation.tool_call_id {
                continuation_tool_calls
                    .push(assistant_tool_call_metadata_value(&outcome, tool_call_id));
                continuation_tool_call_messages.push((
                    tool_call_id.clone(),
                    // Provider-facing history must use the canonical registry id. Keep the
                    // human-readable action name in metadata for UI/audit purposes.
                    outcome.tool_id().to_string(),
                    serde_json::to_string(&outcome.invocation.args)
                        .unwrap_or_else(|_| "{}".to_string()),
                ));
                let message_text = match &result {
                    Ok(result) => result.message.clone(),
                    Err(error) => format!("Error: {}", error),
                };
                let tool_result_msg = tool_result_message(tool_call_id.clone(), message_text);
                tool_result_messages.push(tool_result_msg.clone());
                let mut metadata = tool_metadata_value(&outcome, tool_call_id, &assistant_turn_id);
                metadata["tool_trace_id"] = serde_json::Value::String(tool_trace_id);
                metadata["user_message_id"] = serde_json::json!(user_message_id);
                persisted_native_tool_results.push((metadata, tool_result_msg));
            }
        }

        if has_native_tool_calls {
            let mut assistant_tool_call_metadata_value = serde_json::json!({
                "type": "assistant_tool_calls",
                "turn_id": assistant_turn_id,
                "user_message_id": user_message_id,
                "tool_calls": continuation_tool_calls,
            });
            if !round_reasoning_content.trim().is_empty() {
                assistant_tool_call_metadata_value["reasoning_content"] =
                    serde_json::Value::String(round_reasoning_content.clone());
            }
            if !round_provider_data.is_empty() {
                assistant_tool_call_metadata_value["provider_data"] =
                    serde_json::Value::Array(round_provider_data.clone());
            }
            // Defer hidden conversation creation with the tool exchange until the
            // final transaction. Approvals keep the original selected scope.
            pending_tool_messages.push(MessageWrite::new(
                "assistant", cleaned_text.clone(), Some(assistant_tool_call_metadata_value.to_string()),
            ).with_save_id(format!("{assistant_turn_id}:call:{round}")));
            for (index, (tool_metadata, tool_message)) in persisted_native_tool_results.iter().enumerate() {
                pending_tool_messages.push(MessageWrite::new(
                    "tool", extract_message_text(tool_message), Some(tool_metadata.to_string()),
                ).with_save_id(format!("{assistant_turn_id}:result:{round}:{index}")));
            }
            client_messages.push(LlmChatMessage {
                message: assistant_tool_calls_message(
                    if cleaned_text.is_empty() {
                        None
                    } else {
                        Some(cleaned_text.clone())
                    },
                    continuation_tool_call_messages,
                ),
                reasoning_content: (!round_reasoning_content.trim().is_empty())
                    .then_some(round_reasoning_content.clone()),
                provider_data: round_provider_data,
            });
            client_messages.extend(tool_result_messages.into_iter().map(plain_llm_message));

            tracing::info!(
                target: "chat::tools",
                "[Chat] Continuing after native tool calls with assistant/tool result messages"
            );
            #[cfg(debug_assertions)]
            debug_log_rich_llm_messages(
                &format!("post-tool continuation round {}", round + 1),
                &client_messages,
            );
            continue;
        }

        // Only continue the loop if at least one tool needs its result fed back to the LLM
        if !any_needs_feedback {
            tracing::info!(target: "chat", "[Chat] No feedback-requiring tools, ending loop");
            break;
        }

        // Prompt-mode tools do not have native tool-result messages, so feed a
        // neutral tool-result summary back into the model without forcing a
        // visible reply. The next round may answer, call more tools, or end
        // with empty text.
        client_messages.push(plain_llm_message(system_message(format!(
            "[Tool results]\n{}",
            tool_results.join("\n")
        ))));
        #[cfg(debug_assertions)]
        debug_log_rich_llm_messages(
            &format!("feedback continuation round {}", round + 1),
            &client_messages,
        );
    }

    let mut full_response = strip_leaked_tags(&all_cleaned_text);
    output_warnings.extend(media_delivery_failures.into_iter().map(str::to_owned));
    let mut canonical_response = strip_leaked_tool_results(&all_cleaned_text);

    if stream_failed {
        let mut finish_draft_row_id = draft_row_id;
        if full_response.is_empty() {
            if let Some(ref cid) = conversation_id {
                if is_newly_created_for_hidden {
                    match delete_empty_conversation_if_unused(
                        state,
                        cid,
                        Some(&assistant_turn_id),
                        draft_row_id,
                    )
                    .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::info!(
                                target: "chat",
                                "[Chat] Preserved temporary conversation '{}' on stream failure because it contains other messages",
                                cid
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                target: "chat",
                                "[Chat] Failed to clean up temporary conversation '{}' on stream failure: {}",
                                cid,
                                e
                            );
                        }
                    }
                } else {
                    cleanup_turn_artifacts(state, cid, &assistant_turn_id, draft_row_id).await;
                }
            }
            finish_draft_row_id = None;
        } else {
            // Partial response exists: clean up any technical tool rows from earlier rounds
            if let Some(ref cid) = conversation_id {
                cleanup_turn_artifacts(state, cid, &assistant_turn_id, None).await;
            }
            // Flush typewriter/reveal buffer in frontend
            let _ = policy.emit(app,
                "chat-turn-text-complete",
                serde_json::json!({
                    "turn_id": assistant_turn_id,
                    "text": full_response.clone(),
                    "translation_pending": false,
                    "translation": serde_json::Value::Null,
                }),
            ).await;
        }

        policy.emit(app,
            "chat-turn-finish",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "status": "error",
                "client_request_id": request.client_request_id,
                "conversation_id": identity.conversation_id,
                "assistant_message_id": finish_draft_row_id,
            }),
        ).await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;

        let err_msg = stream_failure_message
            .unwrap_or_else(|| "Chat stream failed or timed out".to_string());
        return Err(KokoroError::Llm(err_msg));
    }

    if !desktop && full_response.is_empty() && images.is_empty() && output_warnings.is_empty() {
        if !executed_any_tool {
            return Err(KokoroError::Llm("No response from AI".into()));
        }
        // Tool results may contain internal data intended only for the model.
        // Persist a deliverable completion notice without disclosing or rerunning them.
        full_response = tool_only_completion_notice(&memory_target_language)?;
        canonical_response = full_response.clone();
    }
    if request.hidden && is_proactive_noop_response(&full_response) {
        if let Some(row_id) = draft_row_id {
            *draft_row_id_for_stream.lock().await = None;
            if let Err(error) = sqlx::query("DELETE FROM conversation_messages WHERE id = ?").bind(row_id).execute(&state.db).await {
                tracing::error!(
                    target: "chat",
                    "[Chat] Failed to delete hidden proactive no-op draft: {}",
                    error
                );
            }
            draft_row_id = None;
        }
        // A hidden no-op turn may still have persisted assistant_tool_calls/tool rows
        // from earlier tool rounds; remove them so no orphan tool exchange survives.
        if let Some(ref cid) = conversation_id {
            if is_newly_created_for_hidden {
                match delete_empty_conversation_if_unused(
                    state,
                    cid,
                    Some(&assistant_turn_id),
                    draft_row_id,
                )
                .await
                {
                    Ok(true) => {
                        conversation_id = None;
                    }
                    Ok(false) => {
                        tracing::info!(
                            target: "chat",
                            "[Chat] Preserved temporary conversation '{}' for hidden no-op turn because it contains other messages",
                            cid
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "chat",
                            "[Chat] Failed to clean up temporary conversation '{}' for hidden no-op turn: {}",
                            cid,
                            e
                        );
                    }
                }
            } else {
                cleanup_turn_artifacts(state, cid, &assistant_turn_id, draft_row_id).await;
            }
        }
        policy.emit(app,
            "chat-turn-text-complete",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "text": "",
                "translation_pending": false,
                "translation": serde_json::Value::Null,
            }),
        ).await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;
        policy.emit(app,
            "chat-turn-finish",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "status": "completed",
                "client_request_id": request.client_request_id,
                "conversation_id": identity.conversation_id,
                "assistant_message_id": draft_row_id,
            }),
        ).await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;
        return Ok(None);
    }

    if let Some(hooks) = hook_runtime.as_ref() {
        let hook_payload = build_chat_hook_payload(
            Some(identity.conversation_id.clone()),
            &char_id,
            Some(assistant_turn_id.clone()),
            Some(request.message.clone()),
            Some(full_response.clone()),
            None,
            request.hidden,
        );
        let hook_fut = hooks.emit_best_effort(&HookEvent::AfterLlmResponse, &hook_payload);
        tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut cancel_rx) => {
                tracing::info!(
                    target: "chat",
                    "[stream_chat] Turn {} (request {}) cancelled during AfterLlmResponse hook",
                    assistant_turn_id,
                    client_request_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
            _ = tokio::time::sleep(chat_hook_execution_timeout()) => {
                tracing::warn!(
                    target: "chat",
                    "[stream_chat] AfterLlmResponse hook timed out after {}s",
                    chat_hook_execution_timeout().as_secs()
                );
            }
            _ = hook_fut => {}
        }
    }

    let user_lang = prompt_snapshot.user_language.clone();
    let resp_lang = prompt_snapshot.response_language.clone();
    let translation_pending = all_translations.is_empty()
        && !full_response.is_empty()
        && !user_lang.is_empty()
        && !resp_lang.is_empty()
        && user_lang != resp_lang;

    policy.emit(app,
        "chat-turn-text-complete",
        serde_json::json!({
            "turn_id": assistant_turn_id,
            "text": full_response.clone(),
            "translation_pending": translation_pending,
            "translation": if all_translations.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(all_translations.join(" "))
            },
        }),
    ).await
    .map_err(|e| KokoroError::Chat(e.to_string()))?;

    // Fallback translation: if main LLM missed the [TRANSLATE:...] tag, use system LLM to fill in
    if translation_pending {
        tracing::info!(
            target: "chat",
            "[Chat] Fallback check: user_lang={:?}, resp_lang={:?}",
            user_lang, resp_lang
        );
        tracing::info!(
            target: "chat",
            "[Chat] Translation missing, triggering fallback translation into {}",
            user_lang
        );
        let fallback_messages = vec![
            system_message(format!(
                "You are a translator. Translate the following text into {}. Output only the translation, nothing else.",
                user_lang
            )),
            user_text_message(full_response.clone()),
        ];
        let fallback_fut = system_provider.chat(fallback_messages, None);
        let fallback_timeout = chat_fallback_execution_timeout();
        let fallback_res = tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut cancel_rx) => {
                tracing::info!(
                    target: "chat",
                    "[stream_chat] Turn {} (request {}) cancelled during fallback translation",
                    assistant_turn_id,
                    client_request_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
            _ = tokio::time::sleep(fallback_timeout) => {
                tracing::warn!(
                    target: "chat",
                    "[stream_chat] Turn {} fallback translation timed out after {}s",
                    assistant_turn_id,
                    fallback_timeout.as_secs()
                );
                None
            }
            res = fallback_fut => Some(res),
        };
        if let Some(res) = fallback_res {
            match res {
                Ok(translation) => {
                    let t = translation.trim().to_string();
                    if !t.is_empty() {
                        tracing::info!(target: "chat", "[Chat] Fallback translation succeeded ({} chars)", t.len());
                        all_translations.push(t);
                    }
                }
                Err(e) => {
                    tracing::error!(target: "chat", "[Chat] Fallback translation failed: {}", e);
                }
            }
        }
    }

    // Fallback cue: if main LLM never called play_cue, infer via system LLM
    if desktop && !cue_set_by_tool && !full_response.is_empty() {
        tracing::info!(target: "chat", "[Chat] Cue not set by tool, triggering fallback cue analysis");
        let mut cue_messages = vec![system_message(
                crate::ai::prompts::CUE_ANALYZER_PROMPT.to_string(),
        )];
        if let Some(cues) = cue_names.as_ref() {
            let mut cues = cues.iter().cloned().collect::<Vec<_>>();
            cues.sort();
            let available_cues = cues.join(", ");
            cue_messages.push(system_message(format!(
                "Available cues for the active model: {}.\nChoose exactly one from this list, or return null if none fit.",
                if available_cues.is_empty() { "(none)" } else { &available_cues }
            )));
        }
        cue_messages.push(user_text_message(full_response.clone()));
        let valid_fallback_cues = cue_names.as_ref();
        let cue_fut = system_provider.chat(cue_messages, None);
        let cue_timeout = chat_fallback_execution_timeout();
        let cue_res = tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut cancel_rx) => {
                tracing::info!(
                    target: "chat",
                    "[stream_chat] Turn {} (request {}) cancelled during fallback cue analysis",
                    assistant_turn_id,
                    client_request_id
                );
                return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()));
            }
            _ = tokio::time::sleep(cue_timeout) => {
                tracing::warn!(
                    target: "chat",
                    "[stream_chat] Turn {} fallback cue analysis timed out after {}s",
                    assistant_turn_id,
                    cue_timeout.as_secs()
                );
                None
            }
            res = cue_fut => Some(res),
        };
        if let Some(res) = cue_res {
            match res {
                Ok(json_str) => {
                    let clean = json_str
                        .trim()
                        .trim_start_matches("```json")
                        .trim_start_matches("```")
                        .trim_end_matches("```");
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean) {
                        if let Some(cue) = val.get("cue").and_then(|v| v.as_str()) {
                            let trimmed = cue.trim();
                            let is_valid = valid_fallback_cues
                                .as_ref()
                                .map(|cues| cues.contains(trimmed))
                                .unwrap_or(false);
                            if is_valid {
                                tracing::info!(target: "chat", "[Chat] Fallback cue: {}", trimmed);
                                let _ = policy.emit(app,
                                    "chat-cue",
                                    serde_json::json!({ "cue": trimmed, "source": "fallback-cue", "client_request_id":client_request_id,"turn_id":assistant_turn_id,"character_id":identity.character_id,"conversation_id":identity.conversation_id }),
                                ).await;
                            } else {
                                tracing::info!(target: "chat", "[Chat] Ignoring invalid fallback cue: {}", trimmed);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(target: "chat", "[Chat] Fallback cue analysis failed: {}", e);
                }
            }
        }
    }

    // Emit combined translation from all rounds
    if !all_translations.is_empty() {
        let combined_translation = all_translations.join(" ");
        final_translation = Some(combined_translation.clone());
        let _ = policy.emit(app,
            "chat-turn-translation",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "translation": combined_translation,
            }),
        ).await;
    }

    if let Some(turn) = external_turn.as_ref() {
        use base64::Engine as _;
        for image in &images {
            match tokio::fs::read(&image.path).await {
                Ok(bytes) => external_media.images.push(persistence::ExternalReplyImage {
                    prompt:image.prompt.clone(), mime_type:crate::vision::server::detect_image_mime(&bytes).unwrap_or_else(|| "image/png".into()),
                    file_name:Path::new(&image.path).file_name().and_then(|name| name.to_str()).unwrap_or("image.png").into(),
                    data_base64:base64::engine::general_purpose::STANDARD.encode(bytes),
                }),
                Err(_) => output_warnings.push("Kokoro Engine could not read the generated image.".into()),
            }
        }
        external_media.output_warnings = output_warnings.clone();
        let mut metadata = serde_json::json!({"turn_id": assistant_turn_id});
        if !all_reasoning_content.trim().is_empty() { metadata["reasoning_content"] = all_reasoning_content.into(); }
        if !final_provider_data.is_empty() { metadata["provider_data"] = final_provider_data.into(); }
        pending_tool_messages.push(MessageWrite::new("assistant", &canonical_response, Some(metadata.to_string()))
            .with_save_id(format!("{client_request_id}:assistant")));
        let work = turn.save_reply_batch(state, &canonical_response, final_translation.as_deref(), external_media.clone(), pending_tool_messages,
            Some(Arc::new(persistence::ExternalReplyExecution {
                speech: speech_snapshot.clone(), memory_provider: system_provider.clone(), memory_language: memory_target_language.clone(), memory_scope: memory_scope.clone(),
            }))).await?;
        super::follow_up::schedule_external_memory_tasks(policy.source(), &request.message, &char_id, turn, state, system_provider.clone(), &memory_target_language, work, memory_scope.clone()).await;
        let row = sqlx::query_scalar("SELECT id FROM conversation_messages WHERE conversation_id=? AND role='assistant' AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.chat_save_id') END=?")
            .bind(&turn.target.conversation_id).bind(format!("{client_request_id}:assistant")).fetch_one(&state.db).await.map_err(KokoroError::chat_persistence)?;
        let _ = policy.emit(app, "chat-turn-finish", serde_json::json!({"turn_id":assistant_turn_id,"client_request_id":client_request_id,"conversation_id":turn.target.conversation_id,"assistant_message_id":row,"status":"completed"})).await;
        return Ok(Some(row));
    }

    let upgrade_config = crate::config::load_memory_upgrade_config(&crate::ai::memory::memory_upgrade_config_path());
    let ingress_options = MemoryEventIngressOptions {
        enabled:upgrade_config.event_trigger_enabled,
        event_cooldown_secs:upgrade_config.event_cooldown_secs,
        intent_routing_enabled:upgrade_config.intent_routing_enabled,
    };
    let memory = {
        let _selection = state.selection.conversation_switch_lock.lock().await;
        if !request.hidden && state.is_memory_enabled() && state.external_memory.epoch() == memory_epoch
            && conversation_id.is_some()
        {
            let mut history = state.memory_history_for(&target, 10).await;
            if !full_response.is_empty() {
                history.push(crate::ai::context::Message { role:"assistant".into(), content:canonical_response.clone(), metadata:None });
                history.drain(..history.len().saturating_sub(10));
            }
            Some(crate::chat::post_commit::DesktopMemoryWork {
                epoch:memory_epoch, count:state.memory_progress_for(&target).await.0, history,
                language:memory_target_language,
                event:select_memory_ingress_decision(&request.message, &ingress_options),
                structured:should_use_structured_extraction(upgrade_config.structured_memory_enabled, &ingress_options),
            })
        } else { None }
    };
    let mut post_commit = crate::chat::post_commit::DesktopPostCommit {
        scope: Some(memory_scope.clone()),
        request_id: client_request_id.clone(), turn_id: assistant_turn_id.clone(),
        character_id:char_id.clone(), conversation_id:conversation_id.clone(),
        database_generation:crate::ai::context::database_operation_generation(),
        provider:system_provider.clone(), memory,
        background:if request.allow_image_gen.unwrap_or(false) && !full_response.is_empty() && !bg_generated_by_tool {
            Some(crate::chat::post_commit::DesktopBackgroundWork {
                reply:full_response.clone(), size:window_size_state.get().await, service:imagegen_state.clone(),
            })
        } else { None },
    };

    // The assistant, its related context and native tool rows, and conversation
    // metadata become visible as committed history in one short transaction.
    if !full_response.is_empty() || !pending_tool_messages.is_empty() {
        let _selection = state.selection.conversation_switch_lock.lock().await;
        memory_scope.ensure_owner_current()?;
        let create = conversation_id.is_none();
        let target = ConversationTarget { require_character: true,
            character_id: identity.character_id.clone(),
            conversation_id: identity.conversation_id.clone(),
            create,
        };
        let mut messages = pending_tool_messages;
        if request.hidden {
            if let Some(observation) = selected_vision_observation.as_ref() {
                messages.insert(0, vision_context_write(observation, &assistant_turn_id));
            }
        }
        if !full_response.is_empty() {
            let mut metadata = serde_json::json!({ "turn_id": assistant_turn_id });
            if !all_translations.is_empty() {
                metadata["translation"] = serde_json::Value::String(all_translations.join(" "));
            }
            if !all_reasoning_content.trim().is_empty() {
                metadata["reasoning_content"] = serde_json::Value::String(all_reasoning_content);
            }
            if !final_provider_data.is_empty() {
                metadata["provider_data"] = serde_json::Value::Array(final_provider_data);
            }
            let mut final_write = MessageWrite::new("assistant", canonical_response, Some(metadata.to_string()))
                .with_save_id(format!("{assistant_turn_id}:assistant"));
            if let Some(row) = draft_row_id {
                final_write = if messages.iter().any(|message| message.role == "tool") {
                    final_write.replacing_draft(row, assistant_turn_id.clone())
                } else {
                    final_write.finalizing(row, assistant_turn_id.clone())
                };
            }
            messages.push(final_write);
        }
        let retry_revision = if create {
            Some(0)
        } else {
            persistence::current_revision(&state.db, &target).await.ok().flatten()
        };
        let select_on_commit = create
            && state.current_conversation_generation() == bound_generation
            && state.get_character_id().await == char_id
            && state.selection.current_conversation_id.lock().await.is_none();
        let saved = match state.save_messages_locked(&target, &messages, select_on_commit, None).await {
            Ok(saved) => saved,
            Err(error) => {
                if let Some(expected_revision) = retry_revision {
                    let runtime = app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>();
                    runtime.prepare_save_retry(&client_request_id, PendingChatSave {
                        target: target.clone(),
                        messages,
                        expected_revision,
                        turn_id: assistant_turn_id.clone(),
                        user_message_id,
                        creation_generation: create.then_some(bound_generation),
                        text: full_response.clone(),
                        has_assistant_text: !full_response.is_empty(),
                        speech: speech.clone(),
                        speech_snapshot: speech_snapshot.clone(),
                        post_commit: { post_commit.conversation_id = Some(target.conversation_id.clone()); post_commit },
                    });
                }
                return Err(error);
            }
        };
        draft_row_id = if full_response.is_empty() { None } else { saved.last().map(|row| row.id) };
        if select_on_commit {
            app.state::<Arc<crate::chat::runtime::ChatRuntimeState>>().event("chat-turn-scope", &serde_json::json!({
                "turn_id": assistant_turn_id, "conversation_id": target.conversation_id,
                "conversation_generation": state.current_conversation_generation(),
            }));
        }
        conversation_id = Some(target.conversation_id);
    }

    post_commit.conversation_id = conversation_id.clone();
    post_commit.dispatch(app.clone());

        policy.emit(app,
            "chat-turn-finish",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "status": "completed",
                "client_request_id": request.client_request_id,
                "conversation_id": identity.conversation_id,
                "assistant_message_id": draft_row_id,
            }),
        ).await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;

        Ok(draft_row_id)
    }
    .await;

    match stream_result {
        Ok(assistant_message_id) => {
            if desktop && assistant_message_id.is_some() {
                if let Some(speech) = speech {
                    let text = app
                        .state::<Arc<crate::chat::runtime::ChatRuntimeState>>()
                        .turn_text(&client_request_id)
                        .unwrap_or_default();
                    crate::chat::runtime::speak_completed_turn(
                        app.clone(),
                        client_request_id,
                        assistant_turn_id.clone(),
                        text,
                        speech,
                        speech_snapshot.clone(),
                    );
                }
            }
            Ok(StreamChatResponse {
                speech_snapshot: speech_snapshot.clone(),
                text: String::new(),
                conversation_id: identity.conversation_id.clone(),
                user_message_id,
                assistant_message_id,
                client_request_id: request.client_request_id,
                status: Some("completed".to_string()),
                translation: final_translation,
                images,
                output_warnings,
                external_media,
                newly_committed: !desktop,
            })
        }
        Err(KokoroError::Chat(message)) if is_turn_cancelled_error_message(&message) => {
            tracing::info!(
                target: "chat",
                "[Chat] Turn {} cancelled by user",
                assistant_turn_id
            );
            // Remove the turn's technical rows (assistant_tool_calls/tool_result) and the
            // streaming draft, then emit the cancelled finish so the frontend reloads a
            // clean conversation.
            let draft_row_id = *draft_row_id_holder.lock().await;
            if let Some(ref cid) = conversation_id {
                if is_newly_created_for_hidden {
                    match delete_empty_conversation_if_unused(
                        state,
                        cid,
                        Some(&assistant_turn_id),
                        draft_row_id,
                    )
                    .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::info!(
                                target: "chat",
                                "[Chat] Preserved temporary conversation '{}' on turn cancel because it contains other messages",
                                cid
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                target: "chat",
                                "[Chat] Failed to clean up temporary conversation '{}' on turn cancel: {}",
                                cid,
                                e
                            );
                        }
                    }
                } else {
                    cleanup_turn_artifacts(state, cid, &assistant_turn_id, draft_row_id).await;
                }
            }
            policy
                .emit(
                    app,
                    "chat-turn-finish",
                    serde_json::json!({
                        "turn_id": assistant_turn_id,
                        "status": "cancelled",
                        "client_request_id": request.client_request_id,
                        "conversation_id": identity.conversation_id,
                        "assistant_message_id": serde_json::Value::Null,
                    }),
                )
                .await
                .map_err(|e| KokoroError::Chat(e.to_string()))?;
            Ok(StreamChatResponse {
                text: String::new(),
                conversation_id: identity.conversation_id.clone(),
                user_message_id,
                assistant_message_id: None,
                client_request_id: request.client_request_id,
                status: Some("cancelled".to_string()),
                ..Default::default()
            })
        }
        Err(error) => {
            // Non-cancel failures keep any partial draft and already-finalized rows, but
            // the turn's technical rows must not survive as orphan tool exchanges.
            if !matches!(error, KokoroError::ChatPersistence(_)) {
                if let Some(ref cid) = conversation_id {
                    cleanup_turn_artifacts(state, cid, &assistant_turn_id, None).await;
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::executor::{
        assistant_tool_call_metadata_value_for_test, tool_metadata_value_for_test,
        ToolExecutionOutcome, ToolInvocation,
    };
    use crate::actions::registry::{
        ActionInfo, ActionPermissionLevel, ActionRiskTag, ActionSource,
    };
    use crate::ai::memory_event_ingress::{
        build_memory_extraction_options_for_test, build_memory_ingress_decision_for_test,
        MemoryEventType,
    };
    use crate::hooks::HookPayload;

    #[test]
    fn tool_completion_notice_exists_for_every_supported_language() {
        for (languages, catalog) in [
            (
                &["en", "English"][..],
                include_str!("../../../src/ui/locales/en.json"),
            ),
            (
                &["zh", "zh-CN", "中文", "简体中文"][..],
                include_str!("../../../src/ui/locales/zh.json"),
            ),
            (
                &["zh-TW", "zh-HK", "繁體中文"][..],
                include_str!("../../../src/ui/locales/zh-TW.json"),
            ),
            (
                &["ja", "ja-JP", "日本語"][..],
                include_str!("../../../src/ui/locales/ja.json"),
            ),
            (
                &["ko", "ko-KR", "한국어"][..],
                include_str!("../../../src/ui/locales/ko.json"),
            ),
            (
                &["ru", "ru-RU", "Русский"][..],
                include_str!("../../../src/ui/locales/ru.json"),
            ),
        ] {
            let catalog: serde_json::Value = serde_json::from_str(catalog).unwrap();
            let expected = catalog["chat"]["tool_only_completion"]
                .as_str()
                .filter(|notice| !notice.trim().is_empty())
                .expect("every supported locale needs the root chat completion notice");
            for language in languages {
                assert_eq!(
                    tool_only_completion_notice(language).unwrap(),
                    expected,
                    "{language}"
                );
            }
        }
    }

    #[test]
    fn tool_completion_notice_falls_back_when_a_translation_is_missing_or_malformed() {
        let expected = tool_only_completion_notice("English").unwrap();
        for catalog in [
            "{",
            "{}",
            r#"{"chat":{"tool_only_completion":null}}"#,
            r#"{"chat":{"tool_only_completion":"  "}}"#,
        ] {
            assert_eq!(
                tool_completion_notice_from_catalog(catalog).unwrap(),
                expected
            );
        }
        assert_eq!(
            tool_only_completion_notice("unsupported language").unwrap(),
            expected
        );
    }

    #[tokio::test]
    async fn failed_user_save_cleanup_cannot_erase_a_retried_turn_registration() {
        let state = Arc::new(TurnCancellationState::new());
        state
            .register_turn_with_request("turn", Some("request"))
            .await;
        let mut previous = TurnCancellationGuard::new(state.clone(), "turn".into());
        previous.clear_before_retry().await;
        assert!(state.resolve_turn_id("request").await.is_none());

        state
            .register_turn_with_request("turn", Some("request"))
            .await;
        drop(previous);
        tokio::task::yield_now().await;
        assert_eq!(
            state.resolve_turn_id("request").await.as_deref(),
            Some("turn")
        );
        let mut signal = state.subscribe_cancellation("turn").await.unwrap();
        state
            .cancel_turn("request", Some("test cancellation".into()))
            .await
            .unwrap();
        signal.changed().await.unwrap();
        assert!(*signal.borrow());
    }

    /// 验证 StreamChatResponse 序列化及向前兼容性（当缺少 assistant_message_id/client_request_id/status 时默认解析为 None）。
    #[test]
    fn test_stream_chat_response_serialization_and_backward_compatibility() {
        // 包含 assistant_message_id 与 client_request_id 与 status 时的正向序列化与反序列化
        let res = StreamChatResponse {
            text: String::new(),
            conversation_id: "conv-123".to_string(),
            user_message_id: Some(10),
            assistant_message_id: Some(11),
            client_request_id: Some("req-123".to_string()),
            status: Some("completed".to_string()),
            ..Default::default()
        };
        let json_str = serde_json::to_string(&res).expect("serialization succeeds");
        let deserialized: StreamChatResponse =
            serde_json::from_str(&json_str).expect("deserialization succeeds");
        assert_eq!(deserialized, res);

        let cancelled_res = StreamChatResponse {
            text: String::new(),
            conversation_id: "conv-123".to_string(),
            user_message_id: Some(10),
            assistant_message_id: None,
            client_request_id: Some("req-123".to_string()),
            status: Some("cancelled".to_string()),
            ..Default::default()
        };
        let cancelled_json =
            serde_json::to_string(&cancelled_res).expect("cancelled serialization succeeds");
        let deserialized_cancelled: StreamChatResponse =
            serde_json::from_str(&cancelled_json).expect("cancelled deserialization succeeds");
        assert_eq!(deserialized_cancelled, cancelled_res);

        let error_res = StreamChatResponse {
            text: String::new(),
            conversation_id: "conv-123".to_string(),
            user_message_id: Some(10),
            assistant_message_id: None,
            client_request_id: Some("req-123".to_string()),
            status: Some("error".to_string()),
            ..Default::default()
        };
        let error_json = serde_json::to_string(&error_res).expect("error serialization succeeds");
        let deserialized_error: StreamChatResponse =
            serde_json::from_str(&error_json).expect("error deserialization succeeds");
        assert_eq!(deserialized_error, error_res);

        // 向前兼容验证：如果接收到没有 assistant_message_id / client_request_id / status 的旧版 JSON
        let legacy_json = r#"{"conversation_id":"conv-legacy","user_message_id":42}"#;
        let legacy_res: StreamChatResponse =
            serde_json::from_str(legacy_json).expect("legacy deserialization succeeds");
        assert_eq!(legacy_res.conversation_id, "conv-legacy");
        assert_eq!(legacy_res.user_message_id, Some(42));
        assert_eq!(legacy_res.assistant_message_id, None);
        assert_eq!(legacy_res.client_request_id, None);
        assert_eq!(legacy_res.status, None);
    }

    #[test]
    fn chat_memory_ingress_triggers_immediate_extraction_for_profile_event() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.event.event_type, MemoryEventType::Profile);
        assert_eq!(decision.trigger_label, "event_profile");
    }

    #[test]
    fn chat_memory_ingress_returns_none_when_event_trigger_disabled() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            false,
            true,
            120,
        );
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_structured_path_requires_both_flags() {
        assert!(build_memory_extraction_options_for_test(true, true, true));
        assert!(!build_memory_extraction_options_for_test(true, true, false));
        assert!(!build_memory_extraction_options_for_test(true, false, true));
    }

    #[test]
    fn chat_memory_ingress_falls_back_to_default_priority_when_intent_routing_disabled() {
        let decision = build_memory_ingress_decision_for_test(
            "不是我喜欢猫，下周我要继续做前端",
            true,
            false,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_correction");
    }

    #[test]
    fn chat_memory_ingress_uses_priority_routing_when_intent_routing_enabled() {
        let decision = build_memory_ingress_decision_for_test(
            "不是我喜欢猫，下周我要继续做前端",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn chat_memory_ingress_keeps_structured_disabled_without_flag() {
        assert!(!build_memory_extraction_options_for_test(false, true, true));
    }

    #[test]
    fn chat_memory_ingress_keeps_structured_disabled_without_event_trigger() {
        assert!(!build_memory_extraction_options_for_test(true, false, true));
    }

    #[test]
    fn chat_memory_ingress_keeps_structured_disabled_without_intent_routing() {
        assert!(!build_memory_extraction_options_for_test(true, true, false));
    }

    #[test]
    fn chat_memory_ingress_enables_structured_when_all_flags_on() {
        assert!(build_memory_extraction_options_for_test(true, true, true));
    }

    #[test]
    fn chat_memory_ingress_prefers_profile_trigger_label_for_profile_event() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_profile");
    }

    #[test]
    fn chat_memory_ingress_prefers_plan_trigger_label_for_plan_event() {
        let decision = build_memory_ingress_decision_for_test(
            "下周我要继续做这个记忆系统架构",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_plan");
    }

    #[test]
    fn chat_memory_ingress_prefers_preference_trigger_label_for_preference_event() {
        let decision = build_memory_ingress_decision_for_test("我更喜欢 Rust", true, true, 120)
            .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn chat_memory_ingress_prefers_correction_trigger_label_for_correction_event() {
        let decision =
            build_memory_ingress_decision_for_test("不是我喜欢猫，是我以前养过猫", true, true, 120)
                .expect("decision");
        assert_eq!(decision.trigger_label, "event_correction");
    }

    #[test]
    fn chat_memory_ingress_ignores_small_talk_even_when_flags_on() {
        let decision = build_memory_ingress_decision_for_test("哈哈好的", true, true, 120);
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_preserves_cooldown_seconds_from_options() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            true,
            true,
            333,
        )
        .expect("decision");
        assert_eq!(decision.event.cooldown_secs, 333);
    }

    #[test]
    fn chat_memory_ingress_returns_none_for_blank_input() {
        let decision = build_memory_ingress_decision_for_test("   ", true, true, 120);
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_detects_profile_event_type_for_profile_input() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.event.event_type, MemoryEventType::Profile);
    }

    #[test]
    fn chat_memory_ingress_detects_plan_event_type_for_plan_input() {
        let decision = build_memory_ingress_decision_for_test(
            "下周我要继续做这个记忆系统架构",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.event.event_type, MemoryEventType::Plan);
    }

    #[test]
    fn chat_memory_ingress_detects_preference_event_type_for_preference_input() {
        let decision = build_memory_ingress_decision_for_test("我更喜欢 Rust", true, true, 120)
            .expect("decision");
        assert_eq!(decision.event.event_type, MemoryEventType::Preference);
    }

    #[test]
    fn chat_memory_ingress_detects_correction_event_type_for_correction_input() {
        let decision =
            build_memory_ingress_decision_for_test("不是我喜欢猫，是我以前养过猫", true, true, 120)
                .expect("decision");
        assert_eq!(decision.event.event_type, MemoryEventType::Correction);
    }

    #[test]
    fn chat_memory_ingress_no_structured_without_enable_flag() {
        assert!(!build_memory_extraction_options_for_test(false, true, true));
    }

    #[test]
    fn chat_memory_ingress_no_structured_without_trigger_flag() {
        assert!(!build_memory_extraction_options_for_test(true, false, true));
    }

    #[test]
    fn chat_memory_ingress_no_structured_without_routing_flag() {
        assert!(!build_memory_extraction_options_for_test(true, true, false));
    }

    #[test]
    fn chat_memory_ingress_structured_enabled_only_when_all_are_enabled() {
        assert!(build_memory_extraction_options_for_test(true, true, true));
    }

    #[test]
    fn chat_memory_ingress_none_for_disabled_event_trigger_on_small_talk() {
        let decision = build_memory_ingress_decision_for_test("哈哈好的", false, true, 120);
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_none_for_disabled_event_trigger_on_profile_input() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            false,
            true,
            120,
        );
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_none_for_non_matching_text() {
        let decision = build_memory_ingress_decision_for_test("今天天气不错", true, true, 120);
        assert!(decision.is_none());
    }

    #[test]
    fn chat_memory_ingress_prefers_first_priority_event_under_intent_routing() {
        let decision = build_memory_ingress_decision_for_test(
            "我喜欢 Rust，下周我要继续做前端",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn chat_memory_ingress_returns_first_detected_when_intent_routing_off() {
        let decision = build_memory_ingress_decision_for_test(
            "我喜欢 Rust，下周我要继续做前端",
            true,
            false,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn chat_memory_ingress_intent_priority_beats_profile() {
        let decision =
            build_memory_ingress_decision_for_test("我是前端开发者，我喜欢 Rust", true, true, 120)
                .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn chat_memory_ingress_default_order_keeps_profile_when_only_profile_matches() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            true,
            false,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_profile");
    }

    #[test]
    fn chat_memory_ingress_default_order_keeps_plan_when_only_plan_matches() {
        let decision = build_memory_ingress_decision_for_test(
            "下周我要继续做这个记忆系统架构",
            true,
            false,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_plan");
    }

    #[test]
    fn chat_memory_ingress_default_order_keeps_correction_when_only_correction_matches() {
        let decision = build_memory_ingress_decision_for_test(
            "不是我喜欢猫，是我以前养过猫",
            true,
            false,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_correction");
    }

    #[test]
    fn chat_memory_ingress_default_order_keeps_preference_when_only_preference_matches() {
        let decision = build_memory_ingress_decision_for_test("我更喜欢 Rust", true, false, 120)
            .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn test_build_chat_error_event_contains_observability_fields() {
        let payload = build_chat_error_event("llm_stream", "provider timeout", "turn-123", true);

        assert_eq!(payload.code, "CHAT_STREAM_ERROR");
        assert_eq!(payload.stage, "llm_stream");
        assert_eq!(payload.retryable, true);
        assert_eq!(payload.trace_id, "turn-123");
        assert_eq!(payload.message, "provider timeout");
    }

    #[test]
    fn test_chat_error_event_serializes_to_json_with_trace_id() {
        let payload = build_chat_error_event("llm_stream", "provider timeout", "turn-abc", true);
        let json = serde_json::to_string(&payload).expect("serialize payload");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse json");

        assert_eq!(value["trace_id"], "turn-abc");
        assert_eq!(value["stage"], "llm_stream");
        assert_eq!(value["code"], "CHAT_STREAM_ERROR");
    }

    #[test]
    fn test_build_chat_hook_payload_preserves_character_and_hidden() {
        let payload = build_chat_hook_payload(
            Some("conv-1".to_string()),
            "char-1",
            Some("turn-1".to_string()),
            Some("hello".to_string()),
            None,
            None,
            true,
        );

        let HookPayload::Chat(chat) = payload else {
            panic!("expected chat payload");
        };

        assert_eq!(chat.conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(chat.character_id, "char-1");
        assert_eq!(chat.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(chat.message.as_deref(), Some("hello"));
        assert!(chat.hidden);
    }

    #[test]
    fn test_build_chat_hook_payload_keeps_final_response_only() {
        let payload = build_chat_hook_payload(
            None,
            "char-2",
            Some("turn-2".to_string()),
            Some("user".to_string()),
            Some("final response".to_string()),
            None,
            false,
        );

        let HookPayload::Chat(chat) = payload else {
            panic!("expected chat payload");
        };

        assert_eq!(chat.response.as_deref(), Some("final response"));
        assert_eq!(chat.tool_round, None);
        assert!(!chat.hidden);
    }

    #[test]
    fn test_apply_before_llm_request_payload_uses_modified_request_for_hidden_message() {
        let payload = BeforeLlmRequestPayload {
            conversation_id: Some("conv-1".to_string()),
            character_id: "char-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            hidden: true,
            request_message: "modified hidden".to_string(),
            messages: vec![
                BeforeLlmRequestMessage {
                    role: "system".to_string(),
                    content: "system prompt".to_string(),
                },
                BeforeLlmRequestMessage {
                    role: "user".to_string(),
                    content: "modified user".to_string(),
                },
            ],
        };

        let original_prompt_messages = vec![
            Message {
                role: "system".to_string(),
                content: "system prompt".to_string(),
                metadata: None,
            },
            Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                metadata: None,
            },
        ];

        let (request_message, client_messages) =
            apply_before_llm_request_payload(payload, &original_prompt_messages)
                .expect("payload should convert");

        assert_eq!(request_message, "modified hidden");
        assert_eq!(client_messages.len(), 2);
        assert_eq!(
            extract_message_text(&client_messages[0].message),
            "system prompt"
        );
        assert_eq!(
            extract_message_text(&client_messages[1].message),
            "modified user"
        );
    }

    #[test]
    fn test_build_effective_before_llm_request_preserves_prompt_order() {
        let prompt_messages = vec![
            Message {
                role: "system".to_string(),
                content: "system prompt".to_string(),
                metadata: None,
            },
            Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                metadata: None,
            },
        ];

        let (request_message, client_messages) = build_effective_before_llm_request(
            Some("conv-1".to_string()),
            "char-1",
            Some("turn-1".to_string()),
            "hello".to_string(),
            false,
            &prompt_messages,
        )
        .expect("payload should convert");

        assert_eq!(request_message, "hello");
        assert_eq!(client_messages.len(), 2);
        assert_eq!(
            extract_message_text(&client_messages[0].message),
            "system prompt"
        );
        assert_eq!(extract_message_text(&client_messages[1].message), "hello");
    }

    #[test]
    fn proactive_noop_accepts_empty_and_pass_only() {
        assert!(is_proactive_noop_response(""));
        assert!(is_proactive_noop_response("  \n"));
        assert!(is_proactive_noop_response("PASS"));
        assert!(is_proactive_noop_response(" pass "));
        assert!(!is_proactive_noop_response("pass for now"));
        assert!(!is_proactive_noop_response("A short comment"));
    }

    #[test]
    fn vision_context_inserts_before_current_hidden_user_message() {
        let now = chrono::Utc::now();
        let observation = crate::vision::context::VisionObservation {
            id: "obs-1".to_string(),
            frame_id: Some("frame-1".to_string()),
            captured_at: now,
            analyzed_at: now,
            summary: "Current screen summary".to_string(),
            source: crate::vision::context::VisionObservationSource::Auto,
        };
        let mut messages = vec![
            plain_llm_message(system_message("system")),
            plain_llm_message(user_text_message("older visible user")),
            plain_llm_message(user_text_message("current hidden instruction")),
        ];

        insert_vision_context_before_latest_user(&mut messages, &observation);

        assert_eq!(
            extract_message_text(&messages[1].message),
            "older visible user"
        );
        assert!(extract_message_text(&messages[2].message).contains("[Screen context]"));
        assert!(!extract_message_text(&messages[2].message).contains("Captured at:"));
        assert!(!extract_message_text(&messages[2].message).contains("Source:"));
        assert_eq!(
            extract_message_text(&messages[3].message),
            "current hidden instruction"
        );
    }

    #[test]
    fn test_deny_kind_for_tool_error_maps_known_prefixes() {
        assert_eq!(
            deny_kind_for_tool_error(
                "Denied pending approval: permission level 'elevated' requires approval"
            ),
            "pending_approval"
        );
        assert_eq!(
            deny_kind_for_tool_error("Denied by fail-closed policy: blocked risk tag 'sensitive'"),
            "fail_closed"
        );
        assert_eq!(
            deny_kind_for_tool_error("Denied by policy: blocked risk tag 'read'"),
            "policy_denied"
        );
        assert_eq!(
            deny_kind_for_tool_error("Denied by hook: blocked"),
            "hook_denied"
        );
    }

    #[test]
    fn test_deny_kind_for_tool_error_defaults_to_execution_error() {
        assert_eq!(
            deny_kind_for_tool_error("database timeout"),
            "execution_error"
        );
    }

    #[test]
    fn test_tool_error_payload_includes_deny_kind_and_original_error() {
        assert_eq!(
            tool_trace_error_deny_kind("Denied by policy: blocked risk tag 'read'"),
            Some("policy_denied".to_string())
        );
        assert_eq!(
            tool_trace_error_message("Denied by policy: blocked risk tag 'read'"),
            Some("Denied by policy: blocked risk tag 'read'".to_string())
        );
    }

    #[test]
    fn tool_error_payload_prefers_permission_decision_over_error_prefix() {
        let outcome = sample_tool_outcome_with_decision(
            crate::actions::PermissionDecision::DenyFailClosed {
                reason: "boom".into(),
            },
            Err("custom message without prefix".to_string()),
        );
        let payload = tool_error_payload(&outcome, "turn-1", "custom message without prefix");
        assert_eq!(
            payload.get("deny_kind").and_then(|v| v.as_str()),
            Some("fail_closed")
        );
    }

    #[test]
    fn tool_trace_error_deny_kind_prefers_outcome_decision_when_available() {
        let outcome = sample_tool_outcome_with_decision(
            crate::actions::PermissionDecision::DenyPolicy {
                reason: "blocked risk tag 'read'".into(),
            },
            Err("random message without prefix".to_string()),
        );
        let payload = tool_error_payload(&outcome, "turn-1", "random message without prefix");

        assert_eq!(
            payload.get("deny_kind").and_then(|v| v.as_str()),
            Some("policy_denied")
        );
    }

    #[test]
    fn test_tool_success_payload_keeps_result_without_deny_kind() {
        assert!(tool_trace_success_has_no_deny_kind());
        assert_eq!(tool_trace_success_message(), Some("ok".to_string()));
    }

    #[test]
    fn test_pending_approval_trace_payload_includes_request_id_and_requested_status() {
        let payload = pending_tool_trace_payload_for_test(
            &sample_tool_trace_outcome_for_test(),
            "turn-1",
            "Denied pending approval: risk tag 'write' requires approval",
            "req-1",
        );
        assert_eq!(
            payload.get("approval_request_id").and_then(|v| v.as_str()),
            Some("req-1")
        );
        assert_eq!(
            payload.get("approval_status").and_then(|v| v.as_str()),
            Some("requested")
        );
        assert_eq!(
            payload.get("deny_kind").and_then(|v| v.as_str()),
            Some("pending_approval")
        );
    }

    #[test]
    fn test_approval_result_payloads_include_resolved_status() {
        let outcome = sample_tool_trace_outcome_for_test();
        let approved = approved_tool_trace_payload_for_test(
            &outcome,
            "turn-1",
            &sample_action_result("ok"),
            "req-1",
        );
        assert_eq!(
            approved.get("approval_status").and_then(|v| v.as_str()),
            Some("approved")
        );
        assert_eq!(
            approved.get("approval_request_id").and_then(|v| v.as_str()),
            Some("req-1")
        );

        let rejected = rejected_tool_trace_payload_for_test(
            &outcome,
            "turn-1",
            "Denied pending approval: rejected by user",
            "req-1",
        );
        assert_eq!(
            rejected.get("approval_status").and_then(|v| v.as_str()),
            Some("rejected")
        );
        assert_eq!(
            rejected.get("approval_request_id").and_then(|v| v.as_str()),
            Some("req-1")
        );
    }

    fn sample_metadata_outcome() -> ToolExecutionOutcome {
        ToolExecutionOutcome {
            invocation: ToolInvocation {
                tool_call_id: Some("call-1".to_string()),
                name: "read_file".to_string(),
                args: HashMap::from([("path".to_string(), "README.md".to_string())]),
            },
            action: Some(ActionInfo {
                id: "mcp__filesystem__read_file".to_string(),
                name: "read_file".to_string(),
                source: ActionSource::Mcp,
                server_name: Some("filesystem".to_string()),
                description: "Read file".to_string(),
                parameters: vec![],
                needs_feedback: true,
                risk_tags: vec![ActionRiskTag::Read],
                permission_level: ActionPermissionLevel::Safe,
            }),
            registry_generation: 0,
            result: Ok(sample_action_result("ok")),
            needs_feedback: true,
            permission_decision: Some(crate::actions::PermissionDecision::Allow),
        }
    }

    #[test]
    fn test_assistant_tool_call_metadata_includes_canonical_identity_fields() {
        let outcome = sample_metadata_outcome();
        let assistant_tool_call_metadata = serde_json::json!({
            "type": "assistant_tool_calls",
            "turn_id": "turn-1",
            "tool_calls": [assistant_tool_call_metadata_value_for_test(&outcome, "call-1")],
        });

        let tool_call = &assistant_tool_call_metadata["tool_calls"][0];
        assert_eq!(
            assistant_tool_call_metadata
                .get("type")
                .and_then(|v| v.as_str()),
            Some("assistant_tool_calls")
        );
        assert_eq!(
            assistant_tool_call_metadata
                .get("turn_id")
                .and_then(|v| v.as_str()),
            Some("turn-1")
        );
        assert_eq!(tool_call.get("id").and_then(|v| v.as_str()), Some("call-1"));
        assert_eq!(
            tool_call.get("tool_id").and_then(|v| v.as_str()),
            Some("mcp__filesystem__read_file")
        );
        assert_eq!(
            tool_call.get("tool_name").and_then(|v| v.as_str()),
            Some("read_file")
        );
        assert_eq!(
            tool_call.get("source").and_then(|v| v.as_str()),
            Some("mcp")
        );
        assert_eq!(
            tool_call.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            tool_call.get("needs_feedback").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            tool_call.get("permission_level").and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            tool_call
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            tool_call.get("arguments").and_then(|v| v.as_str()),
            Some("{\"path\":\"README.md\"}")
        );
    }

    #[test]
    fn test_tool_result_metadata_includes_canonical_identity_fields() {
        let outcome = sample_metadata_outcome();
        let tool_metadata = tool_metadata_value_for_test(&outcome, "call-1", "turn-1");

        assert_eq!(
            tool_metadata.get("type").and_then(|v| v.as_str()),
            Some("tool_result")
        );
        assert_eq!(
            tool_metadata.get("turn_id").and_then(|v| v.as_str()),
            Some("turn-1")
        );
        assert_eq!(
            tool_metadata.get("tool_call_id").and_then(|v| v.as_str()),
            Some("call-1")
        );
        assert_eq!(
            tool_metadata.get("tool_id").and_then(|v| v.as_str()),
            Some("mcp__filesystem__read_file")
        );
        assert_eq!(
            tool_metadata.get("tool_name").and_then(|v| v.as_str()),
            Some("read_file")
        );
        assert_eq!(
            tool_metadata.get("source").and_then(|v| v.as_str()),
            Some("mcp")
        );
        assert_eq!(
            tool_metadata.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            tool_metadata
                .get("needs_feedback")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            tool_metadata
                .get("permission_level")
                .and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            tool_metadata
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
    }

    #[test]
    fn test_tool_trace_payloads_include_identity_and_permission_fields() {
        let outcome = sample_metadata_outcome();
        let success = tool_success_payload(&outcome, "turn-1", &sample_action_result("ok"));
        assert_eq!(
            success.get("tool").and_then(|v| v.as_str()),
            Some("read_file")
        );
        assert_eq!(
            success.get("tool_id").and_then(|v| v.as_str()),
            Some("mcp__filesystem__read_file")
        );
        assert_eq!(success.get("source").and_then(|v| v.as_str()), Some("mcp"));
        assert_eq!(
            success.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            success.get("needs_feedback").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            success.get("permission_level").and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            success
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );

        let pending = pending_tool_trace_payload_for_test(
            &outcome,
            "turn-1",
            "Denied pending approval: permission level 'elevated' requires approval",
            "req-1",
        );
        assert_eq!(pending.get("source").and_then(|v| v.as_str()), Some("mcp"));
        assert_eq!(
            pending.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            pending.get("needs_feedback").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            pending.get("permission_level").and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            pending
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            pending.get("approval_request_id").and_then(|v| v.as_str()),
            Some("req-1")
        );
        assert_eq!(
            pending.get("approval_status").and_then(|v| v.as_str()),
            Some("requested")
        );

        let approved = approved_tool_trace_payload_for_test(
            &outcome,
            "turn-1",
            &sample_action_result("ok"),
            "req-1",
        );
        assert_eq!(approved.get("source").and_then(|v| v.as_str()), Some("mcp"));
        assert_eq!(
            approved.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            approved.get("needs_feedback").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            approved.get("permission_level").and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            approved
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            approved.get("approval_status").and_then(|v| v.as_str()),
            Some("approved")
        );

        let rejected = rejected_tool_trace_payload_for_test(
            &outcome,
            "turn-1",
            "Denied pending approval: rejected by user",
            "req-1",
        );
        assert_eq!(rejected.get("source").and_then(|v| v.as_str()), Some("mcp"));
        assert_eq!(
            rejected.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            rejected.get("needs_feedback").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            rejected.get("permission_level").and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            rejected
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            rejected.get("approval_status").and_then(|v| v.as_str()),
            Some("rejected")
        );

        let approved_error =
            approved_tool_error_payload(&outcome, "turn-1", "execution failed", "req-1");
        assert_eq!(
            approved_error.get("source").and_then(|v| v.as_str()),
            Some("mcp")
        );
        assert_eq!(
            approved_error.get("server_name").and_then(|v| v.as_str()),
            Some("filesystem")
        );
        assert_eq!(
            approved_error
                .get("needs_feedback")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            approved_error
                .get("permission_level")
                .and_then(|v| v.as_str()),
            Some("safe")
        );
        assert_eq!(
            approved_error
                .get("risk_tags")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            approved_error
                .get("approval_status")
                .and_then(|v| v.as_str()),
            Some("approved")
        );
    }

    #[tokio::test]
    async fn test_pending_tool_approval_state_generates_request_id_and_resolves_approve() {
        let state = PendingToolApprovalState::new();
        let request_id = state
            .register(
                "turn-1".to_string(),
                "builtin__write_note".to_string(),
                "write_note".to_string(),
                HashMap::from([("query".to_string(), "kokoro".to_string())]),
            )
            .await;

        assert!(!request_id.is_empty());
        let receiver = state
            .take_receiver(&request_id)
            .await
            .expect("receiver should exist");
        approve_tool_approval_inner(&state, request_id.clone())
            .await
            .expect("approve should succeed");
        match receiver.await.expect("decision should resolve") {
            ToolApprovalDecision::Approved => {}
            other => panic!("expected approved decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_pending_tool_approval_state_resolves_reject_and_unknown_id_errors() {
        let state = PendingToolApprovalState::new();
        let request_id = state
            .register(
                "turn-2".to_string(),
                "builtin__write_note".to_string(),
                "write_note".to_string(),
                HashMap::new(),
            )
            .await;

        let receiver = state
            .take_receiver(&request_id)
            .await
            .expect("receiver should exist");
        reject_tool_approval_inner(
            &state,
            request_id.clone(),
            Some("user rejected".to_string()),
        )
        .await
        .expect("reject should succeed");
        match receiver.await.expect("decision should resolve") {
            ToolApprovalDecision::Rejected { reason } => {
                assert_eq!(reason.as_deref(), Some("user rejected"));
            }
            other => panic!("expected rejected decision, got {other:?}"),
        }

        let missing = approve_tool_approval_inner(&state, "missing".to_string()).await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn pending_tool_approval_state_rejects_second_resolution() {
        let state = PendingToolApprovalState::new();
        let request_id = state
            .register(
                "turn-1".into(),
                "tool-1".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;

        let first = approve_tool_approval_inner(&state, request_id.clone()).await;
        assert!(first.is_ok());

        let second =
            reject_tool_approval_inner(&state, request_id.clone(), Some("late reject".into()))
                .await;
        match second {
            Err(KokoroError::Validation(message)) => {
                assert!(message.contains("already resolved"));
            }
            other => panic!("expected already-resolved validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pending_tool_approval_state_keeps_unknown_id_error_distinct() {
        let state = PendingToolApprovalState::new();
        let missing = approve_tool_approval_inner(&state, "missing".to_string()).await;
        match missing {
            Err(KokoroError::Validation(message)) => {
                assert!(message.contains("Unknown approval request"));
            }
            other => panic!("expected unknown-request validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_pending_tool_approval_cancel_approval_marks_resolved_and_rejects_late_approval() {
        let state = PendingToolApprovalState::new();
        let request_id = state
            .register(
                "turn-c1".to_string(),
                "builtin__test".to_string(),
                "test".to_string(),
                HashMap::new(),
            )
            .await;

        assert!(state.is_pending(&request_id).await);
        state.cancel_approval(&request_id).await;
        assert!(!state.is_pending(&request_id).await);

        let late_approve = approve_tool_approval_inner(&state, request_id.clone()).await;
        match late_approve {
            Err(KokoroError::Validation(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected already-resolved error, got {other:?}"),
        }

        let late_reject = reject_tool_approval_inner(&state, request_id, None).await;
        match late_reject {
            Err(KokoroError::Validation(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected already-resolved error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_pending_tool_approval_cancel_turn_cleans_up_all_turn_requests() {
        let state = PendingToolApprovalState::new();
        let req1 = state
            .register(
                "turn-multi".into(),
                "tool1".into(),
                "tool1".into(),
                HashMap::new(),
            )
            .await;
        let req2 = state
            .register(
                "turn-multi".into(),
                "tool2".into(),
                "tool2".into(),
                HashMap::new(),
            )
            .await;
        let req_other = state
            .register(
                "turn-other".into(),
                "tool3".into(),
                "tool3".into(),
                HashMap::new(),
            )
            .await;

        assert!(state.is_pending(&req1).await);
        assert!(state.is_pending(&req2).await);
        assert!(state.is_pending(&req_other).await);

        state.cancel_turn("turn-multi").await;

        assert!(!state.is_pending(&req1).await);
        assert!(!state.is_pending(&req2).await);
        assert!(state.is_pending(&req_other).await);
    }

    #[tokio::test]
    async fn test_wait_for_tool_approval_decision_cancelled_while_waiting() {
        let cancel_state = TurnCancellationState::new();
        cancel_state.register_turn("turn-wait-cancel").await;
        let approval_state = PendingToolApprovalState::new();

        let req_id = approval_state
            .register(
                "turn-wait-cancel".into(),
                "tool".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;
        let receiver = approval_state
            .take_receiver(&req_id)
            .await
            .expect("receiver should exist");

        let cancel_state_clone = Arc::new(cancel_state);
        let cancel_state_task = cancel_state_clone.clone();

        let cancel_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            cancel_state_task
                .cancel_turn("turn-wait-cancel", Some("user_stop".into()))
                .await
        });

        let wait_res = wait_for_tool_approval_decision(
            &approval_state,
            &cancel_state_clone,
            "turn-wait-cancel",
            &req_id,
            receiver,
        )
        .await;

        cancel_handle.await.unwrap().unwrap();

        match wait_res {
            Err(KokoroError::Chat(msg)) => {
                assert_eq!(msg, TURN_CANCELLED_BY_USER_MESSAGE);
            }
            other => panic!("expected cancelled chat error, got {other:?}"),
        }

        // Approval request must have been cleaned up and marked resolved
        assert!(!approval_state.is_pending(&req_id).await);
        let late_approve = approve_tool_approval_inner(&approval_state, req_id).await;
        assert!(late_approve.is_err());
    }

    #[tokio::test]
    async fn test_wait_for_tool_approval_decision_pre_cancelled() {
        let cancel_state = TurnCancellationState::new();
        cancel_state.register_turn("turn-precancel").await;
        cancel_state
            .cancel_turn("turn-precancel", Some("already_stopped".into()))
            .await
            .unwrap();

        let approval_state = PendingToolApprovalState::new();
        let req_id = approval_state
            .register(
                "turn-precancel".into(),
                "tool".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;
        let receiver = approval_state
            .take_receiver(&req_id)
            .await
            .expect("receiver should exist");

        let wait_res = wait_for_tool_approval_decision(
            &approval_state,
            &cancel_state,
            "turn-precancel",
            &req_id,
            receiver,
        )
        .await;

        match wait_res {
            Err(KokoroError::Chat(msg)) => {
                assert_eq!(msg, TURN_CANCELLED_BY_USER_MESSAGE);
            }
            other => panic!("expected cancelled chat error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_wait_for_tool_approval_decision_approved_normally() {
        let cancel_state = TurnCancellationState::new();
        cancel_state.register_turn("turn-normal-approve").await;
        let approval_state = PendingToolApprovalState::new();

        let req_id = approval_state
            .register(
                "turn-normal-approve".into(),
                "tool".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;
        let receiver = approval_state
            .take_receiver(&req_id)
            .await
            .expect("receiver should exist");

        let approve_handle = {
            let req_id = req_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                req_id
            })
        };

        let req_id_for_approve = approve_handle.await.unwrap();
        approve_tool_approval_inner(&approval_state, req_id_for_approve)
            .await
            .unwrap();

        let wait_res = wait_for_tool_approval_decision(
            &approval_state,
            &cancel_state,
            "turn-normal-approve",
            &req_id,
            receiver,
        )
        .await;

        match wait_res {
            Ok(ToolApprovalDecision::Approved) => {}
            other => panic!("expected approved decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_wait_for_tool_approval_decision_race_approved_but_cancelled_before_return() {
        let cancel_state = TurnCancellationState::new();
        cancel_state.register_turn("turn-race").await;
        let approval_state = PendingToolApprovalState::new();

        let req_id = approval_state
            .register(
                "turn-race".into(),
                "tool".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;
        let receiver = approval_state
            .take_receiver(&req_id)
            .await
            .expect("receiver should exist");

        // First approve the tool
        approve_tool_approval_inner(&approval_state, req_id.clone())
            .await
            .unwrap();

        // But immediately cancel the turn before waiting decision evaluates
        cancel_state
            .cancel_turn("turn-race", Some("racing_stop".into()))
            .await
            .unwrap();

        let wait_res = wait_for_tool_approval_decision(
            &approval_state,
            &cancel_state,
            "turn-race",
            &req_id,
            receiver,
        )
        .await;

        // Must reject execution with cancellation error despite receiving Approved!
        match wait_res {
            Err(KokoroError::Chat(msg)) => {
                assert_eq!(msg, TURN_CANCELLED_BY_USER_MESSAGE);
            }
            other => panic!("expected cancelled chat error on race, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_cancel_chat_turn_unblocks_tool_approval_wait() {
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state
            .register_turn_with_request("turn-unblock-test", Some("request-unblock"))
            .await;
        let approval_state = Arc::new(PendingToolApprovalState::new());

        let req_id = approval_state
            .register(
                "turn-unblock-test".into(),
                "tool".into(),
                "tool".into(),
                HashMap::new(),
            )
            .await;
        let receiver = approval_state
            .take_receiver(&req_id)
            .await
            .expect("receiver should exist");

        let mut waiting = Box::pin(wait_for_tool_approval_decision(
            &approval_state,
            &cancel_state,
            "turn-unblock-test",
            &req_id,
            receiver,
        ));
        assert!(futures::poll!(waiting.as_mut()).is_pending());

        // Hold cleanup at a deterministic boundary: the cancellation signal must
        // already be visible while the pending approval sender is still alive.
        let pending = approval_state.pending.lock().await;
        let mut cancelling = Box::pin(cancel_chat_turn_inner(
            "request-unblock".into(),
            None,
            cancel_state.clone(),
            &approval_state,
        ));
        assert!(futures::poll!(cancelling.as_mut()).is_pending());
        assert!(cancel_state.is_cancelled("turn-unblock-test").await);
        assert!(cancel_state.is_cancelled("request-unblock").await);
        assert!(
            ensure_turn_not_cancelled(&cancel_state, "turn-unblock-test")
                .await
                .is_err()
        );
        drop(pending);
        assert!(futures::poll!(cancelling.as_mut()).is_pending());
        let wait_res = waiting.await;

        match wait_res {
            Err(KokoroError::Chat(msg)) => {
                assert_eq!(msg, TURN_CANCELLED_BY_USER_MESSAGE);
            }
            other => panic!("expected cancelled error, got {other:?}"),
        }
        assert!(!approval_state.is_pending(&req_id).await);
        assert!(approve_tool_approval_inner(&approval_state, req_id)
            .await
            .is_err());
        cancel_state.clear_turn("turn-unblock-test").await;
        cancelling.await.unwrap();
    }

    #[tokio::test]
    async fn turn_cancellation_state_register_cancel_and_idempotent() {
        let state = TurnCancellationState::new();

        state.register_turn("turn-1").await;
        assert!(!state.is_cancelled("turn-1").await);

        assert!(state
            .cancel_turn("turn-1", Some("user".into()))
            .await
            .is_ok());
        assert!(state.is_cancelled("turn-1").await);

        assert!(state
            .cancel_turn("turn-1", Some("again".into()))
            .await
            .is_ok());
        assert!(state.is_cancelled("turn-1").await);
    }

    #[tokio::test]
    async fn cancelled_turn_stops_before_tool_execution() {
        let state = TurnCancellationState::new();
        state.register_turn("turn-1").await;
        state
            .cancel_turn("turn-1", Some("user".into()))
            .await
            .expect("cancel should succeed");

        let mut tool_execute_count = 0usize;
        let result = ensure_turn_not_cancelled(&state, "turn-1").await;
        if result.is_ok() {
            tool_execute_count += 1;
        }

        assert!(result.is_err());
        assert_eq!(tool_execute_count, 0);
    }

    #[tokio::test]
    async fn cancelled_turn_skips_delta_emit_payload_generation() {
        let state = TurnCancellationState::new();
        state.register_turn("turn-1").await;
        state
            .cancel_turn("turn-1", Some("user".into()))
            .await
            .expect("cancel should succeed");

        let payload =
            build_turn_delta_payload_if_not_cancelled(&state, "turn-1", "hello".into(), None).await;
        assert!(payload.is_err());
    }

    #[tokio::test]
    async fn cancel_chat_turn_returns_error_for_empty_target_id() {
        let state = Arc::new(TurnCancellationState::new());
        let result = cancel_chat_turn_inner(
            "   ".to_string(),
            Some("user".into()),
            state,
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn cancel_chat_turn_places_tombstone_and_inherits_on_registration() {
        let state = Arc::new(TurnCancellationState::new());

        // Cancel before registration via client_request_id
        let res = cancel_chat_turn_inner(
            "req-unborn-1".to_string(),
            Some("cancelled_before_registration".into()),
            state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(
            res.is_ok(),
            "Cancellation before registration must succeed via tombstone"
        );

        assert!(state.is_cancelled("req-unborn-1").await);
        assert!(state.has_turn("req-unborn-1").await);

        // Later, stream_chat reaches registration:
        state
            .register_turn_with_request("turn-born-1", Some("req-unborn-1"))
            .await;

        // Turn must immediately inherit cancelled state!
        assert!(state.is_cancelled("turn-born-1").await);
        assert!(state.is_cancelled("req-unborn-1").await);

        // Cleanup
        state.clear_turn("turn-born-1").await;
        assert!(!state.has_turn("turn-born-1").await);
        assert!(!state.has_turn("req-unborn-1").await);
    }

    #[tokio::test]
    async fn cancel_chat_turn_places_tombstone_by_turn_id_and_inherits() {
        let state = Arc::new(TurnCancellationState::new());

        // Cancel before registration via turn_id directly
        let res = cancel_chat_turn_inner(
            "turn-unborn-2".to_string(),
            Some("cancelled_early".into()),
            state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(res.is_ok());

        assert!(state.is_cancelled("turn-unborn-2").await);

        state.register_turn("turn-unborn-2").await;
        assert!(state.is_cancelled("turn-unborn-2").await);

        state.clear_turn("turn-unborn-2").await;
        assert!(!state.has_turn("turn-unborn-2").await);
    }

    #[tokio::test]
    async fn turn_cancellation_real_timing_concurrency() {
        let state = Arc::new(TurnCancellationState::new());
        let cancel_state = state.clone();

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let barrier_cancel = barrier.clone();

        let cancel_handle = tokio::spawn(async move {
            barrier_cancel.wait().await;
            cancel_chat_turn_inner(
                "req-concurrent-timing".to_string(),
                Some("stopped_by_user".into()),
                cancel_state,
                &PendingToolApprovalState::new(),
            )
            .await
        });

        let register_state = state.clone();
        let register_handle = tokio::spawn(async move {
            barrier.wait().await;
            // Introduce a small timing delay to let cancellation land first or concurrently
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            register_state
                .register_turn_with_request("turn-concurrent-timing", Some("req-concurrent-timing"))
                .await;
            register_state.is_cancelled("turn-concurrent-timing").await
        });

        let (cancel_res, was_cancelled) = tokio::join!(cancel_handle, register_handle);
        assert!(cancel_res.unwrap().is_ok());
        assert!(
            was_cancelled.unwrap(),
            "Turn must be cancelled upon registration"
        );
    }

    #[tokio::test]
    async fn turn_cancellation_tombstone_capacity_pruning() {
        let state = TurnCancellationState::new();
        // Insert 300 tombstones (exceeding MAX_TOMBSTONES = 256)
        for i in 0..300 {
            let _ = state
                .cancel_turn(&format!("req-overflow-{i}"), Some("prune".into()))
                .await;
        }

        let inner = state.inner.read().await;
        assert!(
            inner.tombstones.len() <= 256,
            "Tombstones map must be bounded to MAX_TOMBSTONES"
        );
        // Oldest entries (e.g. req-overflow-0) should have been evicted
        assert!(!inner.tombstones.contains_key("req-overflow-0"));
        // Newest entry must exist
        assert!(inner.tombstones.contains_key("req-overflow-299"));
    }

    #[tokio::test]
    async fn turn_cancellation_state_supports_client_request_id() {
        let state = Arc::new(TurnCancellationState::new());
        state
            .register_turn_with_request("turn-real-123", Some("req-client-456"))
            .await;

        assert!(!state.is_cancelled("turn-real-123").await);
        assert!(!state.is_cancelled("req-client-456").await);
        assert!(state.has_turn("turn-real-123").await);
        assert!(state.has_turn("req-client-456").await);

        // Cancel via client_request_id
        let res = cancel_chat_turn_inner(
            "req-client-456".to_string(),
            Some("new_conversation_started".into()),
            state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(res.is_ok());

        assert!(state.is_cancelled("turn-real-123").await);
        assert!(state.is_cancelled("req-client-456").await);

        // Clearing turn also cleans up request_to_turn mapping
        state.clear_turn("turn-real-123").await;
        assert!(!state.has_turn("turn-real-123").await);
        assert!(!state.has_turn("req-client-456").await);
    }

    #[test]
    fn chat_request_deserializes_regenerate_default_and_explicit() {
        let default_req: ChatRequest = serde_json::from_str(r#"{"message":"hi"}"#).unwrap();
        assert!(!default_req.regenerate);
        assert_eq!(default_req.conversation_id, None);

        let regen_req: ChatRequest = serde_json::from_str(
            r#"{"message":"hi","regenerate":true,"conversation_id":"conv-123"}"#,
        )
        .unwrap();
        assert!(regen_req.regenerate);
        assert_eq!(regen_req.conversation_id.as_deref(), Some("conv-123"));

        let null_conv_req: ChatRequest =
            serde_json::from_str(r#"{"message":"hi","conversation_id":null}"#).unwrap();
        assert_eq!(null_conv_req.conversation_id, None);

        let untrusted: ChatRequest = serde_json::from_value(serde_json::json!({
            "message": "hi", "previous_request_ids": ["injected-alias"],
        }))
        .unwrap();
        assert!(untrusted.previous_request_ids.is_empty());
    }

    #[test]
    fn test_chat_session_validation_logic() {
        let check = |req: Option<&str>,
                     init: Option<&str>,
                     curr: Option<&str>,
                     init_gen: u64,
                     curr_gen: u64|
         -> bool {
            let gen_valid = init_gen == curr_gen;
            let conv_valid = match (req, init, curr) {
                (Some(r), _, Some(c)) => r == c,
                (Some(_), _, None) => false,
                (None, Some(i), Some(c)) => i == c,
                (None, None, None) => true,
                _ => false,
            };
            gen_valid && conv_valid
        };

        // 1. Explicit target matches current active, generation unchanged
        assert!(check(Some("conv-A"), Some("conv-A"), Some("conv-A"), 1, 1));
        assert!(check(Some("conv-A"), None, Some("conv-A"), 1, 1));

        // 2. Explicit target with generation changed (ABA scenario: A -> B -> A)
        assert!(!check(Some("conv-A"), Some("conv-A"), Some("conv-A"), 1, 3));

        // 3. Explicit target differs from current active (switched to B)
        assert!(!check(Some("conv-A"), Some("conv-A"), Some("conv-B"), 1, 2));

        // 4. Explicit target cleared while in flight (clear_history)
        assert!(!check(Some("conv-A"), Some("conv-A"), None, 1, 2));

        // 5. Implicit target (new conversation), stays None -> valid new conversation
        assert!(check(None, None, None, 1, 1));

        // 6. Implicit target, but generation changed (None -> B -> None ABA scenario)
        assert!(!check(None, None, None, 1, 3));

        // 7. Implicit target (new conversation), but user clicked B while in flight -> invalid!
        assert!(!check(None, None, Some("conv-B"), 1, 2));

        // 8. Ambient caller without target, initial A, current A, unchanged generation -> valid
        assert!(check(None, Some("conv-A"), Some("conv-A"), 1, 1));

        // 9. Ambient caller without target, initial A, current A, generation changed (ABA) -> invalid!
        assert!(!check(None, Some("conv-A"), Some("conv-A"), 1, 3));

        // 10. Ambient caller without target, initial A, switched to B -> invalid!
        assert!(!check(None, Some("conv-A"), Some("conv-B"), 1, 2));

        // 11. Ambient caller without target, initial A, cleared -> invalid!
        assert!(!check(None, Some("conv-A"), None, 1, 2));
    }

    #[tokio::test]
    async fn draft_recovery_fences_late_admission_before_allowing_resend() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let runtime = crate::chat::runtime::ChatRuntimeState::default();
        let mut request: ChatRequest = serde_json::from_value(serde_json::json!({
            "message": "draft", "character_id": "char-a", "client_request_id": "closed-window"
        }))
        .unwrap();

        assert_eq!(
            recover_chat_draft_submission_inner(&state.db, &runtime, "char-a", "closed-window")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Unsent,
        );
        assert!(
            matches!(runtime.begin(&request), Err(error) if error.starts_with("chat_draft_abandoned:"))
        );
        assert!(runtime.snapshots().is_empty());

        request.client_request_id = Some("resend".into());
        assert!(runtime.begin(&request).unwrap().0);
        assert_eq!(
            recover_chat_draft_submission_inner(&state.db, &runtime, "char-a", "resend")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Pending,
        );
        assert!(!runtime.begin(&request).unwrap().0);
        runtime
            .complete("resend", Err(KokoroError::Chat("chat_turn_busy".into())))
            .unwrap();
        assert_eq!(
            recover_chat_draft_submission_inner(&state.db, &runtime, "char-a", "resend")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Unsent,
        );
    }

    #[tokio::test]
    async fn a_confirmed_user_save_failure_recovers_the_draft_without_sqlite() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let runtime = crate::chat::runtime::ChatRuntimeState::default();
        let request = serde_json::from_value(serde_json::json!({
            "message": "unsent input", "character_id": "owner", "client_request_id": "failed-save"
        }))
        .unwrap();
        runtime.begin(&request).unwrap();
        runtime.prepare_user_save_retry("failed-save", None);
        runtime.complete(
            "failed-save",
            Err(KokoroError::ChatPersistence("save fault".into())),
        );
        state.db.close().await;

        assert_eq!(
            recover_chat_draft_submission_inner(&state.db, &runtime, "owner", "failed-save")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Unsent,
        );
        assert!(recover_chat_draft_submission_inner(
            &state.db,
            &runtime,
            "other-owner",
            "failed-save"
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn draft_submission_recovery_uses_the_saved_request_after_runtime_restart() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let db = &state.db;
        let runtime = crate::chat::runtime::ChatRuntimeState::default();
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "message": "same text", "character_id": "char-a", "client_request_id": "draft-1"
        }))
        .unwrap();
        runtime.begin(&request).unwrap();
        assert_eq!(
            recover_chat_draft_submission_inner(db, &runtime, "char-a", "draft-1")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Pending,
        );

        sqlx::query("INSERT INTO conversations (id, character_id, created_at, updated_at) VALUES ('conv-a', 'char-a', 'now', 'now')")
            .execute(db).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-a', 'user', 'same text', 'legacy malformed metadata', 'now')")
            .execute(db).await.unwrap();
        state
            .add_message_with_metadata_for_conversation(
                "user".into(),
                request.message,
                Some(serde_json::json!({ "client_request_id": "draft-1" }).to_string()),
                "char-a",
                Some("conv-a"),
                None,
            )
            .await
            .unwrap();

        let restarted = crate::chat::runtime::ChatRuntimeState::default();
        assert_eq!(
            recover_chat_draft_submission_inner(db, &restarted, "char-a", "draft-1")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Persisted,
        );
        assert_eq!(
            recover_chat_draft_submission_inner(db, &restarted, "char-b", "draft-1")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Unsent,
        );
        assert_eq!(
            recover_chat_draft_submission_inner(db, &restarted, "char-a", "draft-2")
                .await
                .unwrap(),
            ChatDraftSubmissionStatus::Unsent,
        );
    }

    #[tokio::test]
    async fn test_conversation_state_snapshot_tracks_generation_and_id() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let (init_id, init_gen) = state.conversation_state_snapshot().await;
        assert_eq!(init_id, None);
        assert_eq!(init_gen, 0);

        *state.selection.current_conversation_id.lock().await = Some("conv-test".to_string());
        state.bump_conversation_generation();

        let (updated_id, updated_gen) = state.conversation_state_snapshot().await;
        assert_eq!(updated_id.as_deref(), Some("conv-test"));
        assert_eq!(updated_gen, 1);
    }

    #[test]
    fn should_insert_user_message_skips_when_hidden() {
        assert!(!should_insert_user_message_for_request(true, false, false));
        assert!(!should_insert_user_message_for_request(true, true, false));
        assert!(!should_insert_user_message_for_request(true, true, true));
    }

    #[test]
    fn should_insert_user_message_always_inserts_for_normal_non_hidden_turn() {
        assert!(should_insert_user_message_for_request(false, false, false));
        assert!(should_insert_user_message_for_request(false, false, true));
    }

    #[test]
    fn should_insert_user_message_skips_when_regenerating_and_history_has_trailing_user() {
        // Normal regeneration: trailing user message already in history -> skip duplicate insertion
        assert!(!should_insert_user_message_for_request(false, true, true));
    }

    #[test]
    fn should_insert_user_message_heals_when_regenerating_but_history_lacks_trailing_user() {
        // Defensive self-healing: if history was truncated/empty, fallback to inserting
        assert!(should_insert_user_message_for_request(false, true, false));
    }

    async fn setup_test_chat_db() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                character_id TEXT NOT NULL,
                title TEXT NOT NULL,
                topic TEXT NOT NULL DEFAULT '',
                pinned_state TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE conversation_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn test_validate_conversation_character_id_rejects_mismatch_without_identifiers() {
        let pool = setup_test_chat_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
             VALUES ('conv-foreign', 'char-owner', 'Test', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        let error = validate_conversation_character_id(&pool, "conv-foreign", "char-request")
            .await
            .expect_err("a conversation owned by another character must be rejected");

        match error {
            KokoroError::Validation(message) => {
                assert_eq!(message, "Conversation is not available for this character");
                assert!(!message.contains("conv-foreign"));
                assert!(!message.contains("char-owner"));
                assert!(!message.contains("char-request"));
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_resolve_trailing_visible_user_message_with_tools() {
        let pool = setup_test_chat_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-tools', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 1. User message (id 1)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'user', 'What is the weather?', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 2. Technical tool call message (id 2)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'assistant', 'call weather', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 3. Technical tool result message (id 3)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'tool', '{\"temperature\": 25}', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 4. Final assistant answer (id 4)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'assistant', 'The weather is 25C sunny.', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // When assistant answer is present, trailing visible message is assistant (NOT user)
        let trailing_before = resolve_trailing_visible_user_message(&pool, "conv-tools")
            .await
            .unwrap();
        assert!(
            trailing_before.is_none(),
            "When assistant message is at the end, trailing visible user message must be None"
        );

        // Delete the trailing visible assistant turn using the real delete_last_messages command logic.
        // It must automatically remove id 4 (assistant) and its preceding technical messages (ids 3, 2).
        let history =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new()));
        let current_conv =
            std::sync::Arc::new(tokio::sync::Mutex::new(Some("conv-tools".to_string())));
        let switch_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        crate::commands::context::delete_last_messages_inner(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            None,
            &switch_lock,
        )
        .await
        .unwrap();

        // Verify that id 2, 3, 4 are truly deleted from database
        let remaining_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-tools' ORDER BY id ASC"
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids, vec![1]);

        // Now, the trailing visible message should be the user message (id 1)
        let trailing_after = resolve_trailing_visible_user_message(&pool, "conv-tools")
            .await
            .unwrap();
        assert_eq!(
            trailing_after,
            Some((1, "What is the weather?".to_string()))
        );

        // Verify deduplication decision
        assert!(!should_insert_user_message_for_request(
            false,
            true,
            trailing_after.is_some()
        ));
    }

    #[tokio::test]
    async fn test_resolve_trailing_visible_user_message_in_long_conversation() {
        let pool = setup_test_chat_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-long', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Populate 25 turns (50 messages: 25 users + 25 assistants)
        for i in 1..=25 {
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-long', 'user', ?, NULL, ?)")
                .bind(format!("User question {}", i))
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();

            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-long', 'assistant', ?, NULL, ?)")
                .bind(format!("Assistant reply {}", i))
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        // Delete the last assistant reply (turn 25)
        sqlx::query("DELETE FROM conversation_messages WHERE id = 50")
            .execute(&pool)
            .await
            .unwrap();

        // Check trailing visible user message
        let trailing = resolve_trailing_visible_user_message(&pool, "conv-long")
            .await
            .unwrap();
        assert!(trailing.is_some());
        let (id, content) = trailing.unwrap();
        assert_eq!(id, 49);
        assert_eq!(content, "User question 25");
        assert!(!should_insert_user_message_for_request(false, true, true));
    }

    #[test]
    fn test_ensure_client_request_id_assigns_unique_id_when_missing_or_blank() {
        let mut none_id: Option<String> = None;
        ensure_client_request_id(&mut none_id);
        assert!(none_id.is_some());
        assert!(none_id.as_ref().unwrap().starts_with("req_backend_"));

        let mut empty_id = Some("   ".to_string());
        ensure_client_request_id(&mut empty_id);
        assert!(empty_id.is_some());
        assert!(empty_id.as_ref().unwrap().starts_with("req_backend_"));

        let mut custom_id = Some("client-req-999".to_string());
        ensure_client_request_id(&mut custom_id);
        assert_eq!(custom_id.as_deref(), Some("client-req-999"));
    }

    #[test]
    fn test_resolve_turn_user_message_id_omits_for_hidden_turns() {
        // Hidden turns (pet poke, proactive, background) must never expose or align a user_message_id
        assert_eq!(resolve_turn_user_message_id(true, Some(42)), None);
        assert_eq!(resolve_turn_user_message_id(true, None), None);

        // Non-hidden turns (e.g. regenerate) correctly reflect trailing user id
        assert_eq!(resolve_turn_user_message_id(false, Some(42)), Some(42));
        assert_eq!(resolve_turn_user_message_id(false, None), None);
    }

    #[tokio::test]
    async fn test_poll_stream_with_cancellation_notified_immediately() {
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-cancel-test").await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-cancel-test")
            .await;

        let mut stream = futures::stream::pending::<Result<LlmStreamEvent, String>>();

        // Trigger cancellation
        cancel_state
            .cancel_turn("turn-cancel-test", Some("user_abort".to_string()))
            .await
            .unwrap();

        let res = poll_stream_with_cancellation_and_timeout(
            &mut stream,
            &mut cancel_rx,
            std::time::Duration::from_secs(10),
        )
        .await;

        assert!(matches!(res, StreamPollResult::Cancelled));
    }

    #[tokio::test]
    async fn test_poll_stream_with_timeout_when_stream_hangs() {
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-timeout-test").await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-timeout-test")
            .await;

        let mut stream = futures::stream::pending::<Result<LlmStreamEvent, String>>();

        let res = poll_stream_with_cancellation_and_timeout(
            &mut stream,
            &mut cancel_rx,
            std::time::Duration::from_millis(50),
        )
        .await;

        assert!(matches!(res, StreamPollResult::TimedOut(_)));
    }

    #[tokio::test]
    async fn test_hung_provider_stream_cancelled_and_reacquired_successfully() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-hung-1").await;

        assert!(!orchestrator.is_chat_busy());

        // Turn 1 acquires lock
        let guard1 = orchestrator
            .try_acquire_chat_turn("req-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent Turn 2 while turn 1 is active must fail with chat_turn_busy
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-2")
            .await
            .expect_err("turn 2 must be rejected while turn 1 is active");
        assert!(busy_err.contains("chat_turn_busy"));

        // Simulate turn 1 consuming a hung stream in an async task
        let cancel_state_clone = Arc::clone(&cancel_state);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut cancel_rx = cancel_state_clone
                .subscribe_cancellation("turn-hung-1")
                .await;
            let mut stream = futures::stream::pending::<Result<LlmStreamEvent, String>>();
            let _turn_guard =
                TurnCancellationGuard::new(cancel_state_clone.clone(), "turn-hung-1".to_string());
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Blocked waiting for hung stream or cancellation
            let poll_res = poll_stream_with_cancellation_and_timeout(
                &mut stream,
                &mut cancel_rx,
                std::time::Duration::from_secs(60),
            )
            .await;

            assert!(matches!(poll_res, StreamPollResult::Cancelled));
            // _execution_guard and _turn_guard drop upon return
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        // Simulate cancel_chat_turn_inner (e.g. user clicked Stop)
        let cancel_res = cancel_chat_turn_inner(
            "turn-hung-1".to_string(),
            Some("user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok(), "cancellation should succeed");

        // The background task exits promptly
        handle.await.expect("background task finished");

        // Global lock must be released!
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after cancellation"
        );

        // Turn 2 can now acquire lock and proceed without chat_turn_busy!
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-2")
            .await
            .expect("turn 2 must succeed after turn 1 was cancelled");
        assert_eq!(guard2.context().unwrap().client_request_id, "req-2");
        assert!(orchestrator.is_chat_busy());
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_provider_stream_times_out_and_releases_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-timeout-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent Turn 2 is rejected
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-timeout-2")
            .await
            .expect_err("must be rejected while busy");
        assert!(busy_err.contains("chat_turn_busy"));

        // Simulate hung stream that hits timeout
        let cancel_state_clone = Arc::clone(&cancel_state);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut cancel_rx = cancel_state_clone
                .subscribe_cancellation("turn-timeout-1")
                .await;
            let mut stream = futures::stream::pending::<Result<LlmStreamEvent, String>>();
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-timeout-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let poll_res = poll_stream_with_cancellation_and_timeout(
                &mut stream,
                &mut cancel_rx,
                std::time::Duration::from_millis(50),
            )
            .await;

            assert!(matches!(poll_res, StreamPollResult::TimedOut(_)));
            // Drops _execution_guard upon task exit
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        // Lock released automatically after timeout
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after timeout"
        );

        // Subsequent request succeeds
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-timeout-2")
            .await
            .expect("subsequent request must acquire lock after timeout");
        assert_eq!(guard2.context().unwrap().client_request_id, "req-timeout-2");
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[test]
    fn test_evaluate_round_stream_outcome_timeout_after_partial_output() {
        let termination = RoundStreamTermination::TimedOut("chunk idle timeout".to_string());
        let round_response = "Here is some partial answer before network stalled...";
        let native_tool_calls = vec![];

        let decision =
            evaluate_round_stream_outcome(termination, round_response, native_tool_calls);
        match decision {
            RoundExecutionDecision::TerminateTimedOut {
                err_msg,
                partial_text,
            } => {
                assert_eq!(err_msg, "chunk idle timeout");
                assert_eq!(
                    partial_text,
                    "Here is some partial answer before network stalled..."
                );
            }
            other => panic!("expected TerminateTimedOut, got {:?}", other),
        }
    }

    #[test]
    fn test_evaluate_round_stream_outcome_timeout_after_tool_call_suppresses_tools() {
        // Even if native tool calls were received before timeout occurred, tool execution MUST be suppressed
        let termination = RoundStreamTermination::TimedOut("timeout after 30s".to_string());
        let round_response = "";
        let native_tool_calls = vec![ToolCall {
            tool_call_id: Some("call_1".to_string()),
            name: "execute_dangerous_action".to_string(),
            args: HashMap::from([("action".to_string(), "delete".to_string())]),
        }];

        let decision =
            evaluate_round_stream_outcome(termination, round_response, native_tool_calls);
        match decision {
            RoundExecutionDecision::TerminateTimedOut {
                err_msg,
                partial_text,
            } => {
                assert_eq!(err_msg, "timeout after 30s");
                assert!(partial_text.is_empty());
            }
            other => panic!("expected TerminateTimedOut, got {:?}", other),
        }
    }

    #[test]
    fn test_evaluate_round_stream_outcome_timeout_after_textual_tool_call_tag_suppresses_tools() {
        // If the model produced a prompt-mode tool call tag, timeout must suppress tool execution
        let termination = RoundStreamTermination::TimedOut("timeout after 15s".to_string());
        let round_response = "Checking... [TOOL_CALL:execute_dangerous_action|action=delete]";
        let native_tool_calls = vec![];

        let decision =
            evaluate_round_stream_outcome(termination, round_response, native_tool_calls);
        match decision {
            RoundExecutionDecision::TerminateTimedOut {
                err_msg,
                partial_text,
            } => {
                assert_eq!(err_msg, "timeout after 15s");
                // Leaked tag should be stripped from partial text
                assert_eq!(partial_text, "Checking...");
            }
            other => panic!("expected TerminateTimedOut, got {:?}", other),
        }
    }

    #[test]
    fn test_evaluate_round_stream_outcome_completed_with_tools_executes() {
        let termination = RoundStreamTermination::Completed;
        let round_response = "I will check";
        let native_tool_calls = vec![ToolCall {
            tool_call_id: Some("call_2".to_string()),
            name: "get_weather".to_string(),
            args: HashMap::new(),
        }];

        let decision =
            evaluate_round_stream_outcome(termination, round_response, native_tool_calls);
        match decision {
            RoundExecutionDecision::ExecuteTools {
                tool_calls,
                cleaned_text,
            } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "get_weather");
                assert_eq!(cleaned_text, "I will check");
            }
            other => panic!("expected ExecuteTools, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_turn_artifacts_cleanup_on_timeout_with_prior_round_tool_messages() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();

        // Setup a conversation
        let cid = "conv-timeout-cleanup-test";
        sqlx::query(
            "INSERT INTO conversations (id, character_id, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))"
        )
        .bind(cid)
        .bind("test_char")
        .execute(&state.db)
        .await
        .unwrap();

        let turn_id = "turn-timeout-prior-tools";

        // Simulate Round 1: persisted an assistant_tool_calls row and a tool_result row
        let tool_meta = serde_json::json!({
            "type": "assistant_tool_calls",
            "turn_id": turn_id,
        })
        .to_string();
        state
            .add_message_with_metadata_for_conversation(
                "assistant".to_string(),
                "Let me run a tool".to_string(),
                Some(tool_meta),
                "test_char",
                Some(cid),
                None,
            )
            .await
            .unwrap();

        let result_meta = serde_json::json!({
            "type": "tool_result",
            "turn_id": turn_id,
        })
        .to_string();
        state
            .add_message_with_metadata_for_conversation(
                "tool".to_string(),
                "tool output".to_string(),
                Some(result_meta),
                "test_char",
                Some(cid),
                None,
            )
            .await
            .unwrap();

        // And a draft row representing Round 2's partial text before timeout
        let draft_id = state
            .persist_streaming_draft(cid, "Partial text in round 2 before timeout...", turn_id)
            .await
            .unwrap();

        let rows_before: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, content FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
        )
        .bind(cid)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(rows_before.len(), 3);

        // When Round 2 times out, cleanup_turn_artifacts cleans technical rows but keeps partial draft
        cleanup_turn_artifacts(&state, cid, turn_id, None).await;

        let rows_after: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, content FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
        )
        .bind(cid)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(rows_after.len(), 1);
        assert_eq!(rows_after[0].0, draft_id);
        assert_eq!(rows_after[0].1, "Partial text in round 2 before timeout...");
    }

    #[tokio::test]
    async fn test_hung_tool_execution_cancelled_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-tool-cancel-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-tool-cancel-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent Turn 2 is rejected
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-tool-cancel-2")
            .await
            .expect_err("must be rejected while busy");
        assert!(busy_err.contains("chat_turn_busy"));

        // Simulate hung tool execution guarded by cancellation listener
        let cancel_state_clone = Arc::clone(&cancel_state);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut cancel_rx = cancel_state_clone
                .subscribe_cancellation("turn-tool-cancel-1")
                .await;
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-tool-cancel-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Hanging tool execution simulated as long async work
            let tool_execution_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                "tool_done"
            };

            let outcome = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                res = tool_execution_fut => {
                    Ok(res)
                }
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
            // _execution_guard and _turn_guard dropped upon task exit
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        // Simulate user clicking Stop
        let cancel_res = cancel_chat_turn_inner(
            "turn-tool-cancel-1".to_string(),
            Some("user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok(), "cancellation should succeed");

        // The background tool execution task exits promptly
        handle.await.expect("background task finished");

        // Global turn lock must be released!
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after tool cancellation"
        );

        // Subsequent chat request succeeds immediately without chat_turn_busy!
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-tool-cancel-2")
            .await
            .expect("turn 2 must succeed after tool execution was cancelled");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-tool-cancel-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_after_action_hook_cancelled_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state
            .register_turn("turn-after-action-cancel-1")
            .await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-after-action-cancel-1")
            .await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-after-action-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let cancel_state_clone = cancel_state.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-after-action-cancel-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let hook_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), String>(())
            };

            let outcome: Result<(), KokoroError> = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                res = hook_fut => {
                    res.map_err(|e| KokoroError::Chat(e))
                }
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        let cancel_res = cancel_chat_turn_inner(
            "turn-after-action-cancel-1".to_string(),
            Some("user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok());

        handle.await.expect("task finished");

        assert!(
            !orchestrator.is_chat_busy(),
            "lock must be released after after-action hook cancellation"
        );

        let guard2 = orchestrator
            .try_acquire_chat_turn("req-after-action-2")
            .await
            .expect("turn 2 must succeed after after-action hook was cancelled");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-after-action-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_tool_execution_times_out_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-tool-timeout-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-tool-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent Turn 2 is rejected
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-tool-timeout-2")
            .await
            .expect_err("must be rejected while busy");
        assert!(busy_err.contains("chat_turn_busy"));

        // Simulate hung tool execution that hits timeout
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let timeout_duration = std::time::Duration::from_millis(50);
            let hanging_tool = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                "ok"
            };

            let res = tokio::time::timeout(timeout_duration, hanging_tool).await;
            assert!(res.is_err(), "tool execution must hit timeout");
            // _execution_guard dropped upon task exit
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        // Lock released automatically after timeout
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after tool timeout"
        );

        // Subsequent request succeeds
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-tool-timeout-2")
            .await
            .expect("subsequent request must acquire lock after tool timeout");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-tool-timeout-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_approved_tool_execution_timeout_returns_error() {
        let timeout_duration = std::time::Duration::from_millis(50);
        let hanging_tool = async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<crate::actions::ActionResult, String>(sample_action_result("done"))
        };

        let result = match tokio::time::timeout(timeout_duration, hanging_tool).await {
            Ok(exec_res) => exec_res,
            Err(_) => Err(format!(
                "Tool 'custom_script' execution timed out after {}s",
                timeout_duration.as_secs()
            )),
        };

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Tool 'custom_script' execution timed out"));
    }

    #[tokio::test]
    async fn test_approved_tool_execution_cancelled_while_running() {
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-approved-cancel").await;
        let mut tool_cancel_rx = cancel_state
            .subscribe_cancellation("turn-approved-cancel")
            .await;

        let cancel_state_clone = Arc::clone(&cancel_state);
        let cancel_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            cancel_state_clone
                .cancel_turn("turn-approved-cancel", Some("stop".into()))
                .await
        });

        let timeout_duration = std::time::Duration::from_secs(10);
        let hanging_tool = async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<crate::actions::ActionResult, String>(sample_action_result("done"))
        };

        let outcome: Result<Result<crate::actions::ActionResult, String>, KokoroError> = tokio::select! {
            biased;
            _ = wait_for_cancel_event(&mut tool_cancel_rx) => {
                Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
            }
            res = tokio::time::timeout(timeout_duration, hanging_tool) => {
                match res {
                    Ok(exec_res) => Ok(exec_res),
                    Err(_) => Err(KokoroError::Chat("timeout".to_string())),
                }
            }
        };

        cancel_handle.await.unwrap().unwrap();
        match outcome {
            Err(KokoroError::Chat(msg)) => assert_eq!(msg, TURN_CANCELLED_BY_USER_MESSAGE),
            other => panic!("expected cancelled chat error, got {other:?}"),
        }
    }

    #[test]
    fn test_tool_execution_timeout_default_and_env() {
        // Without env var, defaults to 60s
        let default_timeout = tool_execution_timeout();
        assert!(default_timeout.as_secs() >= 60);
    }

    #[tokio::test]
    async fn test_hung_prompt_composition_times_out_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-prompt-timeout-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-prompt-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent request rejected while turn 1 is active
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-prompt-timeout-2")
            .await
            .expect_err("must be rejected while busy");
        assert!(busy_err.contains("chat_turn_busy"));

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let compose_prompt_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), String>(())
            };
            let timeout_duration = std::time::Duration::from_millis(50);

            let outcome: Result<(), KokoroError> = tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {
                    Err(KokoroError::Chat("Prompt composition timed out after 50ms".to_string()))
                }
                res = compose_prompt_fut => {
                    res.map_err(KokoroError::Chat)
                }
            };

            assert!(outcome.is_err());
            assert!(outcome
                .unwrap_err()
                .to_string()
                .contains("Prompt composition timed out"));
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        // Lock released automatically when guard dropped on error/timeout
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after prompt composition timeout"
        );

        // Turn 2 succeeds immediately
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-prompt-timeout-2")
            .await
            .expect("turn 2 must succeed after prompt composition timed out");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-prompt-timeout-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_prompt_composition_cancelled_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-prompt-cancel-1").await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-prompt-cancel-1")
            .await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-prompt-cancel-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let cancel_state_clone = cancel_state.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-prompt-cancel-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let compose_prompt_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), String>(())
            };

            let outcome: Result<(), KokoroError> = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                res = compose_prompt_fut => {
                    res.map_err(KokoroError::Chat)
                }
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        let cancel_res = cancel_chat_turn_inner(
            "turn-prompt-cancel-1".to_string(),
            Some("user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok());

        handle.await.expect("task finished");

        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after prompt composition cancellation"
        );

        let guard2 = orchestrator
            .try_acquire_chat_turn("req-prompt-cancel-2")
            .await
            .expect("turn 2 must succeed after prompt composition was cancelled");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-prompt-cancel-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_before_llm_modify_hook_times_out_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-hook-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let hook_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), String>(())
            };
            let timeout_duration = std::time::Duration::from_millis(50);

            let outcome: Result<(), KokoroError> = tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {
                    Err(KokoroError::Chat("BeforeLlmRequest hook timed out after 50ms".to_string()))
                }
                res = hook_fut => {
                    res.map_err(KokoroError::Chat)
                }
            };

            assert!(outcome.is_err());
            assert!(outcome
                .unwrap_err()
                .to_string()
                .contains("BeforeLlmRequest hook timed out"));
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after hook timeout"
        );

        let guard2 = orchestrator
            .try_acquire_chat_turn("req-hook-timeout-2")
            .await
            .expect("turn 2 must acquire lock after hook timeout");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-hook-timeout-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_best_effort_hook_times_out_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-best-effort-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let hook_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            };
            let timeout_duration = std::time::Duration::from_millis(50);

            tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {}
                _ = hook_fut => {}
            }
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released when turn finishes despite best-effort hook timeout"
        );

        let guard2 = orchestrator
            .try_acquire_chat_turn("req-best-effort-2")
            .await
            .expect("turn 2 must acquire lock after previous turn completed");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-best-effort-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_fallback_translation_times_out_and_allows_subsequent_chat_turn() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-fallback-timeout-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-fallback-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Concurrent Turn 2 while turn 1 is active must fail with chat_turn_busy
        let busy_err = orchestrator
            .try_acquire_chat_turn("req-fallback-timeout-2")
            .await
            .expect_err("turn 2 must be rejected while turn 1 is active");
        assert!(busy_err.contains("chat_turn_busy"));

        // Simulate turn 1 in post-text fallback stage where fallback translation provider hangs
        let cancel_state_clone = Arc::clone(&cancel_state);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut cancel_rx = cancel_state_clone
                .subscribe_cancellation("turn-fallback-timeout-1")
                .await;
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Simulate hung fallback translation provider
            let fallback_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<String, String>("fake translation".to_string())
            };
            let timeout_duration = std::time::Duration::from_millis(50);

            let fallback_res = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    None
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    None
                }
                res = fallback_fut => Some(res),
            };

            // Fallback timed out, translation is None, but turn execution continues and completes normally!
            assert!(fallback_res.is_none());
            // _execution_guard drops when turn completes upon return
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        // Global chat turn lock must be released when turn finishes despite fallback hang!
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after hung fallback times out"
        );

        // Subsequent chat request can now acquire lock and proceed without chat_turn_busy!
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-fallback-timeout-2")
            .await
            .expect("turn 2 must succeed after turn 1 fallback timeout");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-fallback-timeout-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_fallback_translation_cancelled_releases_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state.register_turn("turn-fallback-cancel-1").await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-fallback-cancel-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        // Simulate turn 1 hanging in fallback translation and then user/external cancel is issued
        let cancel_state_clone = Arc::clone(&cancel_state);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut cancel_rx = cancel_state_clone
                .subscribe_cancellation("turn-fallback-cancel-1")
                .await;
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            let fallback_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<String, String>("delayed translation".to_string())
            };
            let timeout_duration = std::time::Duration::from_secs(15);

            let outcome: Result<Option<Result<String, String>>, KokoroError> = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    Ok(None)
                }
                res = fallback_fut => Ok(Some(res)),
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        // User or external system requests cancellation (e.g. New Chat or Stop)
        let cancel_res = cancel_chat_turn_inner(
            "turn-fallback-cancel-1".to_string(),
            Some("new_chat_or_user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok(), "cancellation should succeed");

        handle.await.expect("task finished");

        // Lock must be released promptly!
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after cancellation during fallback"
        );

        // Subsequent request succeeds immediately!
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-fallback-cancel-2")
            .await
            .expect("subsequent turn must acquire lock after fallback cancellation");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-fallback-cancel-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_system_provider_in_preparation_cancelled_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state
            .register_turn("turn-prep-sysprov-cancel-1")
            .await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-prep-sysprov-cancel-1")
            .await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-prep-sysprov-cancel-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let cancel_state_clone = cancel_state.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-prep-sysprov-cancel-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Simulate hung system_provider() during preparation
            let prep_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), KokoroError>(())
            };
            let timeout_duration = std::time::Duration::from_secs(15);

            let outcome: Result<(), KokoroError> = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    Err(KokoroError::Chat("Turn preparation timed out after 15s".to_string()))
                }
                res = prep_fut => res,
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        // Cancel the turn while stuck in preparation
        let cancel_res = cancel_chat_turn_inner(
            "turn-prep-sysprov-cancel-1".to_string(),
            Some("user_stop".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok());

        handle.await.expect("task finished");

        // Turn lock must be immediately released!
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after preparation cancellation"
        );

        // Turn 2 succeeds immediately without chat_turn_busy
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-prep-sysprov-cancel-2")
            .await
            .expect("turn 2 must succeed after preparation was cancelled");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-prep-sysprov-cancel-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_system_provider_in_preparation_times_out_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-prep-timeout-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Simulate hung system_provider() during preparation
            let prep_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), KokoroError>(())
            };
            let timeout_duration = std::time::Duration::from_millis(50);

            let outcome: Result<(), KokoroError> = tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {
                    Err(KokoroError::Chat("Turn preparation timed out after 50ms".to_string()))
                }
                res = prep_fut => res,
            };

            assert!(outcome.is_err());
            assert!(outcome
                .unwrap_err()
                .to_string()
                .contains("Turn preparation timed out"));
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        handle.await.expect("task finished");

        // Lock must be released when guard drops on timeout
        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after preparation timeout"
        );

        // Turn 2 succeeds immediately
        let guard2 = orchestrator
            .try_acquire_chat_turn("req-prep-timeout-2")
            .await
            .expect("turn 2 must succeed after preparation timed out");
        assert_eq!(
            guard2.context().unwrap().client_request_id,
            "req-prep-timeout-2"
        );
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }

    #[tokio::test]
    async fn test_hung_conversation_lock_in_preparation_cancelled_and_releases_turn_lock() {
        let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        let cancel_state = Arc::new(TurnCancellationState::new());
        cancel_state
            .register_turn("turn-prep-convlock-cancel-1")
            .await;
        let mut cancel_rx = cancel_state
            .subscribe_cancellation("turn-prep-convlock-cancel-1")
            .await;

        assert!(!orchestrator.is_chat_busy());

        let guard1 = orchestrator
            .try_acquire_chat_turn("req-prep-convlock-cancel-1")
            .await
            .expect("turn 1 should acquire lock");
        assert!(orchestrator.is_chat_busy());

        let cancel_state_clone = cancel_state.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _turn_guard = TurnCancellationGuard::new(
                cancel_state_clone.clone(),
                "turn-prep-convlock-cancel-1".to_string(),
            );
            let _execution_guard = guard1;
            let _ = started_tx.send(());

            // Simulate hung conversation_switch_lock.lock()
            let prep_fut = async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<(), KokoroError>(())
            };
            let timeout_duration = std::time::Duration::from_secs(15);

            let outcome: Result<(), KokoroError> = tokio::select! {
                biased;
                _ = wait_for_cancel_event(&mut cancel_rx) => {
                    Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.to_string()))
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    Err(KokoroError::Chat("Turn preparation timed out after 15s".to_string()))
                }
                res = prep_fut => res,
            };

            assert!(
                matches!(outcome, Err(KokoroError::Chat(ref msg)) if msg == TURN_CANCELLED_BY_USER_MESSAGE)
            );
        });

        started_rx.await.expect("task started");
        assert!(orchestrator.is_chat_busy());

        let cancel_res = cancel_chat_turn_inner(
            "turn-prep-convlock-cancel-1".to_string(),
            Some("new_chat".to_string()),
            cancel_state.clone(),
            &PendingToolApprovalState::new(),
        )
        .await;
        assert!(cancel_res.is_ok());

        handle.await.expect("task finished");

        assert!(
            !orchestrator.is_chat_busy(),
            "chat turn lock must be released after conversation lock hang is cancelled"
        );

        let guard2 = orchestrator
            .try_acquire_chat_turn("req-prep-convlock-cancel-2")
            .await
            .expect("turn 2 must succeed after turn 1 cancelled");
        drop(guard2);
        assert!(!orchestrator.is_chat_busy());
    }
}
