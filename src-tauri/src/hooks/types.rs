// pattern: Functional Core
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    BeforeUserMessage,
    AfterUserMessagePersisted,
    BeforeLlmRequest,
    AfterLlmResponse,
    BeforeActionInvoke,
    AfterActionInvoke,
    BeforeTtsPlay,
    AfterTtsPlay,
    OnModLoaded,
    OnModUnloaded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatHookPayload {
    pub conversation_id: Option<String>,
    pub character_id: String,
    pub turn_id: Option<String>,
    pub message: Option<String>,
    pub response: Option<String>,
    pub tool_round: Option<usize>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionHookPayload {
    pub conversation_id: Option<String>,
    pub character_id: String,
    pub tool_call_id: Option<String>,
    pub action_id: Option<String>,
    pub action_name: String,
    pub args: HashMap<String, String>,
    pub success: Option<bool>,
    pub result_message: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModHookPayload {
    pub mod_id: String,
    pub stage: String,
    pub has_theme: bool,
    pub has_layout: bool,
    pub component_count: usize,
    pub script_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsHookPayload {
    pub text: String,
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeforeLlmRequestMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeforeLlmRequestPayload {
    pub conversation_id: Option<String>,
    pub character_id: String,
    pub turn_id: Option<String>,
    pub hidden: bool,
    pub request_message: String,
    pub messages: Vec<BeforeLlmRequestMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeforeActionArgsPayload {
    pub conversation_id: Option<String>,
    pub character_id: String,
    pub tool_call_id: Option<String>,
    pub action_id: String,
    pub action_name: String,
    pub args: HashMap<String, String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookPayload {
    Chat(ChatHookPayload),
    Action(ActionHookPayload),
    Mod(ModHookPayload),
    Tts(TtsHookPayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookModifyPolicy {
    Permissive,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookOutcome {
    Continue,
    Deny { reason: String },
}
