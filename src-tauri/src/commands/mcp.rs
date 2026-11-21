// pattern: Imperative Shell

use crate::actions::ActionRegistry;
use crate::error::KokoroError;
use crate::mcp::{bridge, manager::McpServerStatus, McpManager, McpServerConfig};
use std::sync::Arc;
use tauri::State;
use tokio::sync::{Mutex, RwLock};

#[tauri::command]
#[specta::specta]
pub async fn list_mcp_servers(
    manager: State<'_, Arc<Mutex<McpManager>>>,
) -> Result<Vec<McpServerStatus>, KokoroError> {
    Ok(manager.lock().await.list_status())
}

#[tauri::command]
#[specta::specta]
pub async fn add_mcp_server(
    config: McpServerConfig,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    bridge::add_server(manager.inner(), registry.inner(), config).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_mcp_server(
    name: String,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    bridge::remove_server(manager.inner(), registry.inner(), &name).await
}

#[tauri::command]
#[specta::specta]
pub async fn refresh_mcp_tools(
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    bridge::refresh_all(manager.inner(), registry.inner()).await
}

#[tauri::command]
#[specta::specta]
pub async fn reconnect_mcp_server(
    name: String,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    bridge::reconnect_server(manager.inner(), registry.inner(), &name).await
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_mcp_server(
    name: String,
    enabled: bool,
    manager: State<'_, Arc<Mutex<McpManager>>>,
    registry: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<(), KokoroError> {
    bridge::toggle_server(manager.inner(), registry.inner(), &name, enabled).await
}
