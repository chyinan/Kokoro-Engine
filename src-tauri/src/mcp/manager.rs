// pattern: Imperative Shell
//! MCP Manager — manages multiple MCP server connections.
//!
//! Loads server configs, starts/stops servers, aggregates tools.

use super::client::McpClient;
use super::transport::{McpTransport, SseTransport, StdioTransport, StreamableHttpTransport};
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Config Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for this server.
    pub name: String,
    /// Transport type: "stdio" (default) or "streamable_http".
    #[serde(default = "default_transport_type", rename = "type")]
    pub transport_type: String,
    /// Command to spawn (for stdio transport).
    #[serde(default)]
    pub command: String,
    /// Arguments to the command (for stdio transport).
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables (for stdio transport).
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTP endpoint URL (for streamable_http transport).
    #[serde(default)]
    pub url: Option<String>,
    /// Whether to auto-connect on startup.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_transport_type() -> String {
    "stdio".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub enabled: bool,
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
    /// Servers currently being connected to in the background, keyed by the
    /// connection generation that owns the attempt.
    pending_connections: HashMap<String, u64>,
    /// Monotonic per-server generations invalidate late connection results.
    connection_generations: HashMap<String, u64>,
    /// Error messages from failed connection attempts.
    connection_errors: HashMap<String, String>,
}

impl McpManager {
    pub fn new(config_path: &str) -> Self {
        Self {
            configs: Vec::new(),
            clients: HashMap::new(),
            config_path: config_path.to_string(),
            pending_connections: HashMap::new(),
            connection_generations: HashMap::new(),
            connection_errors: HashMap::new(),
        }
    }

    /// Mark a server as currently connecting.
    pub fn mark_connecting(&mut self, name: &str) -> u64 {
        let generation = self
            .connection_generations
            .entry(name.to_string())
            .and_modify(|value| *value = value.wrapping_add(1))
            .or_insert(1);
        self.pending_connections
            .insert(name.to_string(), *generation);
        self.connection_errors.remove(name);
        *generation
    }

    /// Clear connecting state (on success or failure).
    pub fn clear_connecting(&mut self, name: &str) {
        self.pending_connections.remove(name);
    }

    pub fn clear_connecting_if_current(&mut self, name: &str, generation: u64) -> bool {
        if self.pending_connections.get(name).copied() == Some(generation) {
            self.pending_connections.remove(name);
            true
        } else {
            false
        }
    }

    /// Invalidate all pending work for a server. This is called before a
    /// disable/remove/replace operation so a late handshake cannot revive it.
    pub fn invalidate_connection(&mut self, name: &str) -> u64 {
        let generation = self
            .connection_generations
            .entry(name.to_string())
            .and_modify(|value| *value = value.wrapping_add(1))
            .or_insert(1);
        self.pending_connections.remove(name);
        *generation
    }

    pub fn is_connection_current(&self, name: &str, generation: u64) -> bool {
        self.pending_connections.get(name).copied() == Some(generation)
            && self.connection_generations.get(name).copied() == Some(generation)
            && self
                .configs
                .iter()
                .any(|config| config.name == name && config.enabled)
    }

    /// Commit a client only when the configuration that started the attempt
    /// is still the enabled, current configuration. A stale client is returned
    /// to its caller so its transport can be shut down outside the manager lock.
    pub fn commit_client(
        &mut self,
        name: String,
        generation: u64,
        client: McpClient,
    ) -> Result<(), McpClient> {
        if !self.is_connection_current(&name, generation) {
            return Err(client);
        }

        self.pending_connections.remove(&name);
        if let Some(previous) = self
            .clients
            .insert(name, Arc::new(Mutex::new(client)))
        {
            // The old client is dropped here. Callers that replace a live
            // connection should explicitly disconnect it before committing.
            drop(previous);
        }
        Ok(())
    }

    pub fn finish_connection_error(&mut self, name: &str, generation: u64, error: String) -> bool {
        if !self.is_connection_current(name, generation) {
            return false;
        }
        self.pending_connections.remove(name);
        self.connection_errors.insert(name.to_string(), error);
        true
    }

    /// Record a connection error for a server.
    pub fn set_connection_error(&mut self, name: &str, error: String) {
        self.connection_errors.insert(name.to_string(), error);
    }

    /// Load server configs from disk.
    pub fn load_configs(&mut self) {
        let path = Path::new(&self.config_path);
        if !path.exists() {
            tracing::info!(
                target: "mcp",
                "No config file at {}, starting empty",
                self.config_path
            );
            return;
        }

        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<McpServerConfig>>(&content) {
                Ok(configs) => {
                    tracing::info!(target: "mcp", "Loaded {} server configs", configs.len());
                    self.configs = configs;
                }
                Err(e) => tracing::error!(target: "mcp", "Failed to parse config: {}", e),
            },
            Err(e) => tracing::error!(target: "mcp", "Failed to read config: {}", e),
        }
    }

    /// Save current configs to disk.
    pub fn save_configs(&self) -> Result<(), KokoroError> {
        let content = serde_json::to_string_pretty(&self.configs)
            .map_err(|e| KokoroError::Config(format!("Serialize error: {}", e)))?;
        std::fs::write(&self.config_path, content)
            .map_err(|e| KokoroError::Io(format!("Write error: {}", e)))?;
        Ok(())
    }

    /// Connect to all enabled servers (sequential, holds lock the entire time).
    /// Prefer `prepare_connect_all` + per-server spawned tasks for non-blocking startup.
    pub async fn connect_all(&mut self) {
        let configs: Vec<McpServerConfig> =
            self.configs.iter().filter(|c| c.enabled).cloned().collect();

        for config in configs {
            if let Err(e) = self.connect_server(&config).await {
                tracing::error!(target: "mcp", "Failed to connect '{}': {}", config.name, e);
            }
        }
    }

    /// Return all enabled configs and mark them as "connecting".
    /// Caller should release the lock, then spawn per-server connection tasks.
    pub fn prepare_connect_all(&mut self) -> Vec<(McpServerConfig, u64)> {
        let configs: Vec<McpServerConfig> =
            self.configs.iter().filter(|c| c.enabled).cloned().collect();
        configs
            .into_iter()
            .map(|cfg| {
                let generation = self.mark_connecting(&cfg.name);
                (cfg, generation)
            })
            .collect()
    }

    /// Insert a pre-built client (used after lock-free connection).
    pub fn insert_client(&mut self, name: String, client: McpClient) {
        self.clients.insert(name, Arc::new(Mutex::new(client)));
    }

    /// Connect to a single server.
    pub async fn connect_server(&mut self, config: &McpServerConfig) -> Result<(), KokoroError> {
        let client = build_connected_client(config).await?;
        self.clients
            .insert(config.name.clone(), Arc::new(Mutex::new(client)));
        Ok(())
    }

    /// Detach a client without awaiting its transport shutdown. Callers that
    /// hold the manager mutex can then release it before waiting on I/O.
    pub fn take_client(&mut self, name: &str) -> Option<Arc<Mutex<McpClient>>> {
        self.clients.remove(name)
    }

    /// Replace a server configuration and return its previous client handle.
    ///
    /// This method only mutates in-memory state and persists the config.  The
    /// returned client must be shut down by the caller after releasing the
    /// manager mutex so a slow transport cannot block other MCP operations.
    pub fn upsert_server_config(
        &mut self,
        config: McpServerConfig,
    ) -> Result<Option<Arc<Mutex<McpClient>>>, KokoroError> {
        // Remove existing with same name and invalidate any in-flight
        // connection before the replacement is persisted.
        self.invalidate_connection(&config.name);
        let old_client = self.clients.remove(&config.name);
        self.configs.retain(|c| c.name != config.name);
        self.configs.push(config.clone());
        self.save_configs()?;
        Ok(old_client)
    }

    /// Remove a server configuration and return its client handle. The caller
    /// can release the manager mutex before awaiting transport shutdown.
    pub fn detach_server_for_removal(
        &mut self,
        name: &str,
    ) -> Result<Option<Arc<Mutex<McpClient>>>, KokoroError> {
        self.invalidate_connection(name);
        let client = self.clients.remove(name);
        self.configs.retain(|c| c.name != name);
        self.save_configs()?;
        Ok(client)
    }

    /// Persist a server's enabled state and detach a client when disabling.
    /// Transport shutdown is intentionally left to the caller so it can happen
    /// outside the manager mutex.
    pub fn set_server_enabled(
        &mut self,
        name: &str,
        enabled: bool,
    ) -> Result<Option<Arc<Mutex<McpClient>>>, KokoroError> {
        let config = self
            .configs
            .iter_mut()
            .find(|c| c.name == name)
            .ok_or_else(|| KokoroError::NotFound(format!("Server '{}' not found", name)))?;
        config.enabled = enabled;
        self.save_configs()?;

        if !enabled {
            self.invalidate_connection(name);
            return Ok(self.take_client(name));
        }
        Ok(None)
    }

    /// Get status of all configured servers.
    /// Only locks individual clients that are actually connected — pending /
    /// disconnected servers return immediately without extra lock contention.
    pub async fn list_status(&self) -> Vec<McpServerStatus> {
        let mut statuses = Vec::new();

        for config in &self.configs {
            let is_pending = self.pending_connections.contains_key(&config.name);
            let error = self.connection_errors.get(&config.name).cloned();

            // Fast path: if the server is still connecting or has no client,
            // skip the client lock entirely.
            if is_pending {
                statuses.push(McpServerStatus {
                    name: config.name.clone(),
                    enabled: config.enabled,
                    connected: false,
                    tool_count: 0,
                    server_version: None,
                    status: "connecting".to_string(),
                    error: None,
                });
                continue;
            }

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

            let status = if connected {
                "connected".to_string()
            } else {
                "disconnected".to_string()
            };

            statuses.push(McpServerStatus {
                name: config.name.clone(),
                enabled: config.enabled,
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
        let client = self.client_handle(server_name)?;
        let result = client.lock().await.call_tool(tool_name, arguments).await;
        result
    }

    /// Clone a validated client handle while the manager lock is held. Callers
    /// must release the manager lock before awaiting remote I/O on the client.
    pub fn client_handle(
        &self,
        server_name: &str,
    ) -> Result<Arc<Mutex<McpClient>>, String> {
        let config = self
            .configs
            .iter()
            .find(|config| config.name == server_name)
            .ok_or_else(|| format!("Server '{}' is not configured", server_name))?;
        if !config.enabled {
            return Err(format!("Server '{}' is disabled", server_name));
        }

        // Clone the client handle under the manager lock, then release that
        // lock before waiting for a potentially slow remote tool call.
        let client = self
            .clients
            .get(server_name)
            .cloned()
            .ok_or_else(|| format!("Server '{}' not connected", server_name))?;

        Ok(client)
    }

    /// Clone handles for enabled connected servers.  Callers must release the
    /// manager mutex before awaiting any individual client lock.
    pub fn tool_client_handles(&self) -> Vec<(String, Arc<Mutex<McpClient>>)> {
        self.clients
            .iter()
            .filter_map(|(name, client)| {
                if self
                    .configs
                    .iter()
                    .any(|config| config.name == *name && config.enabled)
                {
                    Some((name.clone(), client.clone()))
                } else {
                    None
                }
            })
            .collect()
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

/// Build and fully initialize an MCP client for the given config **without holding
/// any manager lock**. All slow I/O (process spawn, TCP handshake, MCP initialize)
/// happens here so callers can insert the result with only a brief lock.
pub async fn build_connected_client(config: &McpServerConfig) -> Result<McpClient, KokoroError> {
    tracing::info!(
        target: "mcp",
        "Connecting to '{}' (transport: {})...",
        config.name, config.transport_type
    );

    let transport: Arc<dyn super::transport::McpTransport> = match config.transport_type.as_str() {
        "streamable_http" | "streamable-http" => {
            let url = config.url.as_deref().ok_or_else(|| {
                KokoroError::Config(format!(
                    "Server '{}' has type '{}' but no 'url' configured",
                    config.name, config.transport_type
                ))
            })?;
            Arc::new(StreamableHttpTransport::new(url))
        }
        "sse" => {
            let url = config.url.as_deref().ok_or_else(|| {
                KokoroError::Config(format!(
                    "Server '{}' has type 'sse' but no 'url' configured",
                    config.name
                ))
            })?;
            let transport = SseTransport::new(url);
            if let Err(error) = transport.connect().await {
                let _ = transport.shutdown().await;
                return Err(KokoroError::ExternalService(error));
            }
            Arc::new(transport)
        }
        _ => {
            if config.command.is_empty() {
                if let Some(ref url) = config.url {
                    if url.trim_end_matches('/').ends_with("/sse") {
                        tracing::info!(target: "mcp", "Auto-detected SSE transport for '{}'", config.name);
                        let transport = SseTransport::new(url);
                        if let Err(error) = transport.connect().await {
                            let _ = transport.shutdown().await;
                            return Err(KokoroError::ExternalService(error));
                        }
                        Arc::new(transport)
                    } else {
                        tracing::info!(
                            target: "mcp",
                            "Auto-detected Streamable HTTP transport for '{}'",
                            config.name
                        );
                        Arc::new(StreamableHttpTransport::new(url))
                    }
                } else {
                    return Err(KokoroError::Config(format!(
                        "Server '{}' has no 'command' or 'url' configured",
                        config.name
                    )));
                }
            } else {
                Arc::new(
                    StdioTransport::spawn(&config.command, &config.args, Some(&config.env)).await?,
                )
            }
        }
    };

    let mut client = McpClient::new(transport);
    if let Err(error) = client.connect().await {
        let _ = client.shutdown().await;
        return Err(KokoroError::ExternalService(error));
    }
    Ok(client)
}
