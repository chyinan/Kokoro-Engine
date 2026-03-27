use crate::error::KokoroError;
use serde::Serialize;
use std::sync::Arc;
use tauri::{command, State};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct WindowSizeState {
    pub width: Arc<RwLock<u32>>,
    pub height: Arc<RwLock<u32>>,
}

impl Default for WindowSizeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowSizeState {
    pub fn new() -> Self {
        Self {
            width: Arc::new(RwLock::new(800)),
            height: Arc::new(RwLock::new(600)),
        }
    }
    pub async fn get(&self) -> (u32, u32) {
        (*self.width.read().await, *self.height.read().await)
    }
    pub async fn set(&self, w: u32, h: u32) {
        *self.width.write().await = w;
        *self.height.write().await = h;
    }
}

#[command]
pub async fn set_window_size(
    state: State<'_, WindowSizeState>,
    width: u32,
    height: u32,
) -> Result<(), KokoroError> {
    state.set(width, height).await;
    Ok(())
}

#[derive(Serialize)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
}

#[derive(Serialize)]
pub struct SystemStatus {
    pub engine_running: bool,
    pub active_modules: Vec<String>,
    pub memory_usage_mb: f64,
}

/// Returns basic engine metadata for the frontend to display.
#[tauri::command]
pub fn get_engine_info() -> EngineInfo {
    EngineInfo {
        name: "Kokoro Engine".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
    }
}

/// Returns the current system status including active modules.
#[tauri::command]
pub fn get_system_status() -> SystemStatus {
    SystemStatus {
        engine_running: true,
        active_modules: vec!["core".to_string(), "ui".to_string()],
        memory_usage_mb: 0.0, // TODO: implement actual memory tracking
    }
}
