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
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register an action handler.
    pub fn register(&mut self, handler: impl ActionHandler + 'static) {
        let name = handler.name().to_string();
        println!("[Actions] Registered: {}", name);
        self.handlers.insert(name, Arc::new(handler));
    }

    /// Execute a named action.
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

    /// Generate the prompt instruction block describing available tools.
    pub fn generate_tool_prompt(&self) -> String {
        let actions = self.list_actions();
        if actions.is_empty() {
            return String::new();
        }

        let mut lines = vec![
            "You have the following tools available. To use a tool, include a tag in your response:".to_string(),
            "[TOOL_CALL:tool_name|param1=value1|param2=value2]".to_string(),
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
        lines.push("You may include multiple [TOOL_CALL:...] tags. Place them BEFORE the [EMOTION:...] tag.".to_string());
        lines.push(
            "Only use tools when they are genuinely helpful for the user's request.".to_string(),
        );

        lines.join("\n")
    }
}
