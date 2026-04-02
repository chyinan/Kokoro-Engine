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
pub enum HookPayload {
    Chat(ChatHookPayload),
    Action(ActionHookPayload),
    Mod(ModHookPayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookOutcome {
    Continue,
}
