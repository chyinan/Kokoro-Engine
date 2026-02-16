use serde::Serialize;

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
