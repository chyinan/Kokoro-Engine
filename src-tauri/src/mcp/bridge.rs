//! Publish negotiated MCP tools as actions with the existing strict approval policy.

use super::{
    client::{McpClient, McpConnectError, McpToolResult},
    manager::{ConnectionAttempt, ConnectionError, McpManager, McpServerConfig},
};
use crate::{
    actions::registry::{
        ActionContext, ActionError, ActionHandler, ActionParam, ActionPermissionLevel,
        ActionRegistry, ActionResult, ActionRiskTag,
    },
    error::KokoroError,
    lifecycle::TaskHandle,
};
use async_trait::async_trait;
use rmcp::model::JsonObject;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    sync::{Mutex, OnceCell, RwLock},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

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
}

const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct ConnectionWorker {
    stop: CancellationToken,
    task: Arc<Mutex<TaskHandle<Result<(), String>>>>,
    result: Arc<OnceCell<Result<(), String>>>,
}

impl ConnectionWorker {
    pub(super) fn stop(&self) {
        self.stop.cancel();
    }

    async fn join(&self) -> Result<(), String> {
        self.result
            .get_or_init(|| async {
                self.task
                    .lock()
                    .await
                    .join()
                    .await
                    .map_err(|error| format!("MCP connection task failed: {error}"))?
            })
            .await
            .clone()
    }
}

fn spawn_attempt(
    state: &mut McpManager,
    manager: &Manager,
    registry: &Registry,
    attempt: ConnectionAttempt,
) -> Result<(), KokoroError> {
    let name = attempt.name().to_owned();
    let stop = attempt.lifetime.clone();
    let manager = Arc::downgrade(manager);
    let registry = Arc::downgrade(registry);
    let task = state
        .scope
        .spawn_durable(&name, run_connection(manager, registry, attempt))
        .map_err(|error| KokoroError::ExternalService(error.to_string()))?;
    state.workers.insert(
        name,
        ConnectionWorker {
            stop,
            task: Arc::new(Mutex::new(task)),
            result: Arc::new(OnceCell::new()),
        },
    );
    Ok(())
}

async fn run_connection(
    manager: Weak<Mutex<McpManager>>,
    registry: Weak<RwLock<ActionRegistry>>,
    attempt: ConnectionAttempt,
) -> Result<(), String> {
    let name = attempt.name().to_string();
    let client = match attempt.connect().await {
        Ok(client) => Arc::new(client),
        Err(error) => {
            let cleanup_failed = matches!(
                &error,
                ConnectionError::Session(McpConnectError::Cleanup(_))
            );
            let message = match error {
                ConnectionError::Config(
                    KokoroError::Config(message) | KokoroError::ExternalService(message),
                ) => message,
                error => error.to_string(),
            };
            if let Some(manager) = manager.upgrade() {
                manager
                    .lock()
                    .await
                    .finish_connection_error(&attempt, message.clone());
            }
            if cleanup_failed {
                return Err(message);
            }
            return Ok(());
        }
    };

    let committed = match manager.upgrade() {
        Some(manager) => manager.lock().await.commit_client(&attempt, client.clone()),
        None => false,
    };
    let closed = client.lifetime();
    if committed {
        let watch = watch_catalog(manager.clone(), registry.clone(), client.clone());
        tokio::select! {
            biased;
            _ = attempt.lifetime.cancelled() => {}
            _ = closed.cancelled() => {}
            _ = watch => {}
        }
    } else if let Some(manager) = manager.upgrade() {
        manager.lock().await.finish_connection_error(
            &attempt,
            "MCP transport disconnected during initialization".into(),
        );
    }
    // Keep the connection alive while calls admitted before stop finish.
    let result = client.drain_and_close().await;
    if let (Some(manager), Some(registry)) = (manager.upgrade(), registry.upgrade()) {
        manager.lock().await.mark_disconnected(
            &name,
            &client,
            result.as_ref().err().map(String::as_str),
        );
        publish_catalog(&manager, &registry).await;
    }
    result
}

pub async fn start_enabled(manager: &Manager, registry: &Registry) {
    let control = manager.lock().await.control.clone();
    let _control = control.lock().await;
    let mut state = manager.lock().await;
    if !state.scope.is_running() || state.is_closed() {
        return;
    }
    let attempts = state.prepare_connect_all();
    for attempt in attempts {
        if let Err(error) = spawn_attempt(&mut state, manager, registry, attempt) {
            tracing::warn!(target: "mcp", %error, "MCP auto-start was unable to run");
        }
    }
}

async fn change_connection(
    manager: &Manager,
    registry: &Registry,
    name: &str,
    connect: bool,
    change: impl FnOnce(&mut McpManager) -> Result<Option<Arc<McpClient>>, KokoroError>,
) -> Result<(), KokoroError> {
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let control = manager.lock().await.control.clone();
    let _control = tokio::time::timeout_at(deadline, control.lock())
        .await
        .map_err(|_| {
            KokoroError::ExternalService("Another MCP control operation is still running".into())
        })?;
    let (previous, worker) = {
        let mut state = manager.lock().await;
        check_controls(&state)?;
        let previous = change(&mut state)?;
        let worker = state.workers.get(name).cloned();
        if let Some(worker) = &worker {
            worker.stop();
        }
        (previous, worker)
    };
    publish_catalog(manager, registry).await;

    let completion = async {
        if let Some(worker) = worker {
            worker.join().await
        } else if let Some(previous) = previous {
            previous.drain_and_close().await
        } else {
            Ok(())
        }
    };
    tokio::time::timeout_at(deadline, completion)
        .await
        .map_err(|_| KokoroError::ExternalService(format!(
            "MCP server '{name}' is still stopping. Confirmed shutdown is pending and any outstanding external tool outcome is uncertain."
        )))?
        .map_err(KokoroError::ExternalService)?;
    let mut state = manager.lock().await;
    state.workers.remove(name);
    if connect {
        check_controls(&state)?;
        let attempt = state.prepare_connection(name)?;
        spawn_attempt(&mut state, manager, registry, attempt)?;
    }
    Ok(())
}

fn check_controls(state: &McpManager) -> Result<(), KokoroError> {
    if state.scope.is_running() && !state.is_closed() {
        Ok(())
    } else {
        Err(KokoroError::ExternalService(
            "MCP service is shutting down".into(),
        ))
    }
}

pub async fn add_server(
    manager: &Manager,
    registry: &Registry,
    config: McpServerConfig,
) -> Result<(), KokoroError> {
    let name = config.name.clone();
    change_connection(manager, registry, &name, config.enabled, |state| {
        state.upsert_server_config(config)
    })
    .await
}

pub async fn remove_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
) -> Result<(), KokoroError> {
    change_connection(manager, registry, name, false, |state| {
        state.remove_server_config(name)
    })
    .await
}

pub async fn reconnect_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
) -> Result<(), KokoroError> {
    change_connection(manager, registry, name, true, |state| {
        state.ensure_enabled(name)?;
        Ok(state.detach_client(name))
    })
    .await
}

pub async fn toggle_server(
    manager: &Manager,
    registry: &Registry,
    name: &str,
    enabled: bool,
) -> Result<(), KokoroError> {
    change_connection(manager, registry, name, enabled, |state| {
        state.set_server_enabled(name, enabled)
    })
    .await
}

pub async fn refresh_all(manager: &Manager, registry: &Registry) -> Result<(), KokoroError> {
    let clients: Vec<_> = {
        let state = manager.lock().await;
        check_controls(&state)?;
        state
            .connected_clients()
            .map(|(_, client)| client.clone())
            .collect()
    };
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

pub async fn shutdown(manager: &Manager, registry: &Registry) -> Result<(), KokoroError> {
    let (clients, workers) = {
        let mut state = manager.lock().await;
        let clients = state.begin_shutdown();
        let workers: Vec<_> = state.workers.values().cloned().collect();
        for worker in &workers {
            worker.stop();
        }
        (clients, workers)
    };
    publish_catalog(manager, registry).await;
    // A worker joins its SDK and transport. Also close any clients attached
    // directly to the manager before the worker was installed.
    let (connections, clients) = tokio::join!(
        futures::future::join_all(workers.iter().map(ConnectionWorker::join)),
        futures::future::join_all(clients.iter().map(|client| client.drain_and_close())),
    );
    let failures: Vec<_> = connections
        .into_iter()
        .chain(clients)
        .filter_map(Result::err)
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(KokoroError::ExternalService(failures.join(", ")))
    }
}
