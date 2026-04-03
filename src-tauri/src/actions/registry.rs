// pattern: Mixed (needs refactoring)
// Reason: 该文件同时承载 action 元数据、LLM prompt 生成与执行 handler trait；当前阶段只在现有中心点上最小增补 metadata。
//! Tool Registry — core framework for tool calling.
//!
//! Provides a registry of actions that the LLM can invoke via `[TOOL_CALL:name|args]` tags.
//! Actions are registered at startup and can be invoked by the chat pipeline.

use crate::actions::tool_settings::ToolSettings;
use crate::llm::provider::{LlmToolDefinition, LlmToolParam};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::AppHandle;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionParam {
    pub name: String,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ActionResult {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn ok_with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionError(pub String);

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ActionError {}

/// Context passed to action handlers at execution time.
pub struct ActionContext {
    pub app: AppHandle,
    pub character_id: String,
    pub conversation_id: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    Builtin,
    Mcp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionRiskTag {
    Read,
    Write,
    External,
    Sensitive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionPermissionLevel {
    Safe,
    Elevated,
}

/// Metadata for a registered action (returned to frontend / LLM prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInfo {
    pub id: String,
    pub name: String,
    pub source: ActionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    pub description: String,
    pub parameters: Vec<ActionParam>,
    pub needs_feedback: bool,
    pub risk_tags: Vec<ActionRiskTag>,
    pub permission_level: ActionPermissionLevel,
}

#[derive(Clone)]
struct ActionEntry {
    info: ActionInfo,
    handler: Arc<dyn ActionHandler>,
}

// ── Handler Trait ──────────────────────────────────────

#[async_trait]
pub trait ActionHandler: Send + Sync {
    /// Unique name for this action, e.g. "get_time"
    fn name(&self) -> &str;

    /// Human-readable description for the LLM prompt
    fn description(&self) -> &str;

    /// Parameter definitions
    fn parameters(&self) -> Vec<ActionParam>;

    /// Whether the LLM needs to see the result of this action to continue its response.
    /// Return `true` for information-retrieval tools (get_time, search_memory, etc.).
    /// Return `false` (default) for side-effect tools (play_cue, set_background, etc.).
    fn needs_feedback(&self) -> bool {
        false
    }

    fn risk_tags(&self) -> Vec<ActionRiskTag> {
        vec![ActionRiskTag::Read]
    }

    fn permission_level(&self) -> ActionPermissionLevel {
        ActionPermissionLevel::Safe
    }

    /// Execute the action with the given arguments
    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError>;
}

// ── Registry ───────────────────────────────────────────

pub struct ActionRegistry {
    entries_by_id: HashMap<String, ActionEntry>,
    alias_to_ids: HashMap<String, Vec<String>>,
    mcp_tool_ids: HashSet<String>,
}

const MEMORY_ACTIONS: &[&str] = &["search_memory", "store_memory", "forget_memory"];

fn encode_tool_id_segment(value: &str) -> String {
    value.replace('%', "%25").replace("__", "%5F%5F")
}

pub fn builtin_tool_id(name: &str) -> String {
    format!("builtin__{}", encode_tool_id_segment(name))
}

pub fn mcp_tool_id(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        encode_tool_id_segment(server_name),
        encode_tool_id_segment(tool_name)
    )
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            entries_by_id: HashMap::new(),
            alias_to_ids: HashMap::new(),
            mcp_tool_ids: HashSet::new(),
        }
    }

    fn make_action_info(
        source: ActionSource,
        server_name: Option<String>,
        handler: &impl ActionHandler,
    ) -> ActionInfo {
        let name = handler.name().to_string();
        let id = match source {
            ActionSource::Builtin => builtin_tool_id(&name),
            ActionSource::Mcp => {
                let server_name = server_name.as_deref().unwrap_or_default();
                mcp_tool_id(server_name, &name)
            }
        };

        ActionInfo {
            id,
            name,
            source,
            server_name,
            description: handler.description().to_string(),
            parameters: handler.parameters(),
            needs_feedback: handler.needs_feedback(),
            risk_tags: handler.risk_tags(),
            permission_level: handler.permission_level(),
        }
    }

    fn remove_alias_mapping(&mut self, info: &ActionInfo) {
        let should_remove = if let Some(ids) = self.alias_to_ids.get_mut(&info.name) {
            ids.retain(|id| id != &info.id);
            ids.is_empty()
        } else {
            false
        };

        if should_remove {
            self.alias_to_ids.remove(&info.name);
        }
    }

    fn insert_entry(&mut self, info: ActionInfo, handler: Arc<dyn ActionHandler>) {
        if let Some(old_entry) = self.entries_by_id.remove(&info.id) {
            self.remove_alias_mapping(&old_entry.info);
            if old_entry.info.source == ActionSource::Mcp {
                self.mcp_tool_ids.remove(&old_entry.info.id);
            }
        }

        let alias_ids = self.alias_to_ids.entry(info.name.clone()).or_default();
        if !alias_ids.iter().any(|id| id == &info.id) {
            alias_ids.push(info.id.clone());
            alias_ids.sort();
        }

        if info.source == ActionSource::Mcp {
            self.mcp_tool_ids.insert(info.id.clone());
        }

        println!("[Tools] Registered: {} ({})", info.id, info.name);
        self.entries_by_id
            .insert(info.id.clone(), ActionEntry { info, handler });
    }

    fn resolve_entry(&self, name_or_id: &str) -> Result<&ActionEntry, ActionError> {
        if let Some(entry) = self.entries_by_id.get(name_or_id) {
            return Ok(entry);
        }

        let Some(ids) = self.alias_to_ids.get(name_or_id) else {
            return Err(ActionError(format!("Unknown tool: {}", name_or_id)));
        };

        if ids.len() == 1 {
            let id = &ids[0];
            return self
                .entries_by_id
                .get(id)
                .ok_or_else(|| ActionError(format!("Unknown tool: {}", name_or_id)));
        }

        Err(ActionError(format!(
            "Ambiguous tool '{}'. Use one of: {}",
            name_or_id,
            ids.join(", ")
        )))
    }

    /// Register a built-in action handler.
    pub fn register(&mut self, handler: impl ActionHandler + 'static) {
        let info = Self::make_action_info(ActionSource::Builtin, None, &handler);
        self.insert_entry(info, Arc::new(handler));
    }

    /// Register an MCP tool handler (tracked separately for cleanup).
    pub fn register_mcp(
        &mut self,
        server_name: impl Into<String>,
        handler: impl ActionHandler + 'static,
    ) {
        let info = Self::make_action_info(ActionSource::Mcp, Some(server_name.into()), &handler);
        self.insert_entry(info, Arc::new(handler));
    }

    /// Remove all previously registered MCP tools.
    pub fn clear_mcp_tools(&mut self) {
        let ids: Vec<_> = self.mcp_tool_ids.drain().collect();
        for id in ids {
            if let Some(entry) = self.entries_by_id.remove(&id) {
                self.remove_alias_mapping(&entry.info);
            }
        }
    }

    pub fn resolve_action(&self, name_or_id: &str) -> Result<ActionInfo, ActionError> {
        Ok(self.resolve_entry(name_or_id)?.info.clone())
    }

    pub fn resolve_action_for_execution(
        &self,
        name_or_id: &str,
    ) -> Result<(ActionInfo, Arc<dyn ActionHandler>), ActionError> {
        let entry = self.resolve_entry(name_or_id)?;
        Ok((entry.info.clone(), Arc::clone(&entry.handler)))
    }

    pub fn migrate_tool_settings(&self, settings: &mut ToolSettings) -> bool {
        let existing = settings.enabled_tools.clone();
        let mut changed = false;

        for (key, enabled) in existing {
            if self.entries_by_id.contains_key(&key) {
                continue;
            }

            let Ok(info) = self.resolve_action(&key) else {
                continue;
            };

            if info.source != ActionSource::Builtin {
                continue;
            }

            settings.enabled_tools.remove(&key);
            settings.enabled_tools.entry(info.id).or_insert(enabled);
            changed = true;
        }

        changed
    }

    pub async fn execute(
        &self,
        name_or_id: &str,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let entry = self.resolve_entry(name_or_id)?;
        entry.handler.execute(args, ctx).await
    }

    /// Check if a named action needs its result fed back to the LLM.
    /// Unknown or ambiguous tools default to true — safer to do an extra round
    /// than to swallow results the LLM needs.
    pub fn needs_feedback(&self, name_or_id: &str) -> bool {
        self.resolve_entry(name_or_id)
            .map(|entry| entry.info.needs_feedback)
            .unwrap_or(true)
    }

    /// List all registered actions (for frontend / prompt generation).
    pub fn list_actions(&self) -> Vec<ActionInfo> {
        let mut actions: Vec<_> = self
            .entries_by_id
            .values()
            .map(|entry| entry.info.clone())
            .collect();
        actions.sort_by(|a, b| a.id.cmp(&b.id));
        actions
    }

    pub fn list_actions_for_prompt(&self, memory_enabled: bool) -> Vec<ActionInfo> {
        self.list_actions()
            .into_iter()
            .filter(|action| memory_enabled || !MEMORY_ACTIONS.contains(&action.name.as_str()))
            .collect()
    }

    pub fn list_builtin_actions(&self) -> Vec<ActionInfo> {
        let mut actions: Vec<_> = self
            .entries_by_id
            .values()
            .filter(|entry| entry.info.source == ActionSource::Builtin)
            .map(|entry| entry.info.clone())
            .collect();
        actions.sort_by(|a, b| a.id.cmp(&b.id));
        actions
    }

    pub fn list_actions_for_prompt_with_settings(
        &self,
        memory_enabled: bool,
        tool_settings: &ToolSettings,
    ) -> Vec<ActionInfo> {
        self.list_actions_for_prompt(memory_enabled)
            .into_iter()
            .filter(|action| tool_settings.is_enabled(&action.id))
            .collect()
    }

    pub fn list_tools_for_llm(&self, memory_enabled: bool) -> Vec<LlmToolDefinition> {
        self.list_actions_for_prompt(memory_enabled)
            .into_iter()
            .map(|action| LlmToolDefinition {
                name: action.id,
                description: action.description,
                parameters: action
                    .parameters
                    .into_iter()
                    .map(|param| LlmToolParam {
                        name: param.name,
                        description: param.description,
                        required: param.required,
                    })
                    .collect(),
            })
            .collect()
    }

    pub fn list_tools_for_llm_with_settings(
        &self,
        memory_enabled: bool,
        tool_settings: &ToolSettings,
    ) -> Vec<LlmToolDefinition> {
        self.list_actions_for_prompt_with_settings(memory_enabled, tool_settings)
            .into_iter()
            .map(|action| LlmToolDefinition {
                name: action.id,
                description: action.description,
                parameters: action
                    .parameters
                    .into_iter()
                    .map(|param| LlmToolParam {
                        name: param.name,
                        description: param.description,
                        required: param.required,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Generate the prompt instruction block describing available tools.
    pub fn generate_tool_prompt(&self) -> String {
        self.generate_tool_prompt_for_prompt(true)
    }

    pub fn generate_tool_prompt_for_prompt(&self, memory_enabled: bool) -> String {
        let actions = self.list_actions_for_prompt(memory_enabled);
        self.generate_tool_prompt_from_actions(actions)
    }

    pub fn generate_tool_prompt_for_prompt_with_settings(
        &self,
        memory_enabled: bool,
        tool_settings: &ToolSettings,
    ) -> String {
        let actions = self.list_actions_for_prompt_with_settings(memory_enabled, tool_settings);
        self.generate_tool_prompt_from_actions(actions)
    }

    fn format_action_label(action: &ActionInfo) -> String {
        match action.source {
            ActionSource::Builtin => format!("{} (built-in: {})", action.id, action.name),
            ActionSource::Mcp => format!(
                "{} (mcp/{})",
                action.id,
                action.server_name.as_deref().unwrap_or("unknown")
            ),
        }
    }

    fn generate_tool_prompt_from_actions(&self, actions: Vec<ActionInfo>) -> String {
        if actions.is_empty() {
            return String::new();
        }

        let mut lines = vec![
            "You have the following tools available. To use a tool, include a tag in your response:".to_string(),
            "[TOOL_CALL:canonical_tool_id|param1=value1|param2=value2]".to_string(),
            String::new(),
            "Use the exact canonical tool id shown below when calling a tool.".to_string(),
            "If you see both built-in and MCP tools with similar names, prefer the exact id instead of guessing by alias.".to_string(),
            "When you use a tool, the system will execute it and return the result to you. You can then use the result to continue your response naturally.".to_string(),
            "For information-retrieval tools (e.g. get_time, search_memory), wait for the result before answering the user's question.".to_string(),
            "For side-effect tools (e.g. play_cue), the system will confirm execution; you do not need to elaborate further.".to_string(),
            String::new(),
            "Available tools:".to_string(),
        ];

        for action in &actions {
            let label = Self::format_action_label(action);
            if action.parameters.is_empty() {
                lines.push(format!(
                    "- {}: {}. No parameters.",
                    label, action.description
                ));
            } else {
                let params: Vec<String> = action
                    .parameters
                    .iter()
                    .map(|p| {
                        let req = if p.required { "required" } else { "optional" };
                        format!("{}({}, {})", p.name, p.description, req)
                    })
                    .collect();
                lines.push(format!(
                    "- {}: {}. Params: {}",
                    label,
                    action.description,
                    params.join(", ")
                ));
            }
        }

        lines.push(String::new());
        lines.push(
            "You may include multiple [TOOL_CALL:...] tags in a single response.".to_string(),
        );
        lines.push(
            "Only use tools when they are genuinely helpful for the user's request.".to_string(),
        );

        lines.join("\n")
    }

    pub fn generate_native_tool_prompt_for_prompt(&self, memory_enabled: bool) -> String {
        let actions = self.list_actions_for_prompt(memory_enabled);
        if actions.is_empty() {
            return String::new();
        }

        let mut lines = vec![
            "You have native tools available.".to_string(),
            "Use the tool calling interface when a tool is genuinely helpful.".to_string(),
            "Call tools by their exact canonical tool id.".to_string(),
            "Do not write pseudo tool tags like [TOOL_CALL:...]; call the tool directly.".to_string(),
            "If the current reply clearly fits an existing cue, call play_cue at an appropriate moment.".to_string(),
            "Do not merely describe an expression or animation in prose when play_cue is appropriate.".to_string(),
            String::new(),
            "Available tools:".to_string(),
        ];

        for action in &actions {
            let label = Self::format_action_label(action);
            if action.parameters.is_empty() {
                lines.push(format!(
                    "- {}: {}. No parameters.",
                    label, action.description
                ));
            } else {
                let params: Vec<String> = action
                    .parameters
                    .iter()
                    .map(|p| {
                        let req = if p.required { "required" } else { "optional" };
                        format!("{}({}, {})", p.name, p.description, req)
                    })
                    .collect();
                lines.push(format!(
                    "- {}: {}. Params: {}",
                    label,
                    action.description,
                    params.join(", ")
                ));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAction {
        name: &'static str,
        description: &'static str,
        needs_feedback: bool,
    }

    #[async_trait]
    impl ActionHandler for TestAction {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn parameters(&self) -> Vec<ActionParam> {
            vec![]
        }

        fn needs_feedback(&self) -> bool {
            self.needs_feedback
        }

        async fn execute(
            &self,
            _args: HashMap<String, String>,
            _ctx: ActionContext,
        ) -> Result<ActionResult, ActionError> {
            Ok(ActionResult::ok("ok"))
        }
    }

    fn sample_builtin_action() -> TestAction {
        TestAction {
            name: "search_memory",
            description: "Search memory",
            needs_feedback: true,
        }
    }

    fn sample_mcp_action() -> TestAction {
        TestAction {
            name: "read_file",
            description: "Read file",
            needs_feedback: true,
        }
    }

    // ── ActionResult constructors ─────────────────────────

    #[test]
    fn test_action_result_ok() {
        let r = ActionResult::ok("success");
        assert!(r.success);
        assert_eq!(r.message, "success");
        assert!(r.data.is_none());
    }

    #[test]
    fn test_action_result_ok_with_data() {
        let r = ActionResult::ok_with_data("done", serde_json::json!({"x": 1}));
        assert!(r.success);
        assert!(r.data.is_some());
    }

    #[test]
    fn test_action_result_err() {
        let r = ActionResult::err("oops");
        assert!(!r.success);
        assert_eq!(r.message, "oops");
    }

    // ── Metadata defaults ────────────────────────────────

    #[test]
    fn test_make_action_info_sets_default_metadata_for_builtin() {
        let action = ActionRegistry::make_action_info(ActionSource::Builtin, None, &sample_builtin_action());

        assert_eq!(action.risk_tags, vec![ActionRiskTag::Read]);
        assert_eq!(action.permission_level, ActionPermissionLevel::Safe);
        assert_eq!(action.source, ActionSource::Builtin);
        assert_eq!(action.server_name, None);
    }

    #[test]
    fn test_make_action_info_sets_default_metadata_for_mcp() {
        let action = ActionRegistry::make_action_info(
            ActionSource::Mcp,
            Some("filesystem".to_string()),
            &sample_mcp_action(),
        );

        assert_eq!(action.risk_tags, vec![ActionRiskTag::Read]);
        assert_eq!(action.permission_level, ActionPermissionLevel::Safe);
        assert_eq!(action.source, ActionSource::Mcp);
        assert_eq!(action.server_name.as_deref(), Some("filesystem"));
    }

    #[test]
    fn test_action_info_serialization_includes_metadata_fields() {
        let action = ActionRegistry::make_action_info(ActionSource::Builtin, None, &sample_builtin_action());
        let value = serde_json::to_value(&action).expect("action info should serialize");

        assert_eq!(value.get("risk_tags"), Some(&serde_json::json!(["read"])));
        assert_eq!(value.get("permission_level"), Some(&serde_json::json!("safe")));
    }

    #[test]
    fn test_builtin_action_metadata_can_differentiate_read_and_side_effect_tools() {
        let mut reg = ActionRegistry::new();
        crate::actions::builtin::register_builtins(&mut reg);

        let search_memory = reg.resolve_action("search_memory").unwrap();
        let play_cue = reg.resolve_action("play_cue").unwrap();

        assert_eq!(search_memory.risk_tags, vec![ActionRiskTag::Read]);
        assert_eq!(search_memory.permission_level, ActionPermissionLevel::Safe);
        assert_eq!(play_cue.risk_tags, vec![ActionRiskTag::Write]);
        assert_eq!(play_cue.permission_level, ActionPermissionLevel::Elevated);
    }

    // ── Registry without handlers ─────────────────────────

    #[test]
    fn test_registry_empty_list() {
        let reg = ActionRegistry::new();
        assert!(reg.list_actions().is_empty());
    }

    #[test]
    fn test_registry_generate_tool_prompt_empty() {
        let reg = ActionRegistry::new();
        assert_eq!(reg.generate_tool_prompt(), "");
    }

    #[test]
    fn test_registry_needs_feedback_unknown_defaults_true() {
        let reg = ActionRegistry::new();
        assert!(reg.needs_feedback("nonexistent"));
    }

    #[test]
    fn test_register_builtin_uses_canonical_id() {
        let mut reg = ActionRegistry::new();
        reg.register(TestAction {
            name: "get_time",
            description: "Get time",
            needs_feedback: true,
        });

        let action = reg.resolve_action("get_time").unwrap();
        assert_eq!(action.id, "builtin__get_time");
        assert_eq!(action.source, ActionSource::Builtin);
        assert_eq!(action.risk_tags, vec![ActionRiskTag::Read]);
        assert_eq!(action.permission_level, ActionPermissionLevel::Safe);
    }

    #[test]
    fn test_register_mcp_uses_canonical_id() {
        let mut reg = ActionRegistry::new();
        reg.register_mcp(
            "filesystem",
            TestAction {
                name: "read_file",
                description: "Read file",
                needs_feedback: true,
            },
        );

        let action = reg.resolve_action("read_file").unwrap();
        assert_eq!(action.id, "mcp__filesystem__read_file");
        assert_eq!(action.source, ActionSource::Mcp);
        assert_eq!(action.server_name.as_deref(), Some("filesystem"));
        assert_eq!(action.risk_tags, vec![ActionRiskTag::Read]);
        assert_eq!(action.permission_level, ActionPermissionLevel::Safe);
    }

    #[test]
    fn test_clear_mcp_tools_keeps_builtin() {
        let mut reg = ActionRegistry::new();
        reg.register(TestAction {
            name: "play_cue",
            description: "Play cue",
            needs_feedback: false,
        });
        reg.register_mcp(
            "server_a",
            TestAction {
                name: "play_cue",
                description: "Remote play cue",
                needs_feedback: true,
            },
        );

        reg.clear_mcp_tools();

        let builtin = reg.resolve_action("builtin__play_cue").unwrap();
        assert_eq!(builtin.source, ActionSource::Builtin);
        assert!(reg.resolve_action("mcp__server_a__play_cue").is_err());
        assert_eq!(reg.resolve_action("play_cue").unwrap().id, "builtin__play_cue");
    }

    #[test]
    fn test_resolve_alias_when_unique() {
        let mut reg = ActionRegistry::new();
        reg.register(TestAction {
            name: "send_notification",
            description: "Notify",
            needs_feedback: false,
        });

        let action = reg.resolve_action("send_notification").unwrap();
        assert_eq!(action.id, "builtin__send_notification");
    }

    #[test]
    fn test_resolve_alias_when_ambiguous() {
        let mut reg = ActionRegistry::new();
        reg.register(TestAction {
            name: "search",
            description: "Builtin search",
            needs_feedback: true,
        });
        reg.register_mcp(
            "server_a",
            TestAction {
                name: "search",
                description: "Server A search",
                needs_feedback: true,
            },
        );

        let err = reg.resolve_action("search").unwrap_err();
        assert!(err.0.contains("Ambiguous tool 'search'"));
        assert!(err.0.contains("builtin__search"));
        assert!(err.0.contains("mcp__server_a__search"));
    }

    #[test]
    fn test_migrate_tool_settings_to_builtin_canonical_id() {
        let mut reg = ActionRegistry::new();
        reg.register(TestAction {
            name: "get_time",
            description: "Get time",
            needs_feedback: true,
        });

        let mut settings = ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::from([("get_time".to_string(), false)]),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: Vec::new(),
        };

        let changed = reg.migrate_tool_settings(&mut settings);
        assert!(changed);
        assert_eq!(settings.enabled_tools.get("builtin__get_time"), Some(&false));
        assert!(!settings.enabled_tools.contains_key("get_time"));
    }
}
