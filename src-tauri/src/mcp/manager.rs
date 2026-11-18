//! Saved MCP configurations and the ownership of each named connection.

use super::{
    client::{McpClient, McpConnectError, McpToolInfo},
    transport::{self, LegacySse, TransportMode},
};
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

// ── Config Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct McpServerConfig {
    /// Display name for this server.
    pub name: String,
    /// Transport type: "stdio" (default), "streamable_http", or "sse".
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

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct McpServerStatus {
    pub name: String,
    pub enabled: bool,
    pub connected: bool,
    pub tool_count: u32,
    pub server_version: Option<String>,
    /// "connected" | "connecting" | "disconnected"
    pub status: String,
    /// Error message if connection failed.
    pub error: Option<String>,
}

pub(super) struct ConnectionAttempt {
    config: McpServerConfig,
    generation: u64,
    lifetime: CancellationToken,
}

pub(super) struct CatalogSnapshot {
    pub(super) server: String,
    pub(super) client: Weak<McpClient>,
    pub(super) revision: u64,
    pub(super) tools: Vec<McpToolInfo>,
}

impl ConnectionAttempt {
    pub(super) fn name(&self) -> &str {
        &self.config.name
    }

    pub(super) async fn connect(&self) -> Result<McpClient, KokoroError> {
        build_connected_client(&self.config, self.lifetime.clone()).await
    }
}

pub struct McpManager {
    configs: Vec<McpServerConfig>,
    clients: HashMap<String, Arc<McpClient>>,
    published: HashMap<String, (Weak<McpClient>, u64)>,
    config_path: String,
    pending: HashMap<String, (u64, CancellationToken)>,
    next_generation: u64,
    connection_errors: HashMap<String, String>,
    closed: bool,
}

impl McpManager {
    pub fn new(config_path: &str) -> Self {
        Self {
            configs: Vec::new(),
            clients: HashMap::new(),
            published: HashMap::new(),
            config_path: config_path.to_string(),
            pending: HashMap::new(),
            next_generation: 0,
            connection_errors: HashMap::new(),
            closed: false,
        }
    }

    /// Config files have the same on-disk shape as before the SDK migration.
    pub fn load_configs(&mut self) {
        let path = Path::new(&self.config_path);
        if !path.exists() {
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<McpServerConfig>>(&content) {
                Ok(configs) => self.configs = configs,
                Err(error) => {
                    tracing::error!(target: "mcp", "Invalid MCP config at line {}", error.line())
                }
            },
            Err(error) => tracing::error!(target: "mcp", "Could not read MCP config: {error}"),
        }
    }

    fn persist_configs(&mut self, configs: Vec<McpServerConfig>) -> Result<(), KokoroError> {
        let content = serde_json::to_string_pretty(&configs)
            .map_err(|e| KokoroError::Config(format!("Serialize error: {e}")))?;
        std::fs::write(&self.config_path, content)
            .map_err(|e| KokoroError::Io(format!("Write error: {e}")))?;
        self.configs = configs;
        Ok(())
    }

    fn mark_connecting(&mut self, config: McpServerConfig) -> ConnectionAttempt {
        self.invalidate_pending(&config.name);
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        let lifetime = CancellationToken::new();
        self.pending
            .insert(config.name.clone(), (generation, lifetime.clone()));
        ConnectionAttempt {
            config,
            generation,
            lifetime,
        }
    }

    pub(super) fn prepare_connect_all(&mut self) -> Vec<ConnectionAttempt> {
        if self.closed {
            return Vec::new();
        }
        let configs: Vec<_> = self
            .configs
            .iter()
            .filter(|c| {
                c.enabled
                    && !self.pending.contains_key(&c.name)
                    && !self.clients.contains_key(&c.name)
            })
            .cloned()
            .collect();
        configs
            .into_iter()
            .map(|config| self.mark_connecting(config))
            .collect()
    }

    pub(super) fn prepare_connection(
        &mut self,
        name: &str,
    ) -> Result<ConnectionAttempt, KokoroError> {
        let config = self
            .configs
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| KokoroError::NotFound(format!("Server '{name}' not found")))?;
        if !config.enabled || self.closed {
            return Err(KokoroError::Config(format!("Server '{name}' is disabled")));
        }
        Ok(self.mark_connecting(config.clone()))
    }

    pub(super) fn attempt_is_current(&self, attempt: &ConnectionAttempt) -> bool {
        !self.closed
            && self
                .pending
                .get(attempt.name())
                .map(|(generation, _)| *generation)
                == Some(attempt.generation)
            && self
                .configs
                .iter()
                .any(|c| c.enabled && c.name == attempt.name())
    }

    pub(super) fn commit_client(
        &mut self,
        attempt: &ConnectionAttempt,
        client: Arc<McpClient>,
    ) -> bool {
        if !self.attempt_is_current(attempt) || !client.is_connected() {
            return false;
        }
        self.pending.remove(attempt.name());
        if let Some(previous) = self.clients.insert(attempt.name().to_string(), client) {
            previous.cancel();
        }
        true
    }

    pub(super) fn finish_connection_error(&mut self, attempt: &ConnectionAttempt, error: String) {
        if self.attempt_is_current(attempt) {
            self.pending.remove(attempt.name());
            self.connection_errors
                .insert(attempt.name().to_string(), error);
        }
    }

    fn invalidate_pending(&mut self, name: &str) {
        if let Some((_, lifetime)) = self.pending.remove(name) {
            lifetime.cancel();
        }
        self.connection_errors.remove(name);
    }

    /// Invalidate the old connection synchronously; the caller joins its resource owner outside the lock.
    pub(super) fn detach_client(&mut self, name: &str) -> Option<Arc<McpClient>> {
        self.invalidate_pending(name);
        self.detach_live_client(name)
    }

    pub(super) fn detach_live_client(&mut self, name: &str) -> Option<Arc<McpClient>> {
        let client = self.clients.remove(name)?;
        client.cancel();
        Some(client)
    }

    pub(super) fn upsert_server_config(
        &mut self,
        config: McpServerConfig,
    ) -> Result<Option<Arc<McpClient>>, KokoroError> {
        let mut configs = self.configs.clone();
        let name = config.name.clone();
        configs.retain(|c| c.name != name);
        configs.push(config);
        self.persist_configs(configs)?;
        Ok(self.detach_client(&name))
    }

    pub(super) fn remove_server_config(
        &mut self,
        name: &str,
    ) -> Result<Option<Arc<McpClient>>, KokoroError> {
        let mut configs = self.configs.clone();
        configs.retain(|c| c.name != name);
        self.persist_configs(configs)?;
        Ok(self.detach_client(name))
    }

    pub(super) fn set_server_enabled(
        &mut self,
        name: &str,
        enabled: bool,
    ) -> Result<Option<Arc<McpClient>>, KokoroError> {
        let mut configs = self.configs.clone();
        let config = configs
            .iter_mut()
            .find(|c| c.name == name)
            .ok_or_else(|| KokoroError::NotFound(format!("Server '{name}' not found")))?;
        config.enabled = enabled;
        self.persist_configs(configs)?;
        Ok(self.detach_client(name))
    }

    pub fn list_status(&self) -> Vec<McpServerStatus> {
        self.configs
            .iter()
            .map(|config| {
                let client = self
                    .clients
                    .get(&config.name)
                    .filter(|client| client.is_connected());
                let status = if self.pending.contains_key(&config.name) {
                    "connecting"
                } else if client.is_some() {
                    "connected"
                } else {
                    "disconnected"
                };
                McpServerStatus {
                    name: config.name.clone(),
                    enabled: config.enabled,
                    connected: client.is_some(),
                    tool_count: client.map_or(0, |client| client.tool_count()),
                    server_version: client.and_then(|client| client.server_version()),
                    status: status.to_string(),
                    error: self.connection_errors.get(&config.name).cloned(),
                }
            })
            .collect()
    }

    pub(super) fn connected_clients(&self) -> impl Iterator<Item = (&str, &Arc<McpClient>)> {
        self.configs
            .iter()
            .filter(|config| config.enabled)
            .filter_map(|config| {
                let client = self.clients.get(&config.name)?;
                client
                    .is_connected()
                    .then_some((config.name.as_str(), client))
            })
    }

    pub(super) fn catalog_for_publication(&mut self) -> Option<Vec<CatalogSnapshot>> {
        let snapshots: Vec<_> = self
            .connected_clients()
            .filter_map(|(server, client)| {
                let (revision, tools) = client.tools()?;
                (!tools.is_empty()).then(|| CatalogSnapshot {
                    server: server.to_string(),
                    client: Arc::downgrade(client),
                    revision,
                    tools,
                })
            })
            .collect();
        // Rebuilding the same entries would advance the shared ActionRegistry
        // generation and invalidate approvals with no change to their tools.
        if self.published.len() == snapshots.len()
            && snapshots.iter().all(|snapshot| {
                self.published
                    .get(&snapshot.server)
                    .is_some_and(|(client, revision)| {
                        client.ptr_eq(&snapshot.client) && *revision == snapshot.revision
                    })
            })
        {
            return None;
        }
        self.published = snapshots
            .iter()
            .map(|s| (s.server.clone(), (s.client.clone(), s.revision)))
            .collect();
        Some(snapshots)
    }

    pub(super) fn mark_disconnected(&mut self, name: &str, client: &Arc<McpClient>) {
        if self
            .clients
            .get(name)
            .is_some_and(|current| Arc::ptr_eq(client, current))
        {
            self.clients.remove(name);
            self.connection_errors
                .insert(name.to_string(), "MCP transport disconnected".into());
        }
    }

    pub(super) fn begin_shutdown(&mut self) -> Vec<Arc<McpClient>> {
        self.closed = true;
        for (_, (_, lifetime)) in self.pending.drain() {
            lifetime.cancel();
        }
        self.clients
            .drain()
            .map(|(_, client)| {
                client.cancel();
                client
            })
            .collect()
    }
}

impl Drop for McpManager {
    fn drop(&mut self) {
        self.begin_shutdown();
    }
}

/// Connection and initial tool discovery happen without holding any manager lock.
pub(super) async fn build_connected_client(
    config: &McpServerConfig,
    lifetime: CancellationToken,
) -> Result<McpClient, KokoroError> {
    let mode = transport::transport_mode(config)?;
    let connect = async {
        let result = match mode {
            TransportMode::Stdio => {
                McpClient::start(transport::stdio(config)?, false, lifetime.clone()).await
            }
            TransportMode::StreamableHttp => {
                let transport = transport::streamable_http(transport::server_url(config)?)?;
                McpClient::start(transport, false, lifetime.clone()).await
            }
            TransportMode::LegacySse => {
                let transport = LegacySse::connect(transport::server_url(config)?)
                    .await
                    .map_err(|error| KokoroError::ExternalService(error.to_string()))?;
                McpClient::start(transport, true, lifetime.clone()).await
            }
        };
        result.map_err(|error| KokoroError::ExternalService(error.to_string()))
    };
    tokio::select! {
        biased;
        _ = lifetime.cancelled() => Err(KokoroError::ExternalService(McpConnectError::Cancelled.to_string())),
        result = tokio::time::timeout(Duration::from_secs(45), connect) => {
            result.unwrap_or_else(|_| Err(KokoroError::ExternalService("MCP connection timed out".into())))
        }
    }
}
