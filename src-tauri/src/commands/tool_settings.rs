use crate::actions::tool_settings::{self, ToolSettings};
use crate::error::KokoroError;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

fn tool_settings_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("tool_settings.json")
}

#[tauri::command]
pub async fn get_tool_settings(
    state: State<'_, Arc<RwLock<ToolSettings>>>,
) -> Result<ToolSettings, KokoroError> {
    Ok(state.read().await.clone())
}

#[tauri::command]
pub async fn save_tool_settings(
    settings: ToolSettings,
    state: State<'_, Arc<RwLock<ToolSettings>>>,
) -> Result<(), KokoroError> {
    let sanitized = settings.sanitized();
    {
        let mut guard = state.write().await;
        *guard = sanitized.clone();
    }
    tool_settings::save_config(&tool_settings_path(), &sanitized)
}
