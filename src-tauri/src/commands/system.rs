use crate::ai::context::AIOrchestrator;
use crate::commands::live2d::load_active_live2d_profile;
use crate::error::KokoroError;
use serde::Serialize;
use std::sync::Arc;
use tauri::{command, Manager, State};
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
pub fn get_system_status(app: tauri::AppHandle, state: State<'_, AIOrchestrator>) -> SystemStatus {
    let mut active_modules = Vec::new();

    if app.try_state::<AIOrchestrator>().is_some() {
        active_modules.push("core".to_string());
    }
    if app.try_state::<crate::tts::TtsService>().is_some() {
        active_modules.push("tts".to_string());
    }
    if app.try_state::<crate::stt::SttService>().is_some() {
        active_modules.push("stt".to_string());
    }
    if app.try_state::<crate::imagegen::ImageGenService>().is_some() {
        active_modules.push("imagegen".to_string());
    }
    if app
        .try_state::<std::sync::Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>()
        .is_some()
    {
        active_modules.push("mcp".to_string());
    }
    if app
        .try_state::<tokio::sync::Mutex<crate::mods::ModManager>>()
        .is_some()
    {
        active_modules.push("mods".to_string());
    }
    if app
        .try_state::<std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>>()
        .is_some()
    {
        active_modules.push("vision".to_string());
    }
    if load_active_live2d_profile().is_some() {
        active_modules.push("live2d".to_string());
    }
    if state.is_memory_enabled() {
        active_modules.push("memory".to_string());
    }
    if state.is_proactive_enabled() {
        active_modules.push("proactive".to_string());
    }

    SystemStatus {
        engine_running: !active_modules.is_empty(),
        active_modules,
        memory_usage_mb: 0.0,
    }
}
