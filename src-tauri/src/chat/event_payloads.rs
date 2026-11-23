//! Wire payloads produced by the chat journal and its desktop subscribers.
use crate::actions::registry::{ActionPermissionLevel, ActionRiskTag, ActionSource};
use crate::error::FailureEvent;
use crate::imagegen::ImageGenResult;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Clone, Serialize, Type, Event)]
#[serde(transparent)]
#[tauri_specta(event_name = "chat-failure")]
pub(crate) struct ChatFailure(pub(crate) FailureEvent);

#[derive(Clone, Serialize, Type, Event)]
#[serde(transparent)]
#[tauri_specta(event_name = "memory:updated")]
pub(crate) struct MemoryUpdated(pub(crate) String);

#[derive(Clone, Default, Deserialize, Serialize, Type)]
pub(crate) struct ChatEventScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_generation: Option<Option<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<Option<String>>,
}

#[derive(Clone, Default, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-cue")]
pub(crate) struct CueEvent {
    #[serde(flatten)]
    pub scope: ChatEventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Option<String>>,
    pub cue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-acknowledged")]
pub(crate) struct ChatTurnAcknowledgedEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-bound")]
pub(crate) struct ChatTurnBoundEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) user_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-start")]
pub(crate) struct ChatTurnStartEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) user_message_id: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-delta")]
pub(crate) struct ChatTurnDeltaEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) delta: String,
}

#[derive(Clone, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChatFinishStatus {
    Completed,
    Error,
    Cancelled,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChatTurnStatus {
    Pending,
    Accepted,
    Streaming,
    Completed,
    Cancelled,
    Rejected,
    Error,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-finish")]
pub(crate) struct ChatTurnFinishEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: Option<String>,
    pub(crate) status: ChatFinishStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) assistant_message_id: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-text-complete")]
pub(crate) struct ChatTurnTextCompleteEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) text: String,
    pub(crate) translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) translation_pending: Option<bool>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-translation")]
pub(crate) struct ChatTurnTranslationEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) translation: String,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-warning")]
pub(crate) struct ChatWarningEvent {
    pub(crate) message: String,
    pub(crate) character_id: String,
    pub(crate) conversation_id: String,
    pub(crate) client_request_id: String,
    pub(crate) turn_id: String,
    pub(crate) conversation_generation: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolDenyKind {
    HookDenied,
    PolicyDenied,
    FailClosed,
    PendingApproval,
    ExecutionError,
}

#[derive(Clone, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolApprovalStatus {
    Requested,
    Approved,
    Rejected,
}

#[derive(Clone, Default, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-turn-tool")]
pub(crate) struct ChatTurnToolEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    pub(crate) tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<Option<ActionSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) server_name: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) needs_feedback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) permission_level: Option<Option<ActionPermissionLevel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) risk_tags: Option<Vec<ActionRiskTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<crate::actions::ActionResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) deny_kind: Option<ToolDenyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) approval_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) approval_status: Option<ToolApprovalStatus>,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-typing")]
pub(crate) struct ChatTypingEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    pub(crate) turn_id: String,
    #[serde(flatten)]
    pub(crate) typing: crate::ai::typing_sim::TypingParams,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-imagegen")]
pub(crate) struct ChatImageGenEvent {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) turn_id: Option<Option<String>>,
    pub(crate) prompt: String,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "chat-imagegen-error")]
pub(crate) struct ChatImageGenError {
    #[serde(flatten)]
    pub(crate) scope: ChatEventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) turn_id: Option<Option<String>>,
    pub(crate) message: String,
}

#[derive(Clone, Deserialize, Serialize, Type, Event)]
#[tauri_specta(event_name = "imagegen:done")]
pub(crate) struct ImageGenDone {
    #[serde(flatten)]
    pub result: ImageGenResult,
    #[serde(flatten)]
    pub scope: ChatEventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Option<String>>,
}
