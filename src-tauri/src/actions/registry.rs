//! Action Registry — core framework for tool calling.
//!
//! Provides a registry of actions that the LLM can invoke via `[TOOL_CALL:name|args]` tags.
//! Actions are registered at startup and can be invoked by the chat pipeline.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

/// Metadata for a registered action (returned to frontend / LLM prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInfo {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ActionParam>,
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
    /// Return `false` (default) for side-effect tools (play_cue, play_sound, etc.).
    fn needs_feedback(&self) -> bool {
        false
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
    handlers: HashMap<String, Arc<dyn ActionHandler>>,
    mcp_tool_names: std::collections::HashSet<String>,
}

const MEMORY_ACTIONS: &[&str] = &["search_memory", "store_memory", "forget_memory"];

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            mcp_tool_names: std::collections::HashSet::new(),
        }
    }

    /// Register an action handler.
    pub fn register(&mut self, handler: impl ActionHandler + 'static) {
        let name = handler.name().to_string();
        println!("[Actions] Registered: {}", name);
        self.handlers.insert(name, Arc::new(handler));
    }

    /// Register an MCP tool handler (tracked separately for cleanup).
    pub fn register_mcp(&mut self, handler: impl ActionHandler + 'static) {
        let name = handler.name().to_string();
        self.mcp_tool_names.insert(name.clone());
        self.handlers.insert(name, Arc::new(handler));
    }

    /// Remove all previously registered MCP tools.
    pub fn clear_mcp_tools(&mut self) {
        for name in self.mcp_tool_names.drain() {
            self.handlers.remove(&name);
        }
    }

    pub async fn execute(
        &self,
        name: &str,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| ActionError(format!("Unknown action: {}", name)))?;

        handler.execute(args, ctx).await
    }

    /// Check if a named action needs its result fed back to the LLM.
    /// Unknown tools (e.g. MCP tools) default to true — safer to do an extra round
    /// than to swallow results the LLM needs.
    pub fn needs_feedback(&self, name: &str) -> bool {
        self.handlers
            .get(name)
            .map(|h| h.needs_feedback())
            .unwrap_or(true)
    }

    /// List all registered actions (for LLM prompt injection).
    pub fn list_actions(&self) -> Vec<ActionInfo> {
        self.handlers
            .values()
            .map(|h| ActionInfo {
                name: h.name().to_string(),
                description: h.description().to_string(),
                parameters: h.parameters(),
            })
            .collect()
    }

    pub fn list_actions_for_prompt(&self, memory_enabled: bool) -> Vec<ActionInfo> {
        self.handlers
            .values()
            .filter(|h| memory_enabled || !MEMORY_ACTIONS.contains(&h.name()))
            .map(|h| ActionInfo {
                name: h.name().to_string(),
                description: h.description().to_string(),
                parameters: h.parameters(),
            })
            .collect()
    }

    /// Generate the prompt instruction block describing available tools.
    pub fn generate_tool_prompt(&self) -> String {
        self.generate_tool_prompt_for_prompt(true)
    }

    pub fn generate_tool_prompt_for_prompt(&self, memory_enabled: bool) -> String {
        let actions = self.list_actions_for_prompt(memory_enabled);
        if actions.is_empty() {
            return String::new();
        }

        let mut lines = vec![
            "You have the following tools available. To use a tool, include a tag in your response:".to_string(),
            "[TOOL_CALL:tool_name|param1=value1|param2=value2]".to_string(),
            String::new(),
            "When you use a tool, the system will execute it and return the result to you. You can then use the result to continue your response naturally.".to_string(),
            "For information-retrieval tools (e.g. get_time, search_memory), wait for the result before answering the user's question.".to_string(),
            "For side-effect tools (e.g. play_cue), the system will confirm execution; you do not need to elaborate further.".to_string(),
            String::new(),
            "Available tools:".to_string(),
        ];

        for action in &actions {
            if action.parameters.is_empty() {
                lines.push(format!(
                    "- {}: {}. No parameters.",
                    action.name, action.description
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
                    action.name,
                    action.description,
                    params.join(", ")
                ));
            }
        }

        lines.push(String::new());
        lines.push("You may include multiple [TOOL_CALL:...] tags in a single response.".to_string());
        lines.push(
            "Only use tools when they are genuinely helpful for the user's request.".to_string(),
        );

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
