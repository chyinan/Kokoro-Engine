//! MCP Client — protocol-level wrapper over a transport.
//!
//! Handles MCP lifecycle: initialize → list_tools → call_tool.
//! Wraps any `McpTransport` implementation.

use super::transport::McpTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

// ── MCP Protocol Types ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for tool input
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    #[serde(default)]
    pub content: Vec<McpContentPart>,
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "resource")]
    Resource { resource: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(default)]
    pub resources: Option<Value>,
    #[serde(default)]
    pub prompts: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

// ── MCP Client ──────────────────────────────────────────

pub struct McpClient {
    transport: Arc<dyn McpTransport>,
    server_info: Option<ServerInfo>,
    tools: Vec<McpToolInfo>,
}

impl McpClient {
    /// Create a new MCP client wrapping the given transport.
    pub fn new(transport: Arc<dyn McpTransport>) -> Self {
        Self {
            transport,
            server_info: None,
            tools: Vec::new(),
        }
    }

    /// Perform MCP handshake: initialize + list tools.
    pub async fn connect(&mut self) -> Result<(), String> {
        // Step 1: Initialize
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "kokoro-engine",
                "version": "0.1.0"
            }
        });

        let result = self
            .transport
            .request("initialize", Some(init_params))
            .await?;
        let init: InitializeResult = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse initialize response: {}", e))?;

        self.server_info = Some(init.server_info.clone());
        println!(
            "[MCP] Connected to {} v{}",
            init.server_info.name,
            init.server_info.version.as_deref().unwrap_or("?")
        );

        // Step 2: Send initialized notification
        let _ = self
            .transport
            .notify("notifications/initialized", None)
            .await;

        // Step 3: List tools (if server supports tools)
        if init.capabilities.tools.is_some() {
            self.refresh_tools().await?;
        }

        Ok(())
    }

    /// Refresh the tool list from the server.
    pub async fn refresh_tools(&mut self) -> Result<(), String> {
        let result = self.transport.request("tools/list", None).await?;

        #[derive(Deserialize)]
        struct ToolListResult {
            tools: Vec<McpToolInfo>,
        }

        let list: ToolListResult = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse tools/list: {}", e))?;

        println!("[MCP] Discovered {} tools", list.tools.len());
        for tool in &list.tools {
            println!(
                "  - {}: {}",
                tool.name,
                tool.description.as_deref().unwrap_or("(no description)")
            );
        }

        self.tools = list.tools;
        Ok(())
    }

    /// Call a tool by name with the given arguments.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpToolResult, String> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.transport.request("tools/call", Some(params)).await?;
        let tool_result: McpToolResult = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse tool result: {}", e))?;

        Ok(tool_result)
    }

    /// Get the list of available tools.
    pub fn tools(&self) -> &[McpToolInfo] {
        &self.tools
    }

    /// Get server info (available after connect).
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Check if transport is still alive.
    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Shutdown the client and underlying transport.
    pub async fn shutdown(&self) -> Result<(), String> {
        self.transport.shutdown().await
    }
}
