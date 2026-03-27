use crate::error::KokoroError;
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
) -> Result<String, KokoroError> {
    let server = state.lock().await;
    server.upload(&file_bytes, &filename).map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn get_vision_config(state: State<'_, VisionWatcher>) -> Result<VisionConfig, KokoroError> {
    let config = state.config.read().await;
    Ok(config.clone())
}

#[tauri::command]
pub async fn save_vision_config(
    app_handle: AppHandle,
    state: State<'_, VisionWatcher>,
    config: VisionConfig,
) -> Result<(), KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("vision_config.json");
    crate::vision::config::save_config(&config_path, &config).map_err(KokoroError::Config)?;
    let was_enabled = state.config.read().await.enabled;
    *state.config.write().await = config.clone();
    if config.enabled && !was_enabled {
        state.start(app_handle.clone());
    } else if !config.enabled && was_enabled {
        state.stop();
    }
    Ok(())
}

#[tauri::command]
pub async fn start_vision_watcher(
    app_handle: AppHandle,
    state: State<'_, VisionWatcher>,
) -> Result<(), KokoroError> {
    state.start(app_handle);
    Ok(())
}

#[tauri::command]
pub async fn stop_vision_watcher(state: State<'_, VisionWatcher>) -> Result<(), KokoroError> {
    state.stop();
    Ok(())
}

#[tauri::command]
pub async fn capture_screen_now(
    state: State<'_, VisionWatcher>,
    llm_service: State<'_, crate::llm::service::LlmService>,
) -> Result<String, KokoroError> {
    let screenshot = capture_screen().map_err(|e| KokoroError::ExternalService(e.to_string()))?;
    let config = state.config.read().await.clone();
    let client = state.client.clone();
    let description = crate::vision::watcher::analyze_screenshot(&client, &config, &screenshot, Some(&llm_service))
        .await
        .map_err(|e| KokoroError::ExternalService(e.to_string()))?;
    state.context.update(description.clone()).await;
    Ok(description)
}
