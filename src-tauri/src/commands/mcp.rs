use crate::actions::ActionRegistry;
use crate::mcp::manager::{McpManager, McpServerConfig, McpServerStatus};
use std::sync::Arc;
use tauri::State;
use tokio::sync::{Mutex, RwLock};

/// List all configured MCP servers and their connection status.
#[tauri::command]
pub async fn list_mcp_servers(
    manager: State<'_, Arc<Mutex<McpManager>>>,
) -> Result<Vec<McpServerStatus>, String> {
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
) -> Result<(), String> {
    let mgr_arc = manager.inner().clone();
    let reg_arc = registry.inner().clone();

    {
        let mut mgr = mgr_arc.lock().await;
        // Add to config but don't connect yet
        mgr.add_server(config.clone(), false).await?;

        // Mark as "connecting" so the UI can poll and show spinner
        if config.enabled {
            mgr.mark_connecting(&config.name);
        }
    } // lock released

    // Spawn background task to connect
    if config.enabled {
        let cfg = config.clone();
        tauri::async_runtime::spawn(async move {
            println!("[MCP] Background connecting to '{}'...", cfg.name);
            let connect_result = {
                let mut mgr = mgr_arc.lock().await;
                let result = mgr.connect_server(&cfg).await;
                mgr.clear_connecting(&cfg.name);
                if let Err(ref e) = result {
                    mgr.set_connection_error(&cfg.name, e.to_string());
                }
                result
            };

            match connect_result {
                Ok(()) => {
                    println!("[MCP] Connected '{}', refreshing tools...", cfg.name);
                    crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
                }
                Err(e) => {
                    eprintln!("[MCP] Connection failed for '{}': {}", cfg.name, e);
                }
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
) -> Result<(), String> {
    let mut mgr = manager.lock().await;
    mgr.remove_server(&name).await?;
    Ok(())
}

/// Refresh tools from all connected MCP servers and re-register in ActionRegistry.
#[tauri::command]
pub async fn refresh_mcp_tools(
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), String> {
    crate::mcp::bridge::register_mcp_tools(&manager.inner().clone(), registry.inner()).await;
    Ok(())
}

/// Retry connecting a disconnected MCP server.
#[tauri::command]
pub async fn reconnect_mcp_server(
    name: String,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), String> {
    let mgr_arc = manager.inner().clone();
    let reg_arc = registry.inner().clone();

    let cfg = {
        let mut mgr = mgr_arc.lock().await;
        let cfg = mgr
            .get_config(&name)
            .ok_or_else(|| format!("Server '{}' not found", name))?;
        // Disconnect existing (if any) before retrying
        let _ = mgr.disconnect_server(&name).await;
        mgr.mark_connecting(&name);
        cfg
    };

    tauri::async_runtime::spawn(async move {
        println!("[MCP] Retrying connection to '{}'...", cfg.name);
        let connect_result = {
            let mut mgr = mgr_arc.lock().await;
            let result = mgr.connect_server(&cfg).await;
            mgr.clear_connecting(&cfg.name);
            if let Err(ref e) = result {
                mgr.set_connection_error(&cfg.name, e.to_string());
            }
            result
        };

        match connect_result {
            Ok(()) => {
                println!("[MCP] Reconnected '{}', refreshing tools...", cfg.name);
                crate::mcp::bridge::register_mcp_tools(&mgr_arc, &reg_arc).await;
            }
            Err(e) => {
                eprintln!("[MCP] Reconnection failed for '{}': {}", cfg.name, e);
            }
        }
    });

    Ok(())
}
