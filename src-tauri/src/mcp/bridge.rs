// pattern: Imperative Shell
//! MCP ↔ ActionRegistry Bridge
//!
//! Wraps each MCP tool as an `ActionHandler` so the LLM can invoke
//! MCP tools through the same `[TOOL_CALL:...]` mechanism as builtins.

use super::manager::McpManager;
use crate::actions::registry::{
    ActionContext, ActionError, ActionHandler, ActionParam, ActionPermissionLevel,
    ActionResult, ActionRiskTag,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::sync::OnceLock;

static MCP_REFRESH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

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

    fn needs_feedback(&self) -> bool {
        true
    }

    /// MCP servers are external code and their schemas are not a trusted
    /// authorization source. Keep every unknown tool behind the strictest
    /// shared action policy until a user-facing capability grant exists.
    fn risk_tags(&self) -> Vec<ActionRiskTag> {
        vec![
            ActionRiskTag::Read,
            ActionRiskTag::Write,
            ActionRiskTag::External,
            ActionRiskTag::Sensitive,
        ]
    }

    fn permission_level(&self) -> ActionPermissionLevel {
        ActionPermissionLevel::Elevated
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

        let client = {
            let manager = self.manager.lock().await;
            manager
                .client_handle(&self.server_name)
                .map_err(ActionError)?
        };
        let result = client
            .lock()
            .await
            .call_tool(&self.tool_name, arguments)
            .await
            .map_err(ActionError)?;

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
    // Serialize refresh snapshots and publication. Without this, an older
    // refresh that captured a soon-to-be-removed client could publish after a
    // newer remove/reconnect refresh and resurrect stale actions.
    let _refresh_guard = MCP_REFRESH_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;

    // Clone only the enabled client handles while holding the manager lock.
    // Listing tools acquires each client lock and may contend with a remote
    // call, so all awaits happen after the global manager lock is released.
    let handles = {
        let mgr = manager.lock().await;
        mgr.tool_client_handles()
    };
    let mut tools = Vec::new();
    for (server_name, client) in handles {
        let client = client.lock().await;
        tools.extend(
            client
                .tools()
                .iter()
                .cloned()
                .map(|tool| (server_name.clone(), tool)),
        );
    }

    let mut reg = registry.write().await;
    reg.clear_mcp_tools();
    for (server_name, tool) in tools {
        let handler = McpToolHandler {
            server_name: server_name.clone(),
            tool_name: tool.name.clone(),
            description: tool.description.unwrap_or_default(),
            input_schema: tool.input_schema,
            manager: manager.clone(),
        };
        reg.register_mcp(server_name, handler);
    }
}
