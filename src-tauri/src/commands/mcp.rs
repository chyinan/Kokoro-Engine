// pattern: Imperative Shell

use crate::actions::ActionRegistry;
use crate::error::KokoroError;
use crate::mcp::manager::{McpManager, McpServerConfig, McpServerStatus};
use std::sync::Arc;
use tauri::State;
use tokio::sync::{Mutex, RwLock};

fn format_connection_error(error: &KokoroError) -> String {
    match error {
        KokoroError::Config(message)
        | KokoroError::Database(message)
        | KokoroError::Llm(message)
        | KokoroError::Tts(message)
        | KokoroError::Stt(message)
        | KokoroError::Io(message)
        | KokoroError::ExternalService(message)
        | KokoroError::Mod(message)
        | KokoroError::NotFound(message)
        | KokoroError::Unauthorized(message)
        | KokoroError::Internal(message)
        | KokoroError::Chat(message)
        | KokoroError::Validation(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_connection_error;
    use crate::error::KokoroError;

    #[test]
    fn strips_internal_prefix_for_connection_errors() {
        let raw = KokoroError::Internal("MCP server process exited".to_string());
        assert_eq!(format_connection_error(&raw), "MCP server process exited");
    }
}

/// List all configured MCP servers and their connection status.
#[tauri::command]
pub async fn list_mcp_servers(
    manager: State<'_, Arc<Mutex<McpManager>>>,
) -> Result<Vec<McpServerStatus>, KokoroError> {
    let mgr = manager.lock().await;
    Ok(mgr.list_status().await)
}

/// Add a new MCP server — saves config immediately, then connects in background.
/// Returns Ok(()) as soon as the config is saved so the UI isn't blocked.
#[tauri::command]
pub async fn add_mcp_server(
    config: McpServerConfig,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    let mgr_arc = manager.inner().clone();
    let reg_arc = registry.inner().clone();

    let (old_client, generation) = {
        let mut mgr = mgr_arc.lock().await;
        let old_client = mgr.upsert_server_config(config.clone())?;
        let generation = config.enabled.then(|| mgr.mark_connecting(&config.name));
        (old_client, generation)
    };

    // A replacement may have an active transport.  Shut it down after the
    // manager lock is released so a slow MCP process cannot block commands.
    if let Some(client) = old_client {
        client
            .lock()
            .await
            .shutdown()
            .await
            .map_err(KokoroError::ExternalService)?;
    }

    if let Some(generation) = generation {
        let cfg = config.clone();
        let mgr_arc = mgr_arc.clone();
        let reg_arc = reg_arc.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!(target: "mcp", "Background connecting to '{}'...", cfg.name);
            let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
            let (connect_result, stale_client) = {
                let mut mgr = mgr_arc.lock().await;
                match build_result {
                    Ok(client) => match mgr.commit_client(cfg.name.clone(), generation, client) {
                        Ok(()) => (Ok(()), None),
                        Err(client) => (Err("connection result was superseded".to_string()), Some(client)),
                    },
                    Err(e) => {
                        let message = format_connection_error(&e);
                        let accepted = mgr.finish_connection_error(&cfg.name, generation, message.clone());
                        if accepted { (Err(message), None) }
                        else { (Err("connection result was superseded".to_string()), None) }
                    }
                }
            };
            if let Some(client) = stale_client {
                let _ = client.shutdown().await;
            }
            match connect_result {
                Ok(()) => {
                    tracing::info!(target: "mcp", "Connected '{}', refreshing tools...", cfg.name);
                    crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
                }
                Err(e) => tracing::error!(target: "mcp", "Connection failed for '{}': {}", cfg.name, e),
            }
        });
    }

    Ok(())
}

/// Remove an MCP server.
#[tauri::command]
pub async fn remove_mcp_server(
    name: String,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    let client = {
        let mut mgr = manager.lock().await;
        mgr.detach_server_for_removal(&name)?
    };
    if let Some(client) = client {
        client
            .lock()
            .await
            .shutdown()
            .await
            .map_err(KokoroError::ExternalService)?;
    }
    // Remove stale MCP actions immediately so the model and direct action
    // callers cannot invoke tools from the deleted server.
    crate::mcp::bridge::register_mcp_tools(&manager.inner().clone(), registry.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn refresh_mcp_tools(
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    crate::mcp::bridge::register_mcp_tools(&manager.inner().clone(), registry.inner()).await;
    Ok(())
}

/// Retry connecting a disconnected MCP server.
#[tauri::command]
pub async fn reconnect_mcp_server(
    name: String,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    let mgr_arc = manager.inner().clone();
    let reg_arc = registry.inner().clone();

        let (cfg, generation, old_client) = {
            let mut mgr = mgr_arc.lock().await;
        let cfg = mgr
            .get_config(&name)
            .ok_or_else(|| KokoroError::NotFound(format!("Server '{}' not found", name)))?;
        // Disconnect existing (if any) before retrying
            let old_client = mgr.take_client(&name);
            let generation = mgr.mark_connecting(&name);
            (cfg, generation, old_client)
        };

    if let Some(client) = old_client {
        client
            .lock()
            .await
            .shutdown()
            .await
            .map_err(KokoroError::ExternalService)?;
    }

    tauri::async_runtime::spawn(async move {
        tracing::info!(target: "mcp", "Retrying connection to '{}'...", cfg.name);
        let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
        let (connect_result, stale_client) = {
            let mut mgr = mgr_arc.lock().await;
            match build_result {
                Ok(client) => match mgr.commit_client(cfg.name.clone(), generation, client) {
                    Ok(()) => (Ok(()), None),
                    Err(client) => (Err("connection result was superseded".to_string()), Some(client)),
                },
                Err(e) => {
                    let message = format_connection_error(&e);
                    if mgr.finish_connection_error(&cfg.name, generation, message.clone()) {
                        (Err(message), None)
                    } else {
                        (Err("connection result was superseded".to_string()), None)
                    }
                }
            }
        };
        if let Some(client) = stale_client {
            let _ = client.shutdown().await;
        }
        match connect_result {
            Ok(()) => {
                tracing::info!(target: "mcp", "Reconnected '{}', refreshing tools...", cfg.name);
                crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
            }
            Err(e) => {
                tracing::error!(target: "mcp", "Reconnection failed for '{}': {}", cfg.name, e)
            }
        }
    });

    Ok(())
}

/// Toggle a server's enabled state — disable disconnects, enable reconnects in background.
#[tauri::command]
pub async fn toggle_mcp_server(
    name: String,
    enabled: bool,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    let mgr_arc = manager.inner().clone();
    let reg_arc = registry.inner().clone();

    let (cfg, client_to_shutdown) = {
        let mut mgr = mgr_arc.lock().await;
        let client_to_shutdown = mgr.set_server_enabled(&name, enabled)?;

        let next_config = if enabled {
            let cfg = mgr
                .get_config(&name)
                .ok_or_else(|| KokoroError::NotFound(format!("Server '{}' not found", name)))?;
            let generation = mgr.mark_connecting(&name);
            Some((cfg, generation))
        } else {
            // Disabled — refresh action registry to remove tools
            None
        };
        (next_config, client_to_shutdown)
    };

    if let Some(client) = client_to_shutdown {
        client
            .lock()
            .await
            .shutdown()
            .await
            .map_err(KokoroError::ExternalService)?;
    }

    if let Some((cfg, generation)) = cfg {
        // Enable: spawn background connection
        tauri::async_runtime::spawn(async move {
            tracing::info!(target: "mcp", "Enabling and connecting '{}'...", cfg.name);
            let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
            let (connect_result, stale_client) = {
                let mut mgr = mgr_arc.lock().await;
                match build_result {
                    Ok(client) => match mgr.commit_client(cfg.name.clone(), generation, client) {
                        Ok(()) => (Ok(()), None),
                        Err(client) => (Err("connection result was superseded".to_string()), Some(client)),
                    },
                    Err(e) => {
                        let message = format_connection_error(&e);
                        if mgr.finish_connection_error(&cfg.name, generation, message.clone()) {
                            (Err(message), None)
                        } else {
                            (Err("connection result was superseded".to_string()), None)
                        }
                    }
                }
            };
            if let Some(client) = stale_client {
                let _ = client.shutdown().await;
            }
            match connect_result {
                Ok(()) => {
                    tracing::info!(target: "mcp", "Connected '{}', refreshing tools...", cfg.name);
                    crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
                }
                Err(e) => {
                    tracing::error!(target: "mcp", "Connection failed for '{}': {}", cfg.name, e)
                }
            }
        });
    } else {
        // Disable: refresh action registry immediately
        crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
    }

    Ok(())
}
