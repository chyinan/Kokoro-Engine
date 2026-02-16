//! MCP Manager — manages multiple MCP server connections.
//!
//! Loads server configs, starts/stops servers, aggregates tools.

use super::client::McpClient;
use super::transport::StdioTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Config Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for this server.
    pub name: String,
    /// Command to spawn (e.g., "npx", "python", "node").
    pub command: String,
    /// Arguments to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether to auto-connect on startup.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub server_version: Option<String>,
    /// "connected" | "connecting" | "disconnected"
    pub status: String,
    /// Error message if connection failed.
    pub error: Option<String>,
}

// ── Manager ─────────────────────────────────────────────

pub struct McpManager {
    configs: Vec<McpServerConfig>,
    clients: HashMap<String, Arc<Mutex<McpClient>>>,
    config_path: String,
    /// Servers currently being connected to in the background.
    pending_connections: HashSet<String>,
    /// Error messages from failed connection attempts.
    connection_errors: HashMap<String, String>,
}

impl McpManager {
    pub fn new(config_path: &str) -> Self {
        Self {
            configs: Vec::new(),
            clients: HashMap::new(),
            config_path: config_path.to_string(),
            pending_connections: HashSet::new(),
            connection_errors: HashMap::new(),
        }
    }

    /// Mark a server as currently connecting.
    pub fn mark_connecting(&mut self, name: &str) {
        self.pending_connections.insert(name.to_string());
        self.connection_errors.remove(name);
    }

    /// Clear connecting state (on success or failure).
    pub fn clear_connecting(&mut self, name: &str) {
        self.pending_connections.remove(name);
    }

    /// Record a connection error for a server.
    pub fn set_connection_error(&mut self, name: &str, error: String) {
        self.connection_errors.insert(name.to_string(), error);
    }

    /// Load server configs from disk.
    pub fn load_configs(&mut self) {
        let path = Path::new(&self.config_path);
        if !path.exists() {
            println!(
                "[MCP] No config file at {}, starting empty",
                self.config_path
            );
            return;
        }

        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<McpServerConfig>>(&content) {
                Ok(configs) => {
                    println!("[MCP] Loaded {} server configs", configs.len());
                    self.configs = configs;
                }
                Err(e) => eprintln!("[MCP] Failed to parse config: {}", e),
            },
            Err(e) => eprintln!("[MCP] Failed to read config: {}", e),
        }
    }

    /// Save current configs to disk.
    pub fn save_configs(&self) -> Result<(), String> {
        let content = serde_json::to_string_pretty(&self.configs)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(&self.config_path, content).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Connect to all enabled servers.
    pub async fn connect_all(&mut self) {
        let configs: Vec<McpServerConfig> =
            self.configs.iter().filter(|c| c.enabled).cloned().collect();

        for config in configs {
            if let Err(e) = self.connect_server(&config).await {
                eprintln!("[MCP] Failed to connect '{}': {}", config.name, e);
            }
        }
    }

    /// Connect to a single server.
    pub async fn connect_server(&mut self, config: &McpServerConfig) -> Result<(), String> {
        println!("[MCP] Connecting to '{}'...", config.name);

        let transport =
            StdioTransport::spawn(&config.command, &config.args, Some(&config.env)).await?;

        let mut client = McpClient::new(Arc::new(transport));
        client.connect().await?;

        self.clients
            .insert(config.name.clone(), Arc::new(Mutex::new(client)));
        Ok(())
    }

    /// Disconnect and remove a server.
    pub async fn disconnect_server(&mut self, name: &str) -> Result<(), String> {
        if let Some(client) = self.clients.remove(name) {
            client.lock().await.shutdown().await?;
        }
        Ok(())
    }

    /// Add a new server config and optionally connect.
    pub async fn add_server(
        &mut self,
        config: McpServerConfig,
        connect: bool,
    ) -> Result<(), String> {
        // Remove existing with same name
        self.configs.retain(|c| c.name != config.name);
        self.configs.push(config.clone());
        self.save_configs()?;

        if connect && config.enabled {
            self.connect_server(&config).await?;
        }
        Ok(())
    }

    /// Remove a server config and disconnect.
    pub async fn remove_server(&mut self, name: &str) -> Result<(), String> {
        self.disconnect_server(name).await?;
        self.configs.retain(|c| c.name != name);
        self.save_configs()?;
        Ok(())
    }

    /// Get status of all configured servers.
    pub async fn list_status(&self) -> Vec<McpServerStatus> {
        let mut statuses = Vec::new();

        for config in &self.configs {
            let (connected, tool_count, version) =
                if let Some(client) = self.clients.get(&config.name) {
                    let c = client.lock().await;
                    (
                        c.is_connected(),
                        c.tools().len(),
                        c.server_info().and_then(|s| s.version.clone()),
                    )
                } else {
                    (false, 0, None)
                };

            let is_pending = self.pending_connections.contains(&config.name);
            let error = self.connection_errors.get(&config.name).cloned();

            let status = if connected {
                "connected".to_string()
            } else if is_pending {
                "connecting".to_string()
            } else {
                "disconnected".to_string()
            };

            statuses.push(McpServerStatus {
                name: config.name.clone(),
                connected,
                tool_count,
                server_version: version,
                status,
                error,
            });
        }

        statuses
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<super::client::McpToolResult, String> {
        let client = self
            .clients
            .get(server_name)
            .ok_or_else(|| format!("Server '{}' not connected", server_name))?;

        client.lock().await.call_tool(tool_name, arguments).await
    }

    /// Get all tools from all connected servers, keyed by (server_name, tool).
    pub async fn all_tools(&self) -> Vec<(String, super::client::McpToolInfo)> {
        let mut all = Vec::new();
        for (name, client) in &self.clients {
            let c = client.lock().await;
            for tool in c.tools() {
                all.push((name.clone(), tool.clone()));
            }
        }
        all
    }

    /// Get configs (for serialization to frontend).
    pub fn configs(&self) -> &[McpServerConfig] {
        &self.configs
    }

    /// Look up a server config by name.
    pub fn get_config(&self, name: &str) -> Option<McpServerConfig> {
        self.configs.iter().find(|c| c.name == name).cloned()
    }
}
