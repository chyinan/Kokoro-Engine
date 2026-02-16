use crate::vision::capture::capture_screen;
use crate::vision::config::VisionConfig;
use crate::vision::server::VisionServer;
use crate::vision::watcher::VisionWatcher;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

#[tauri::command]
pub async fn upload_vision_image(
    state: State<'_, Arc<Mutex<VisionServer>>>,
    file_bytes: Vec<u8>,
    filename: String,
) -> Result<String, String> {
    let server = state.lock().await;
    server.upload(&file_bytes, &filename)
}

// ── Vision Config Commands ─────────────────────────────

#[tauri::command]
pub async fn get_vision_config(state: State<'_, VisionWatcher>) -> Result<VisionConfig, String> {
    let config = state.config.read().await;
    Ok(config.clone())
}

#[tauri::command]
pub async fn save_vision_config(
    app_handle: AppHandle,
    state: State<'_, VisionWatcher>,
    config: VisionConfig,
) -> Result<(), String> {
    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("vision_config.json");
    crate::vision::config::save_config(&config_path, &config)?;

    // Update in-memory config
    let was_enabled = state.config.read().await.enabled;
    *state.config.write().await = config.clone();

    // Start/stop watcher based on enabled state
    if config.enabled && !was_enabled {
        state.start(app_handle.clone());
    } else if !config.enabled && was_enabled {
        state.stop();
    }

    Ok(())
}

// ── Vision Watcher Control ─────────────────────────────

#[tauri::command]
pub async fn start_vision_watcher(
    app_handle: AppHandle,
    state: State<'_, VisionWatcher>,
) -> Result<(), String> {
    state.start(app_handle);
    Ok(())
}

#[tauri::command]
pub async fn stop_vision_watcher(state: State<'_, VisionWatcher>) -> Result<(), String> {
    state.stop();
    Ok(())
}

// ── One-shot Capture (for testing) ────────────────────

#[tauri::command]
pub async fn capture_screen_now(state: State<'_, VisionWatcher>) -> Result<String, String> {
    let screenshot = capture_screen()?;
    let config = state.config.read().await.clone();

    let client = reqwest::Client::new();
    let description =
        crate::vision::watcher::analyze_screenshot(&client, &config, &screenshot).await?;

    state.context.update(description.clone()).await;
    Ok(description)
}
