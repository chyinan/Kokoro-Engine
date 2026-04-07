// pattern: Mixed (needs refactoring)
// Reason: 该命令文件同时承担 Tauri IPC 编排、流式对话副作用与少量 payload 整形；本次只在现有边界内最小接入 BeforeLlmRequest modify。
use crate::actions::executor::{
    apply_before_action_args_payload, assistant_tool_call_metadata_value,
    build_action_hook_payload, build_before_action_args_payload, tool_metadata_value,
};
use crate::actions::tool_settings::ToolSettings;
use crate::actions::{
    build_tool_audit_event, builtin_tool_id, execute_tool_calls, ActionContext, ActionRegistry,
    ActionResult, PermissionDecision, ToolAuditInput, ToolInvocation,
};
use crate::ai::context::AIOrchestrator;
use crate::ai::context::Message;
use crate::ai::memory_extractor;
use crate::commands::system::WindowSizeState;
use crate::error::KokoroError;
use crate::hooks::{
    BeforeLlmRequestMessage, BeforeLlmRequestPayload, ChatHookPayload, HookEvent, HookPayload,
    HookRuntime,
};
use crate::imagegen::ImageGenService;
use crate::llm::messages::{
    assistant_tool_calls_message, extract_message_text, history_message_to_chat_message,
    replace_user_message_with_images, system_message, tool_result_message, user_text_message,
};
use crate::llm::provider::LlmStreamEvent;
use crate::llm::service::LlmService;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{command, Emitter, Manager, State, Window};
use tokio::sync::{oneshot, Mutex, RwLock};
use uuid::Uuid;

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

pub struct PendingToolApprovalState {
    pending: Mutex<HashMap<String, PendingToolApproval>>,
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
        })
    }
}

async fn approve_tool_approval_inner(
    approval_state: &PendingToolApprovalState,
    approval_request_id: String,
) -> Result<(), KokoroError> {
    approval_state
        .resolve(&approval_request_id, ToolApprovalDecision::Approved)
        .await
}

async fn reject_tool_approval_inner(
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
    let app_data = dirs_next::data_dir()
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

#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub allow_image_gen: Option<bool>,
    pub images: Option<Vec<String>>,
    pub character_id: Option<String>,
    /// If true, neither the user message nor the assistant response is saved to history.
    /// Used for touch interactions and proactive triggers where the instruction shouldn't appear in chat.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Serialize, Clone)]
#[allow(dead_code)]
struct ChatImageGenEvent {
    prompt: String,
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
) -> Result<
    (
        String,
        Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
    ),
    String,
> {
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
            history_message_to_chat_message(&message.role, message.content, metadata)
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
) -> Result<
    (
        String,
        Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
    ),
    String,
> {
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

const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";
const TRANSLATE_TAG_PREFIX: &str = "[TRANSLATE:";

/// Tag prefixes that should be buffered (not emitted to frontend mid-stream).
const BUFFERED_TAG_PREFIXES: &[&str] = &[TOOL_CALL_TAG_PREFIX, TRANSLATE_TAG_PREFIX];

/// Returns the byte position up to which it's safe to emit text to the frontend.
/// Holds back any suffix that could be the start of a known tag prefix.
fn find_safe_emit_boundary(text: &str) -> usize {
    if let Some(last_bracket) = text.rfind('[') {
        let suffix = &text[last_bracket..];
        for prefix in BUFFERED_TAG_PREFIXES {
            if suffix.len() < prefix.len() {
                // Partial match — could still become a full tag
                if prefix.starts_with(suffix) {
                    return last_bracket;
                }
            } else if suffix.starts_with(prefix) {
                // Full prefix match — definitely a tag, hold it
                return last_bracket;
            }
        }
    }
    text.len()
}

/// Strip any `<tool_result>...</tool_result>` blocks or stray tags that the LLM may echo back.
fn strip_leaked_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Remove <tool_result>...</tool_result> blocks (greedy within single block)
    while let Some(start) = result.find("<tool_result>") {
        if let Some(end) = result[start..].find("</tool_result>") {
            let tag_end = start + end + "</tool_result>".len();
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — remove from <tool_result> to end of line
            let line_end = result[start..]
                .find('\n')
                .map(|i| start + i)
                .unwrap_or(result.len());
            result = format!("{}{}", result[..start].trim_end(), &result[line_end..]);
        }
    }
    result.trim().to_string()
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

fn deny_kind_for_outcome(outcome: &crate::actions::ToolExecutionOutcome, error: &str) -> &'static str {
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

fn emit_tool_trace_event(
    app: &tauri::AppHandle,
    turn_id: &str,
    outcome: &crate::actions::ToolExecutionOutcome,
) {
    match &outcome.result {
        Ok(result) => {
            let _ = app.emit(
                "chat-turn-tool",
                tool_success_payload(outcome, turn_id, result),
            );
        }
        Err(error) => {
            let _ = app.emit(
                "chat-turn-tool",
                tool_error_payload(outcome, turn_id, error),
            );
        }
    }
}

async fn execute_single_tool_after_approval(
    app: &tauri::AppHandle,
    registry_state: &std::sync::Arc<RwLock<ActionRegistry>>,
    character_id: &str,
    tool_call: &ToolInvocation,
) -> Result<ActionResult, String> {
    let hook_runtime = app.try_state::<HookRuntime>();
    let resolved = {
        let registry = registry_state.read().await;
        registry.resolve_action_for_execution(&tool_call.name)
    };
    let (action, handler) = resolved.map_err(|error| error.0.clone())?;
    let mut args_payload = build_before_action_args_payload(
        None,
        character_id,
        Some("chat".to_string()),
        tool_call,
        &action,
    );
    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_before_action_args_modify(&mut args_payload)
            .await;
    }
    let effective_args = apply_before_action_args_payload(args_payload);
    let ctx = ActionContext {
        app: app.clone(),
        character_id: character_id.to_string(),
        conversation_id: None,
        source: Some("chat".to_string()),
    };
    let result = handler.execute(effective_args, ctx).await.map_err(|e| e.0);
    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_best_effort(
                &HookEvent::AfterActionInvoke,
                &build_action_hook_payload(
                    None,
                    character_id,
                    Some("chat".to_string()),
                    tool_call,
                    Some(&action),
                    Some(result.is_ok()),
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

async fn wait_for_tool_approval_and_execute(
    app: &tauri::AppHandle,
    approval_state: &PendingToolApprovalState,
    registry_state: &std::sync::Arc<RwLock<ActionRegistry>>,
    character_id: &str,
    turn_id: &str,
    outcome: &crate::actions::ToolExecutionOutcome,
    pending_error: &str,
) -> Result<(Result<ActionResult, String>, serde_json::Value), KokoroError> {
    let approval_request_id = approval_state
        .register(
            turn_id.to_string(),
            outcome.tool_id().to_string(),
            outcome.tool_name().to_string(),
            outcome.invocation.args.clone(),
        )
        .await;
    let requested_payload =
        pending_tool_trace_payload(outcome, turn_id, pending_error, &approval_request_id);
    let receiver = approval_state
        .take_receiver(&approval_request_id)
        .await
        .ok_or_else(|| {
            KokoroError::Internal("Missing approval receiver after registration".to_string())
        })?;

    app.emit("chat-turn-tool", requested_payload.clone())
        .map_err(|e| KokoroError::Chat(e.to_string()))?;

    let decision = receiver.await.map_err(|_| {
        KokoroError::Validation(format!(
            "Approval request '{}' was dropped",
            approval_request_id
        ))
    })?;

    match decision {
        ToolApprovalDecision::Approved => {
            let result = execute_single_tool_after_approval(
                app,
                registry_state,
                character_id,
                &outcome.invocation,
            )
            .await;
            let payload = match &result {
                Ok(value) => {
                    approved_tool_trace_payload(outcome, turn_id, value, &approval_request_id)
                }
                Err(error) => {
                    approved_tool_error_payload(outcome, turn_id, error, &approval_request_id)
                }
            };
            Ok((result, payload))
        }
        ToolApprovalDecision::Rejected { reason } => {
            let rejected_message = rejected_pending_approval_message(reason);
            let payload = rejected_tool_trace_payload(
                outcome,
                turn_id,
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

/// Strip `[TRANSLATE:...]` tags from text.
fn strip_translate_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — remove from [TRANSLATE: to end
            result = result[..start].trim_end().to_string();
        }
    }
    result.trim().to_string()
}

fn merge_continuation_text(accumulated: &mut String, next: &str) {
    if next.is_empty() {
        return;
    }
    if accumulated.is_empty() {
        accumulated.push_str(next);
        return;
    }
    if next.starts_with(accumulated.as_str()) {
        *accumulated = next.to_string();
        return;
    }
    if accumulated.ends_with(next) {
        return;
    }

    let mut overlap = 0usize;
    let max_overlap = accumulated.len().min(next.len());
    for candidate in (1..=max_overlap).rev() {
        if accumulated.is_char_boundary(accumulated.len() - candidate)
            && next.is_char_boundary(candidate)
            && accumulated[accumulated.len() - candidate..] == next[..candidate]
        {
            overlap = candidate;
            break;
        }
    }

    if overlap > 0 {
        accumulated.push_str(&next[overlap..]);
    } else {
        if !accumulated.ends_with(char::is_whitespace) && !next.starts_with(char::is_whitespace) {
            accumulated.push(' ');
        }
        accumulated.push_str(next);
    }
}

/// Extract the content inside `[TRANSLATE:...]` tags, then strip them from text.
/// Returns (cleaned_text, Option<translation>).
fn extract_translate_tags(text: &str) -> (String, Option<String>) {
    let mut translations = Vec::new();
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..start + end_bracket];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — extract what we can
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            result = result[..start].trim_end().to_string();
        }
    }
    let translation = if translations.is_empty() {
        None
    } else {
        Some(translations.join(" "))
    };
    (result.trim().to_string(), translation)
}

/// Parsed tool call from `[TOOL_CALL:name|key=val|key=val]`
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolCall {
    tool_call_id: Option<String>,
    name: String,
    args: HashMap<String, String>,
}

impl From<ToolCall> for ToolInvocation {
    fn from(value: ToolCall) -> Self {
        Self {
            tool_call_id: value.tool_call_id,
            name: value.name,
            args: value.args,
        }
    }
}

/// Parse all `[TOOL_CALL:name|key=val|...]` tags from the text.
/// Returns (cleaned_text, Vec<ToolCall>).
fn parse_tool_call_tags(text: &str) -> (String, Vec<ToolCall>) {
    let mut result = text.to_string();
    let mut calls = Vec::new();

    while let Some(start) = result.rfind(TOOL_CALL_TAG_PREFIX) {
        let rest = &result[start..];
        if let Some(end_bracket) = rest.find(']') {
            let inner = &rest[TOOL_CALL_TAG_PREFIX.len()..end_bracket];
            let parts: Vec<&str> = inner.split('|').collect();

            if let Some(name) = parts.first() {
                let name = name.trim().to_string();
                let mut args = HashMap::new();

                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }

                calls.push(ToolCall {
                    tool_call_id: None,
                    name,
                    args,
                });
            }

            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                if tag_end < result.len() {
                    &result[tag_end..]
                } else {
                    ""
                }
            );
        } else {
            break;
        }
    }

    // 额外支持简化格式: [action_name|key=val|key=val]
    // 例: [play_cue|cue=shy]
    let mut extra_calls = Vec::new();
    let mut cleaned = result.clone();
    let mut offset = 0;
    while offset < cleaned.len() {
        let Some(rel_start) = cleaned[offset..].find('[') else {
            break;
        };
        let start = offset + rel_start;
        let rest = &cleaned[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(pipe_pos) = inner.find('|') {
            let name_part = &inner[..pipe_pos];
            let is_identifier =
                !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');
            let has_kv = inner[pipe_pos + 1..].contains('=');

            if is_identifier && has_kv {
                let parts: Vec<&str> = inner.split('|').collect();
                let name = parts[0].trim().to_string();
                let mut args = HashMap::new();
                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }
                extra_calls.push(ToolCall {
                    tool_call_id: None,
                    name,
                    args,
                });
                let tag_end = start + end + 1;
                cleaned = format!(
                    "{}{}",
                    cleaned[..start].trim_end(),
                    if tag_end < cleaned.len() {
                        &cleaned[tag_end..]
                    } else {
                        ""
                    }
                );
                // offset 不变，继续从同一位置扫描（内容已缩短）
                matched = true;
            }
        }
        if !matched {
            // 跳过这个 [ 继续往后找
            offset = start + 1;
        }
    }
    calls.extend(extra_calls);

    // 支持冒号格式: [action_name:value]
    // 例: [play_cue:happy]、[set_background:beach]
    // 将 value 映射到该 action 的主参数名
    let primary_arg_map: &[(&str, &str)] = &[("play_cue", "cue"), ("set_background", "prompt")];
    let mut colon_calls = Vec::new();
    let mut cleaned2 = cleaned.clone();
    let mut offset2 = 0;
    while offset2 < cleaned2.len() {
        let Some(rel_start) = cleaned2[offset2..].find('[') else {
            break;
        };
        let start = offset2 + rel_start;
        let rest = &cleaned2[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(colon_pos) = inner.find(':') {
            let name_part = inner[..colon_pos].trim();
            let val_part = inner[colon_pos + 1..].trim();
            let is_identifier =
                !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');

            if is_identifier && !val_part.is_empty() {
                if let Some(&(_, arg_key)) = primary_arg_map.iter().find(|&&(n, _)| n == name_part)
                {
                    let mut args = HashMap::new();
                    args.insert(arg_key.to_string(), val_part.to_string());
                    colon_calls.push(ToolCall {
                        tool_call_id: None,
                        name: name_part.to_string(),
                        args,
                    });
                    let tag_end = start + end + 1;
                    cleaned2 = format!(
                        "{}{}",
                        cleaned2[..start].trim_end(),
                        if tag_end < cleaned2.len() {
                            &cleaned2[tag_end..]
                        } else {
                            ""
                        }
                    );
                    matched = true;
                }
            }
        }
        if !matched {
            offset2 = start + 1;
        }
    }
    calls.extend(colon_calls);

    calls.reverse();
    (cleaned2.trim().to_string(), calls)
}

// ── Stream Chat Command ────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn stream_chat(
    window: Window,
    app: tauri::AppHandle,
    request: ChatRequest,
    state: State<'_, AIOrchestrator>,
    imagegen_state: State<'_, ImageGenService>,
    llm_state: State<'_, LlmService>,
    _action_registry: State<'_, std::sync::Arc<RwLock<crate::actions::ActionRegistry>>>,
    tool_settings_state: State<'_, std::sync::Arc<RwLock<ToolSettings>>>,
    approval_state: State<'_, Arc<PendingToolApprovalState>>,
    _vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
    window_size_state: State<'_, WindowSizeState>,
    vision_server: State<
        '_,
        std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>,
    >,
) -> Result<(), KokoroError> {
    // 0. Resolve character ID for this request (not stored in shared state)
    let char_id = request
        .character_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let conversation_id = state.current_conversation_id.lock().await.clone();
    let hook_runtime = app.try_state::<HookRuntime>();
    // Keep shared character_id in sync for modules that still read it (heartbeat)
    state.set_character_id(char_id.clone()).await;

    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_best_effort(
                &HookEvent::BeforeUserMessage,
                &build_chat_hook_payload(
                    conversation_id.clone(),
                    &char_id,
                    None,
                    Some(request.message.clone()),
                    None,
                    None,
                    request.hidden,
                ),
            )
            .await;
    }

    // Record user activity
    state.touch_activity().await;

    // Typing simulation
    {
        let is_question = request.message.contains('?') || request.message.contains('？');
        let typing_params = crate::ai::typing_sim::calculate_typing_delay(
            "neutral",
            0.5,
            0.6,
            request.message.chars().count(),
            is_question,
        );
        let _ = app.emit("chat-typing", &typing_params);
    }

    // 1. Update History with User Message (skip for hidden/touch interactions)
    let system_provider = llm_state.system_provider().await;
    if !request.hidden {
        state
            .add_message_with_metadata(
                "user".to_string(),
                request.message.clone(),
                None,
                &char_id,
                Some(system_provider.clone()),
            )
            .await;

        if let Some(hooks) = hook_runtime.as_ref() {
            hooks
                .emit_best_effort(
                    &HookEvent::AfterUserMessagePersisted,
                    &build_chat_hook_payload(
                        conversation_id.clone(),
                        &char_id,
                        None,
                        Some(request.message.clone()),
                        None,
                        None,
                        request.hidden,
                    ),
                )
                .await;
        }
    }

    // ── LAYER 1 & 2: SYSTEM SETUP ───────────────────────────────

    // ── EXECUTION & STATE UPDATE ────────────────────────────────

    // ── LAYER 3: PERSONA GENERATION ─────────────────────────────

    let llm_config = llm_state.config().await;
    let chat_provider = llm_state.provider().await;
    let native_tools_enabled = llm_config
        .providers
        .iter()
        .find(|provider| provider.id == llm_config.active_provider)
        .map(|provider| provider.supports_native_tools)
        .unwrap_or(true);
    tracing::info!(
        target: "chat",
        "[Chat] active_provider={}, native_tools_enabled={}",
        llm_config.active_provider, native_tools_enabled
    );

    // Native tool-calling requests already carry structured tool definitions,
    // so avoid duplicating a long textual tool prompt there.
    let tool_prompt = {
        let registry = _action_registry.read().await;
        let tool_settings = tool_settings_state.read().await;
        let prompt = if native_tools_enabled {
            String::new()
        } else {
            registry.generate_tool_prompt_for_prompt_with_settings(
                state.is_memory_enabled(),
                &tool_settings,
            )
        };
        if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        }
    };

    let native_tools = {
        let registry = _action_registry.read().await;
        let tool_settings = tool_settings_state.read().await;
        registry.list_tools_for_llm_with_settings(state.is_memory_enabled(), &tool_settings)
    };

    // Compose Persona Prompt
    let prompt_messages = state
        .compose_prompt(
            &request.message,
            request.allow_image_gen.unwrap_or(false),
            tool_prompt,
            native_tools_enabled,
            &char_id,
        )
        .await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;

    let assistant_turn_id = uuid::Uuid::new_v4().to_string();
    let mut before_llm_request_payload = build_before_llm_request_payload(
        conversation_id.clone(),
        &char_id,
        Some(assistant_turn_id.clone()),
        request.message.clone(),
        request.hidden,
        &prompt_messages,
    );

    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_before_llm_request_modify(&mut before_llm_request_payload)
            .await;
    }

    let (effective_request_message, mut client_messages) =
        apply_before_llm_request_payload(before_llm_request_payload, &prompt_messages)
            .map_err(KokoroError::Chat)?;

    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_best_effort(
                &HookEvent::BeforeLlmRequest,
                &build_chat_hook_payload(
                    conversation_id.clone(),
                    &char_id,
                    Some(assistant_turn_id.clone()),
                    Some(effective_request_message.clone()),
                    None,
                    None,
                    request.hidden,
                ),
            )
            .await;
    }
    app.emit(
        "chat-turn-start",
        serde_json::json!({
            "turn_id": assistant_turn_id,
        }),
    )
    .map_err(|e| KokoroError::Chat(e.to_string()))?;

    // 注入视觉上下文（如果有最近的屏幕观察）
    if let Some(vision_desc) = _vision_watcher.context.get_context_string().await {
        client_messages.push(system_message(format!(
            "[Vision] The user's screen currently shows: {}",
            vision_desc
        )));
    }

    // Attach images to the last user message if present
    if let Some(images) = &request.images {
        if !images.is_empty() {
            // Find the last message with role "user"
            if let Some(last_user_msg) = client_messages
                .iter_mut()
                .rfind(|m| crate::llm::messages::is_user_message(m))
            {
                let text_content = extract_message_text(last_user_msg);

                // Process images: convert local URLs to base64
                let mut processed_images = Vec::with_capacity(images.len());
                let vision_server_guard = vision_server.lock().await;
                let port = vision_server_guard.port;
                let upload_dir = vision_server_guard.upload_dir.clone();
                drop(vision_server_guard);

                for img_url in images {
                    let mut final_url = img_url.clone();
                    // Check if local
                    if img_url.contains(&format!("http://127.0.0.1:{}", port)) {
                        // Extract filename
                        if let Some(filename) = img_url.split("/vision/").nth(1) {
                            let file_path = upload_dir.join(filename);
                            if let Ok(file_content) = tokio::fs::read(&file_path).await {
                                // Convert to base64
                                use base64::Engine as _;
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(&file_content);
                                // Detect mime type
                                let mime = crate::vision::server::detect_image_mime(&file_content)
                                    .unwrap_or("image/png".to_string());
                                final_url = format!("data:{};base64,{}", mime, b64);
                            }
                        }
                    }
                    processed_images.push(final_url);
                }

                // Create multimodal content
                replace_user_message_with_images(last_user_msg, text_content, processed_images)
                    .map_err(KokoroError::Chat)?;
                tracing::info!(target: "chat", "[Chat] Attached {} images to user message", images.len());
            }
        }
    }

    // For hidden messages (touch interactions), the user message wasn't added to
    // history, so we must explicitly include it in the context for the LLM to see.
    if request.hidden {
        client_messages.push(user_text_message(effective_request_message.clone()));
    }

    #[cfg(debug_assertions)]
    {
        tracing::info!(
            target: "llm",
            "[LLM/Debug] active_provider={} native_tools_enabled={} tool_count={}",
            llm_config.active_provider,
            native_tools_enabled,
            native_tools.len()
        );
        debug_log_llm_messages("initial chat request", &client_messages);
    }

    // Stream Response with Tool Call Feedback Loop
    let max_tool_rounds = {
        let tool_settings = tool_settings_state.read().await;
        tool_settings.max_tool_rounds.max(1)
    };
    let mut all_cleaned_text = String::new();
    let mut all_translations = Vec::new();
    let mut bg_generated_by_tool = false;
    let mut cue_set_by_tool = false;
    let mut draft_row_id: Option<i64> = None;
    let mut forced_text_after_side_effect = false;
    let mut stream_failed = false;
    let mut text_retry_count = 0u32;
    let mut force_text_only_round = false;

    for round in 0..max_tool_rounds {
        tracing::info!(target: "chat", "[Chat] Tool loop round {}", round + 1);

        let mut stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, String>> + Send>,
        > = if native_tools_enabled && !force_text_only_round {
            chat_provider
                .chat_stream_with_tools(client_messages.clone(), None, native_tools.clone())
                .await
                .map_err(KokoroError::Chat)?
        } else {
            let text_stream = chat_provider
                .chat_stream(client_messages.clone(), None)
                .await
                .map_err(KokoroError::Chat)?;
            Box::pin(text_stream.map(|item| item.map(LlmStreamEvent::Text)))
        };

        let mut round_response = String::new();
        let mut emit_buffer = String::new();
        let mut native_tool_calls = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(event) => {
                    match event {
                        LlmStreamEvent::Text(content) => {
                            round_response.push_str(&content);
                            emit_buffer.push_str(&content);

                            // Only emit text up to the safe boundary (before any potential tag)
                            let safe = find_safe_emit_boundary(&emit_buffer);
                            if safe > 0 {
                                let to_emit = emit_buffer[..safe].to_string();
                                emit_buffer = emit_buffer[safe..].to_string();
                                app.emit(
                                    "chat-turn-delta",
                                    serde_json::json!({
                                        "turn_id": assistant_turn_id,
                                        "delta": to_emit,
                                    }),
                                )
                                .map_err(|e| KokoroError::Chat(e.to_string()))?;
                            }
                        }
                        LlmStreamEvent::ToolCall(tool_call) => {
                            native_tool_calls.push(ToolCall {
                                tool_call_id: Some(tool_call.id),
                                name: tool_call.name,
                                args: tool_call.args,
                            });
                        }
                    }
                }
                Err(e) => {
                    if round_response.is_empty() && emit_buffer.is_empty() {
                        stream_failed = true;
                        app.emit("chat-error", e)
                            .map_err(|e| KokoroError::Chat(e.to_string()))?;
                    } else {
                        tracing::error!(
                            target: "chat",
                            "[Chat] Ignoring trailing stream error after partial response: {}",
                            e
                        );
                    }
                    break;
                }
            }
        }

        // Flush remaining buffer — strip any complete tags before emitting
        if !emit_buffer.is_empty() {
            let (cleaned_remainder, _) = parse_tool_call_tags(&emit_buffer);
            let cleaned_remainder = strip_translate_tags(&cleaned_remainder);
            if !cleaned_remainder.is_empty() {
                app.emit(
                    "chat-turn-delta",
                    serde_json::json!({
                        "turn_id": assistant_turn_id,
                        "delta": cleaned_remainder,
                    }),
                )
                .map_err(|e| KokoroError::Chat(e.to_string()))?;
            }
        }

        let (cleaned_text, mut tool_calls) = parse_tool_call_tags(&round_response);
        let (cleaned_text, round_translation) = extract_translate_tags(&cleaned_text);
        tool_calls.extend(native_tool_calls);

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

        // Collect translation from this round
        if let Some(t) = round_translation {
            all_translations.push(t);
        }

        // Accumulate cleaned text for history
        merge_continuation_text(&mut all_cleaned_text, &cleaned_text);

        // Persist assistant draft incrementally (hidden interactions still save the response, just not the user message)
        if !all_cleaned_text.is_empty() {
            let draft_content = strip_leaked_tags(&all_cleaned_text);
            if !draft_content.is_empty() {
                match draft_row_id {
                    None => {
                        // First round: insert draft row
                        match state
                            .persist_streaming_draft(&draft_content, &char_id)
                            .await
                        {
                            Ok(id) => {
                                draft_row_id = Some(id);
                            }
                            Err(e) => {
                                tracing::error!(target: "chat", "[Chat] Failed to persist streaming draft: {}", e);
                            }
                        }
                    }
                    Some(id) => {
                        // Subsequent rounds: update draft row
                        if let Err(e) = state.update_streaming_draft(id, &draft_content, None).await
                        {
                            tracing::error!(target: "chat", "[Chat] Failed to update streaming draft: {}", e);
                        }
                    }
                }
            }
        }

        // No tool calls → final round
        if tool_calls.is_empty() {
            break;
        }

        // Execute tool calls and collect results
        let tool_invocations = {
            let registry = _action_registry.inner().read().await;
            tool_calls
                .iter()
                .map(|tool_call| {
                    crate::commands::actions::build_tool_invocation_from_input(
                        &registry,
                        &tool_call.name,
                        tool_call.args.clone(),
                        tool_call.tool_call_id.clone(),
                    )
                    .map_err(|error| KokoroError::Validation(error.0))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let execution_outcomes = execute_tool_calls(
            window.app_handle(),
            &_action_registry.inner().clone(),
            &tool_settings_state.inner().clone(),
            &char_id,
            &tool_invocations,
        )
        .await;
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

        for outcome in execution_outcomes {
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
                invocation_source: "chat",
                risk_tags: &outcome.tool_risk_tags(),
                permission_level: outcome.tool_permission_level().unwrap_or("safe"),
                decision: outcome
                    .permission_decision
                    .as_ref()
                    .unwrap_or(&PermissionDecision::Allow),
                approved_by_user: None,
                conversation_id: None,
                character_id: Some(&char_id),
            });
            tracing::info!(target: "tools", "[ToolAudit] {:?}", audit_event);

            let result = if let Err(error) = &outcome.result {
                if matches!(
                    outcome.permission_decision,
                    Some(PermissionDecision::DenyPendingApproval { .. })
                ) {
                    let (resolved_result, resolved_payload) = wait_for_tool_approval_and_execute(
                        &app,
                        approval_state.inner().as_ref(),
                        &_action_registry.inner().clone(),
                        &char_id,
                        &assistant_turn_id,
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
                    app.emit("chat-turn-tool", resolved_payload)
                        .map_err(|e| KokoroError::Chat(e.to_string()))?;
                    resolved_result
                } else {
                    tracing::error!(target: "tools", "[ToolCall] {} failed: {}", outcome.tool_name(), error);
                    emit_tool_trace_event(&app, &assistant_turn_id, &outcome);
                    outcome.result.clone()
                }
            } else {
                if let Ok(success) = &outcome.result {
                    tracing::info!(target: "tools", "[ToolCall] {} => {}", outcome.tool_name(), success.message);
                }
                emit_tool_trace_event(&app, &assistant_turn_id, &outcome);
                outcome.result.clone()
            };

            tool_results.push(match &result {
                Ok(value) => format!("- {}: {}", outcome.tool_id(), value.message),
                Err(error) => format!("- {}: Error: {}", outcome.tool_id(), error),
            });

            if let Some(tool_call_id) = &outcome.invocation.tool_call_id {
                continuation_tool_calls
                    .push(assistant_tool_call_metadata_value(&outcome, tool_call_id));
                continuation_tool_call_messages.push((
                    tool_call_id.clone(),
                    outcome.tool_name().to_string(),
                    serde_json::to_string(&outcome.invocation.args)
                        .unwrap_or_else(|_| "{}".to_string()),
                ));
                let message_text = match &result {
                    Ok(result) => result.message.clone(),
                    Err(error) => format!("Error: {}", error),
                };
                let tool_result_msg = tool_result_message(tool_call_id.clone(), message_text);
                tool_result_messages.push(tool_result_msg.clone());
                persisted_native_tool_results.push((
                    tool_metadata_value(&outcome, tool_call_id, &assistant_turn_id),
                    tool_result_msg,
                ));
            }
        }

        if has_native_tool_calls {
            let assistant_tool_call_metadata = serde_json::json!({
                "type": "assistant_tool_calls",
                "turn_id": assistant_turn_id,
                "tool_calls": continuation_tool_calls,
            })
            .to_string();
            state
                .add_message_with_metadata(
                    "assistant".to_string(),
                    cleaned_text.clone(),
                    Some(assistant_tool_call_metadata),
                    &char_id,
                    None,
                )
                .await;
            for (tool_metadata, tool_message) in &persisted_native_tool_results {
                let tool_content = extract_message_text(tool_message);
                state
                    .add_message_with_metadata(
                        "tool".to_string(),
                        tool_content,
                        Some(tool_metadata.to_string()),
                        &char_id,
                        None,
                    )
                    .await;
            }
            client_messages.push(assistant_tool_calls_message(
                if cleaned_text.is_empty() {
                    None
                } else {
                    Some(cleaned_text.clone())
                },
                continuation_tool_call_messages,
            ));
            client_messages.extend(tool_result_messages);

            // Apply the same side-effect / feedback guard as the text-tool path
            // to prevent infinite loops when LLM keeps emitting only tool calls
            // without any dialogue text.
            if !any_needs_feedback {
                if all_cleaned_text.trim().is_empty() && !forced_text_after_side_effect {
                    tracing::info!(target: "chat", "[Chat] Native side-effect tools ran without text, forcing one follow-up text round");
                    forced_text_after_side_effect = true;
                    client_messages.push(system_message(
                        "The tool has already been executed successfully. \
                         Now respond with a natural dialogue reply for the user. \
                         Do NOT call the same tool again unless absolutely necessary."
                            .to_string(),
                    ));
                } else if all_cleaned_text.trim().is_empty() {
                    // Already forced once but still no text — retry with tools disabled
                    if text_retry_count < 3 {
                        text_retry_count += 1;
                        forced_text_after_side_effect = false;
                        force_text_only_round = true;
                        tracing::info!(target: "chat", "[Chat] Native tool loop: no text after forced round, retrying without tools ({}/3)", text_retry_count);
                        // Strip trailing tool/system/empty-assistant messages to avoid poisoning the context
                        while client_messages.len() > 1 {
                            let should_pop = match client_messages.last() {
                                Some(async_openai::types::chat::ChatCompletionRequestMessage::Tool(_)) => true,
                                Some(async_openai::types::chat::ChatCompletionRequestMessage::System(_)) => true,
                                Some(async_openai::types::chat::ChatCompletionRequestMessage::Assistant(m)) => {
                                    m.content.as_ref().map(|_c| extract_message_text(
                                        &async_openai::types::chat::ChatCompletionRequestMessage::Assistant(m.clone())
                                    ).trim().is_empty()).unwrap_or(true)
                                }
                                _ => false,
                            };
                            if should_pop {
                                client_messages.pop();
                            } else {
                                break;
                            }
                        }
                        client_messages.push(system_message(
                            "IMPORTANT: Respond with dialogue text only. Do NOT call any tools."
                                .to_string(),
                        ));
                        continue;
                    }
                    tracing::info!(
                        target: "chat::tools",
                        "[Chat] Native tool loop: still no text after {} retries, ending loop",
                        text_retry_count
                    );
                    break;
                }
                // If there IS text, fall through to continue normally
            }

            tracing::info!(
                target: "chat::tools",
                "[Chat] Continuing after native tool calls with assistant/tool result messages"
            );
            #[cfg(debug_assertions)]
            debug_log_llm_messages(
                &format!("post-tool continuation round {}", round + 1),
                &client_messages,
            );
            continue;
        }

        // Only continue the loop if at least one tool needs its result fed back to the LLM
        if !any_needs_feedback {
            if all_cleaned_text.trim().is_empty() && !forced_text_after_side_effect {
                tracing::info!(target: "chat", "[Chat] Side-effect tools ran without any text reply, forcing one follow-up text round");
                forced_text_after_side_effect = true;
                client_messages.push(system_message(format!(
                    "[Tool results]\n\
                    {}\n\n\
                    The side-effect tool has already been executed successfully.\n\
                    Now continue with a natural reply for the user in plain dialogue text.\n\
                    Do not explain the tool call, do not output metadata, and do not repeat the same side-effect tool unless it is still necessary.",
                    tool_results.join("\n")
                )));
                #[cfg(debug_assertions)]
                debug_log_llm_messages(
                    &format!("forced follow-up round {}", round + 1),
                    &client_messages,
                );
                continue;
            }

            tracing::info!(target: "chat", "[Chat] No feedback-requiring tools, ending loop");
            break;
        }

        // Only inject tool results — no need to replay the assistant's previous output
        client_messages.push(system_message(format!(
            "[Tool results]\n\
            {}\n\n\
            Incorporate these results naturally into your dialogue. Do NOT echo raw data or JSON.",
            tool_results.join("\n")
        )));
        #[cfg(debug_assertions)]
        debug_log_llm_messages(
            &format!("feedback continuation round {}", round + 1),
            &client_messages,
        );
    }

    let full_response = strip_leaked_tags(&all_cleaned_text);

    if let Some(hooks) = hook_runtime.as_ref() {
        hooks
            .emit_best_effort(
                &HookEvent::AfterLlmResponse,
                &build_chat_hook_payload(
                    conversation_id.clone(),
                    &char_id,
                    Some(assistant_turn_id.clone()),
                    Some(request.message.clone()),
                    Some(full_response.clone()),
                    None,
                    request.hidden,
                ),
            )
            .await;
    }

    // Fallback translation: if main LLM missed the [TRANSLATE:...] tag, use system LLM to fill in
    if all_translations.is_empty() && !full_response.is_empty() {
        let user_lang = state.user_language.lock().await.clone();
        let resp_lang = state.response_language.lock().await.clone();
        tracing::info!(
            target: "chat",
            "[Chat] Fallback check: user_lang={:?}, resp_lang={:?}",
            user_lang, resp_lang
        );
        if !user_lang.is_empty() && !resp_lang.is_empty() && user_lang != resp_lang {
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
            match system_provider.chat(fallback_messages, None).await {
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
    if !cue_set_by_tool && !full_response.is_empty() {
        tracing::info!(target: "chat", "[Chat] Cue not set by tool, triggering fallback cue analysis");
        let mut emotion_messages = vec![system_message(
            crate::ai::prompts::EMOTION_ANALYZER_PROMPT.to_string(),
        )];
        if let Some(profile) = crate::commands::live2d::load_active_live2d_profile() {
            let available_cues = profile
                .cue_map
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            emotion_messages.push(system_message(format!(
                "Available cues for the active model: {}.\nChoose exactly one from this list, or return null if none fit.",
                if available_cues.is_empty() { "(none)" } else { &available_cues }
            )));
        }
        emotion_messages.push(user_text_message(full_response.clone()));
        let valid_fallback_cues =
            crate::commands::live2d::load_active_live2d_profile().map(|profile| {
                profile
                    .cue_map
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>()
            });
        match system_provider.chat(emotion_messages, None).await {
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
                            let _ = app.emit(
                                "chat-cue",
                                serde_json::json!({ "cue": trimmed, "source": "fallback-cue" }),
                            );
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

    // Emit combined translation from all rounds
    if !all_translations.is_empty() {
        let combined_translation = all_translations.join(" ");
        let _ = app.emit(
            "chat-turn-translation",
            serde_json::json!({
                "turn_id": assistant_turn_id,
                "translation": combined_translation,
            }),
        );
    }

    // 8. Update History with final response
    // hidden 模式下跳过用户消息保存，但助手回复仍需持久化以便重载后显示
    if !full_response.is_empty() {
        let metadata = if !all_translations.is_empty() {
            let combined = all_translations.join(" ");
            Some(
                serde_json::json!({
                    "translation": combined,
                    "turn_id": assistant_turn_id,
                })
                .to_string(),
            )
        } else {
            Some(
                serde_json::json!({
                    "turn_id": assistant_turn_id,
                })
                .to_string(),
            )
        };

        // Update the draft row with final content + metadata (DB already has the row)
        if let Some(row_id) = draft_row_id {
            if let Err(e) = state
                .update_streaming_draft(row_id, &full_response, metadata.as_deref())
                .await
            {
                tracing::error!(target: "chat", "[Chat] Failed to finalize streaming draft: {}", e);
            }
        }

        // Add to in-memory history only (DB already persisted)
        {
            let max_chars = *state.max_message_chars.lock().await;
            let content = if full_response.chars().count() > max_chars {
                let truncated: String = full_response.chars().take(max_chars).collect();
                format!("{}…[truncated]", truncated)
            } else {
                full_response.clone()
            };
            state
                .push_history_message(Message {
                    role: "assistant".to_string(),
                    content,
                    metadata: None,
                })
                .await;
        }
    }

    // Periodic memory extraction
    let msg_count = state.get_message_count().await;
    let memory_msg_count = state.get_memory_trigger_count().await;
    tracing::info!(
        target: "memory",
        "[Memory] User message count: {}, memory trigger count: {}",
        msg_count, memory_msg_count
    );
    if state.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 5 == 0 {
        tracing::info!(
            target: "memory",
            "[Memory] Triggering memory extraction (count={})",
            msg_count
        );
        let history = state.get_recent_memory_history(10).await;
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_mem = char_id.clone();
        let provider_for_mem = system_provider.clone();
        let memory_enabled = state.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            memory_extractor::extract_and_store_memories(
                &history,
                &memory_mgr,
                provider_for_mem,
                char_id_for_mem,
            )
            .await;
        });
    }

    // Periodic memory consolidation (every 20 user messages)
    if state.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 20 == 0 {
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_consolidation = char_id.clone();
        let provider_for_consolidation = system_provider.clone();
        let memory_enabled = state.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            match memory_mgr
                .consolidate_memories(&char_id_for_consolidation, provider_for_consolidation)
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(target: "memory", "[Memory] Consolidated {} memory clusters", count);
                }
                Err(e) => {
                    tracing::error!(target: "memory", "[Memory] Consolidation failed: {}", e);
                }
                _ => {}
            }
        });
    }

    // Background image generation: analyze reply and optionally generate a scene image
    // Skip if the main LLM already triggered set_background via tool call
    if request.allow_image_gen.unwrap_or(false)
        && !full_response.is_empty()
        && !bg_generated_by_tool
    {
        let imagegen_svc = imagegen_state.inner().clone();
        let system_provider = llm_state.system_provider().await;
        let reply_for_analysis = full_response.clone();
        let window_for_img = window.clone();
        let window_size = window_size_state.get().await;

        tauri::async_runtime::spawn(async move {
            let analyze_messages = vec![
                system_message(crate::ai::prompts::BG_IMAGE_ANALYZER_PROMPT.to_string()),
                user_text_message(format!("Character reply: {}", reply_for_analysis)),
            ];

            let json_str = match system_provider.chat(analyze_messages, None).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(target: "imagegen", "[ImageGen] BG analyzer LLM failed: {}", e);
                    return;
                }
            };

            let clean = json_str
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```");

            #[derive(serde::Deserialize)]
            struct BgAnalysis {
                should_generate: bool,
                image_prompt: Option<String>,
            }

            let analysis: BgAnalysis = match serde_json::from_str(clean) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        target: "imagegen",
                        "[ImageGen] BG analyzer parse failed: {} | raw: {}",
                        e, json_str
                    );
                    return;
                }
            };

            if !analysis.should_generate {
                tracing::info!(target: "imagegen", "[ImageGen] BG analyzer: no image needed");
                return;
            }

            let prompt = match analysis.image_prompt {
                Some(p) if !p.is_empty() => p,
                _ => return,
            };

            tracing::info!(target: "imagegen", "[ImageGen] BG analyzer triggered generation: {}", prompt);

            match imagegen_svc
                .generate(prompt.clone(), None, None, Some(window_size))
                .await
            {
                Ok(result) => {
                    let _ = window_for_img.emit("imagegen:done", &result);
                    tracing::info!(target: "imagegen", "[ImageGen] BG image generated: {}", result.image_url);
                }
                Err(e) => {
                    tracing::error!(target: "imagegen", "[ImageGen] BG generation failed: {}", e);
                    let _ = window_for_img.emit("imagegen:error", e.to_string());
                }
            }
        });
    }

    let finish_status = if stream_failed && full_response.is_empty() {
        "error"
    } else {
        "completed"
    };
    app.emit(
        "chat-turn-finish",
        serde_json::json!({
            "turn_id": assistant_turn_id,
            "status": finish_status,
        }),
    )
    .map_err(|e| KokoroError::Chat(e.to_string()))?;

    Ok(())
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
    use crate::hooks::HookPayload;

    // ── extract_translate_tags ──────────────────────────────

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
        assert_eq!(extract_message_text(&client_messages[0]), "system prompt");
        assert_eq!(extract_message_text(&client_messages[1]), "modified user");
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
        assert_eq!(extract_message_text(&client_messages[0]), "system prompt");
        assert_eq!(extract_message_text(&client_messages[1]), "hello");
    }

    #[test]
    fn test_extract_translate_tags_basic() {
        let input = "こんにちは[TRANSLATE:你好]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, Some("你好".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_none() {
        let input = "こんにちは";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, None);
    }

    #[test]
    fn test_extract_translate_tags_multiple() {
        let input = "A[TRANSLATE:甲] B[TRANSLATE:乙]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "AB");
        assert_eq!(translation, Some("甲 乙".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_unclosed() {
        let input = "hello[TRANSLATE:world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "hello");
        assert_eq!(translation, Some("world".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_empty_content() {
        let input = "hello[TRANSLATE:]world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "helloworld");
        assert_eq!(translation, None);
    }

    // ── strip_translate_tags ────────────────────────────────

    #[test]
    fn test_strip_translate_tags() {
        let input = "こんにちは[TRANSLATE:你好]";
        assert_eq!(strip_translate_tags(input), "こんにちは");
    }

    #[test]
    fn test_strip_translate_tags_no_tag() {
        let input = "こんにちは";
        assert_eq!(strip_translate_tags(input), "こんにちは");
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

    // ── strip_leaked_tags ───────────────────────────────────

    #[test]
    fn test_strip_leaked_tags_removes_tool_result() {
        let input = "before<tool_result>leaked data</tool_result>after";
        assert_eq!(strip_leaked_tags(input), "beforeafter");
    }

    #[test]
    fn test_strip_leaked_tags_unclosed() {
        let input = "before<tool_result>leaked\nafter";
        assert_eq!(strip_leaked_tags(input), "before\nafter");
    }

    #[test]
    fn test_strip_leaked_tags_no_tag() {
        let input = "clean text";
        assert_eq!(strip_leaked_tags(input), "clean text");
    }

    // ── find_safe_emit_boundary ─────────────────────────────

    #[test]
    fn test_safe_emit_boundary_no_bracket() {
        let text = "hello world";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_tool_call() {
        let text = "hello [TOOL_CA";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_translate() {
        let text = "hello [TRANS";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_unrelated_bracket() {
        let text = "hello [world]";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    // ── parse_tool_call_tags ────────────────────────────────

    #[test]
    fn test_parse_tool_call_basic() {
        let input = "text[TOOL_CALL:play_cue|cue=happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_no_tag() {
        let input = "just text";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "just text");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_multiple_args() {
        let input = "[TOOL_CALL:set_background|prompt=beach|style=anime]";
        let (_, calls) = parse_tool_call_tags(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args.get("prompt"), Some(&"beach".to_string()));
        assert_eq!(calls[0].args.get("style"), Some(&"anime".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_format() {
        let input = "text[play_cue|cue=shy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"shy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_multiple() {
        let input = "hello[play_cue|cue=happy]world[play_cue|cue=sad]end";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "helloworldend");
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_parse_tool_call_simplified_no_false_positive() {
        // 普通方括号内容不应被误识别
        let input = "text [some words] more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text [some words] more");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_colon_format() {
        let input = "text[play_cue:happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_colon_unknown_action_no_match() {
        // 未在映射表中的 action 不应被识别为工具调用
        let input = "text[unknown_action:value]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text[unknown_action:value]more");
        assert!(calls.is_empty());
    }
}
