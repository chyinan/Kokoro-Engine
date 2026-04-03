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

    {
        let mut mgr = mgr_arc.lock().await;
        mgr.add_server(config.clone(), false).await?;
        if config.enabled {
            mgr.mark_connecting(&config.name);
        }
    }

    // Spawn background task to connect
    if config.enabled {
        let cfg = config.clone();
        tauri::async_runtime::spawn(async move {
            println!("[MCP] Background connecting to '{}'...", cfg.name);
            let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
            let connect_result = {
                let mut mgr = mgr_arc.lock().await;
                mgr.clear_connecting(&cfg.name);
                match build_result {
                    Ok(client) => {
                        mgr.insert_client(cfg.name.clone(), client);
                        Ok(())
                    }
                    Err(e) => {
                        mgr.set_connection_error(&cfg.name, format_connection_error(&e));
                        Err(e)
                    }
                }
            };
            match connect_result {
                Ok(()) => {
                    println!("[MCP] Connected '{}', refreshing tools...", cfg.name);
                    crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
                }
                Err(e) => eprintln!("[MCP] Connection failed for '{}': {}", cfg.name, e),
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
) -> Result<(), KokoroError> {
    let mut mgr = manager.lock().await;
    mgr.remove_server(&name).await?;
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

    let cfg = {
        let mut mgr = mgr_arc.lock().await;
        let cfg = mgr
            .get_config(&name)
            .ok_or_else(|| KokoroError::NotFound(format!("Server '{}' not found", name)))?;
        // Disconnect existing (if any) before retrying
        let _ = mgr.disconnect_server(&name).await;
        mgr.mark_connecting(&name);
        cfg
    };

    tauri::async_runtime::spawn(async move {
        println!("[MCP] Retrying connection to '{}'...", cfg.name);
        let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
        let connect_result = {
            let mut mgr = mgr_arc.lock().await;
            mgr.clear_connecting(&cfg.name);
            match build_result {
                Ok(client) => {
                    mgr.insert_client(cfg.name.clone(), client);
                    Ok(())
                }
                Err(e) => {
                    mgr.set_connection_error(&cfg.name, format_connection_error(&e));
                    Err(e)
                }
            }
        };
        match connect_result {
            Ok(()) => {
                println!("[MCP] Reconnected '{}', refreshing tools...", cfg.name);
                crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
            }
            Err(e) => eprintln!("[MCP] Reconnection failed for '{}': {}", cfg.name, e),
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

    let cfg = {
        let mut mgr = mgr_arc.lock().await;
        mgr.toggle_server(&name, enabled).await?;

        if enabled {
            let cfg = mgr
                .get_config(&name)
                .ok_or_else(|| KokoroError::NotFound(format!("Server '{}' not found", name)))?;
            mgr.mark_connecting(&name);
            Some(cfg)
        } else {
            // Disabled — refresh action registry to remove tools
            None
        }
    };

    if let Some(cfg) = cfg {
        // Enable: spawn background connection
        tauri::async_runtime::spawn(async move {
            println!("[MCP] Enabling and connecting '{}'...", cfg.name);
            let build_result = crate::mcp::manager::build_connected_client(&cfg).await;
            let connect_result = {
                let mut mgr = mgr_arc.lock().await;
                mgr.clear_connecting(&cfg.name);
                match build_result {
                    Ok(client) => {
                        mgr.insert_client(cfg.name.clone(), client);
                        Ok(())
                    }
                    Err(e) => {
                        mgr.set_connection_error(&cfg.name, format_connection_error(&e));
                        Err(e)
                    }
                }
            };
            match connect_result {
                Ok(()) => {
                    println!("[MCP] Connected '{}', refreshing tools...", cfg.name);
                    crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
                }
                Err(e) => eprintln!("[MCP] Connection failed for '{}': {}", cfg.name, e),
            }
        });
    } else {
        // Disable: refresh action registry immediately
        crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
    }

    Ok(())
}
