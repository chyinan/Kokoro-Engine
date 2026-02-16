//! MCP ↔ ActionRegistry Bridge
//!
//! Wraps each MCP tool as an `ActionHandler` so the LLM can invoke
//! MCP tools through the same `[TOOL_CALL:...]` mechanism as builtins.

use super::manager::McpManager;
use crate::actions::registry::{
    ActionContext, ActionError, ActionHandler, ActionParam, ActionResult,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// An ActionHandler that delegates to an MCP server tool.
pub struct McpToolHandler {
    /// Which MCP server this tool belongs to.
    pub server_name: String,
    /// The tool name on the MCP server.
    pub tool_name: String,
    /// Human-readable description for LLM prompt.
    pub description: String,
    /// JSON Schema for tool input (from MCP server).
    pub input_schema: Option<serde_json::Value>,
    /// Shared reference to the MCP manager.
    pub manager: Arc<Mutex<McpManager>>,
}

impl McpToolHandler {
    /// Parse JSON Schema properties into ActionParam list.
    fn schema_to_params(schema: &Option<serde_json::Value>) -> Vec<ActionParam> {
        let schema = match schema {
            Some(s) => s,
            None => return Vec::new(),
        };

        let properties = match schema.get("properties").and_then(|p| p.as_object()) {
            Some(p) => p,
            None => return Vec::new(),
        };

        let required: Vec<String> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        properties
            .iter()
            .map(|(name, prop)| {
                let description = prop
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                ActionParam {
                    name: name.clone(),
                    description,
                    required: required.contains(name),
                }
            })
            .collect()
    }
}

#[async_trait]
impl ActionHandler for McpToolHandler {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Vec<ActionParam> {
        Self::schema_to_params(&self.input_schema)
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        _ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        // Convert HashMap<String, String> to JSON object
        let arguments = serde_json::Value::Object(
            args.into_iter()
                .map(|(k, v)| {
                    // Try to parse as JSON value, fall back to string
                    let val = serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v));
                    (k, val)
                })
                .collect(),
        );

        let manager = self.manager.lock().await;
        let result = manager
            .call_tool(&self.server_name, &self.tool_name, arguments)
            .await
            .map_err(|e| ActionError(e))?;

        // Convert MCP result to ActionResult
        if result.is_error {
            let error_text = result
                .content
                .iter()
                .filter_map(|c| match c {
                    super::client::McpContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ActionResult::err(error_text))
        } else {
            let text = result
                .content
                .iter()
                .filter_map(|c| match c {
                    super::client::McpContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ActionResult::ok(text))
        }
    }
}

/// Register all MCP tools into the ActionRegistry.
/// Called after McpManager connects to servers.
pub async fn register_mcp_tools(
    manager: &Arc<Mutex<McpManager>>,
    registry: &tokio::sync::RwLock<crate::actions::ActionRegistry>,
) {
    let mgr = manager.lock().await;
    let tools = mgr.all_tools().await;
    drop(mgr); // Release lock before acquiring registry write lock

    let mut reg = registry.write().await;
    for (server_name, tool) in tools {
        let handler = McpToolHandler {
            server_name,
            tool_name: tool.name.clone(),
            description: tool.description.unwrap_or_default(),
            input_schema: tool.input_schema,
            manager: manager.clone(),
        };
        reg.register(handler);
    }
}
