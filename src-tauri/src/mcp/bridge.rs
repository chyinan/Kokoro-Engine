//! Publish negotiated MCP tools as actions with the existing strict approval policy.

use super::{
    client::{McpClient, McpToolResult},
    manager::{ConnectionAttempt, McpManager, McpServerConfig},
};
use crate::{
    actions::registry::{
        ActionContext, ActionError, ActionHandler, ActionParam, ActionPermissionLevel,
        ActionRegistry, ActionResult, ActionRiskTag,
    },
    error::KokoroError,
};
use async_trait::async_trait;
use rmcp::model::JsonObject;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::sync::{Mutex, RwLock};

type Manager = Arc<Mutex<McpManager>>;
type Registry = Arc<RwLock<ActionRegistry>>;

struct McpToolHandler {
    name: String,
    description: String,
    input_schema: Arc<JsonObject>,
    // An approved action stays bound to the definition and peer it described.
    // An old handler must never send a call to a new connection of the same name.
    client: Weak<McpClient>,
    catalog_revision: u64,
}

#[async_trait]
impl ActionHandler for McpToolHandler {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Vec<ActionParam> {
        let Some(properties) = self
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
        else {
            return Vec::new();
        };
        let required = self.input_schema.get("required").and_then(Value::as_array);
        properties
            .iter()
            .map(|(name, property)| ActionParam {
                name: name.clone(),
                description: property
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                required: required.is_some_and(|entries| {
                    entries.iter().any(|entry| entry.as_str() == Some(name))
                }),
            })
            .collect()
    }

    fn needs_feedback(&self) -> bool {
        true
    }

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
        let client = self
            .client
            .upgrade()
            .ok_or_else(|| ActionError("MCP tool connection is no longer available".into()))?;
        let arguments = args
            .into_iter()
            .map(|(key, value)| {
                let parsed = serde_json::from_str(&value).unwrap_or(Value::String(value));
                (key, parsed)
            })
            .collect();
        let result = client
            .call_tool(self.catalog_revision, &self.name, arguments)
            .await
            .map_err(|error| ActionError(error.to_string()))?;
        Ok(action_result(result))
    }
}

pub(super) fn action_result(result: McpToolResult) -> ActionResult {
    let mut messages = Vec::new();
    for block in &result.content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") if block.get("text").and_then(Value::as_str).is_some() => {
                messages.push(block["text"].as_str().unwrap_or_default().to_string());
            }
            Some("image") => messages.push("[MCP image content attached]".into()),
            Some("audio") => messages.push("[MCP audio content attached]".into()),
            Some("resource_link") => messages.push(format!(
                "[MCP resource link: {}]",
                block
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown URI")
            )),
            Some("resource") => messages.push(format!(
                "[MCP embedded resource: {}]",
                block
                    .pointer("/resource/uri")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown URI")
            )),
            _ => messages.push("[Unrecognized MCP content block attached as JSON]".into()),
        }
    }
    let mut data = serde_json::Map::new();
    data.insert("content".into(), Value::Array(result.content));
    if let Some(structured) = result.structured_content {
        messages.push(format!("MCP structured content: {structured}"));
        data.insert("structuredContent".into(), structured);
    }
    data.insert("isError".into(), Value::Bool(result.is_error));
    let message = messages.join("\n");
    let message = if message.is_empty() {
        if result.is_error {
            "MCP tool returned an error without details".into()
        } else {
            "MCP tool returned no content".into()
        }
    } else {
        message
    };
    ActionResult {
        success: !result.is_error,
        message,
        data: Some(Value::Object(data)),
    }
}

pub(super) async fn publish_catalog(manager: &Manager, registry: &Registry) {
    // All callers acquire locks in this order. With both guards, a removal or
    // replacement cannot race the publication of its previous generation.
    let mut actions = registry.write().await;
    let mut manager = manager.lock().await;
    let Some(snapshots) = manager.catalog_for_publication() else {
        return;
    };
    actions.clear_mcp_tools();
    for snapshot in snapshots {
        for tool in snapshot.tools {
            actions.register_mcp(
                &snapshot.server,
                McpToolHandler {
                    name: tool.name,
                    description: tool.description.unwrap_or_default(),
                    input_schema: tool.input_schema,
                    client: snapshot.client.clone(),
                    catalog_revision: snapshot.revision,
                },
            );
        }
    }
}

async fn watch_catalog(
    manager: Weak<Mutex<McpManager>>,
    registry: Weak<RwLock<ActionRegistry>>,
    name: String,
    client: Arc<McpClient>,
) {
    let lifetime = client.lifetime();
    let mut changes = client.changes();
    loop {
        // Also run once at startup to pick up a change that arrived before subscription.
        changes.borrow_and_update();
        if let (Some(manager), Some(registry)) = (manager.upgrade(), registry.upgrade()) {
            publish_catalog(&manager, &registry).await;
        } else {
            return;
        }
        if !lifetime.is_cancelled() {
            if client.refresh_tools().await.is_err() {
                tracing::debug!(target: "mcp", "Could not refresh MCP catalog");
            }
            if let (Some(manager), Some(registry)) = (manager.upgrade(), registry.upgrade()) {
                publish_catalog(&manager, &registry).await;
            }
        }
        tokio::select! {
            biased;
            _ = lifetime.cancelled() => break,
            changed = changes.changed() => if changed.is_err() { break; },
        }
    }
    if let (Some(manager), Some(registry)) = (manager.upgrade(), registry.upgrade()) {
        manager.lock().await.mark_disconnected(&name, &client);
        publish_catalog(&manager, &registry).await;
    }
}

fn spawn_attempt(
    manager: &Manager,
    registry: &Registry,
    attempt: ConnectionAttempt,
    previous: Option<Arc<McpClient>>,
) {
    let manager = manager.clone();
    let registry = registry.clone();
    tokio::spawn(async move {
        if let Some(previous) = previous {
            previous.shutdown().await;
        }
        if !manager.lock().await.attempt_is_current(&attempt) {
            return;
        }
        let client = match attempt.connect().await {
            Ok(client) => Arc::new(client),
            Err(error) => {
                let message = match error {
                    KokoroError::Config(message) | KokoroError::ExternalService(message) => message,
                    _ => error.to_string(),
                };
                manager
                    .lock()
                    .await
                    .finish_connection_error(&attempt, message);
                return;
            }
        };
        let committed = manager.lock().await.commit_client(&attempt, client.clone());
        if committed {
            let manager = Arc::downgrade(&manager);
            let registry = Arc::downgrade(&registry);
            tokio::spawn(watch_catalog(
                manager,
                registry,
                attempt.name().to_string(),
                client,
            ));
        } else {
            // A failed or superseded startup must not resurrect the server.
            manager.lock().await.finish_connection_error(
                &attempt,
                "MCP transport disconnected during initialization".into(),
            );
            client.shutdown().await;
        }
    });
}

pub async fn start_enabled(manager: &Manager, registry: &Registry) {
    let attempts = manager.lock().await.prepare_connect_all();
    for attempt in attempts {
        spawn_attempt(manager, registry, attempt, None);
    }
}

async fn apply_connection_change(
    manager: &Manager,
    registry: &Registry,
    attempt: Option<ConnectionAttempt>,
    previous: Option<Arc<McpClient>>,
) {
    publish_catalog(manager, registry).await;
    if let Some(attempt) = attempt {
        spawn_attempt(manager, registry, attempt, previous);
    } else if let Some(previous) = previous {
        previous.shutdown().await;
    }
}

pub async fn add_server(
    manager: &Manager,
    registry: &Registry,
    config: McpServerConfig,
) -> Result<(), KokoroError> {
    let (attempt, previous) = {
        let mut manager = manager.lock().await;
        let enabled = config.enabled;
        let name = config.name.clone();
        let previous = manager.upsert_server_config(config)?;
        let attempt = enabled
            .then(|| manager.prepare_connection(&name))
            .transpose()?;
        (attempt, previous)
    };
    apply_connection_change(manager, registry, attempt, previous).await;
    Ok(())
}

pub async fn remove_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
) -> Result<(), KokoroError> {
    let previous = manager.lock().await.remove_server_config(name)?;
    apply_connection_change(manager, registry, None, previous).await;
    Ok(())
}

pub async fn reconnect_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
) -> Result<(), KokoroError> {
    let (attempt, previous) = {
        let mut manager = manager.lock().await;
        let attempt = manager.prepare_connection(name)?;
        let previous = manager.detach_live_client(name);
        (attempt, previous)
    };
    apply_connection_change(manager, registry, Some(attempt), previous).await;
    Ok(())
}

pub async fn toggle_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
    enabled: bool,
) -> Result<(), KokoroError> {
    let (attempt, previous) = {
        let mut manager = manager.lock().await;
        let previous = manager.set_server_enabled(name, enabled)?;
        let attempt = enabled
            .then(|| manager.prepare_connection(name))
            .transpose()?;
        (attempt, previous)
    };
    apply_connection_change(manager, registry, attempt, previous).await;
    Ok(())
}

pub async fn refresh_all(manager: &Manager, registry: &Registry) -> Result<(), KokoroError> {
    let clients: Vec<_> = manager
        .lock()
        .await
        .connected_clients()
        .map(|(_, client)| client.clone())
        .collect();
    for client in &clients {
        client.invalidate_tools();
    }
    publish_catalog(manager, registry).await;
    let results =
        futures::future::join_all(clients.iter().map(|client| client.refresh_tools())).await;
    publish_catalog(manager, registry).await;
    results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map(|_| ())
        .map_err(|error| KokoroError::ExternalService(error.to_string()))
}

pub async fn shutdown(manager: &Manager) {
    let clients = manager.lock().await.begin_shutdown();
    futures::future::join_all(clients.iter().map(|client| client.shutdown())).await;
}
