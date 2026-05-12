use crate::error::KokoroError;
use crate::vision::capture::{
    capture_screen_with_options, list_screens, CaptureOptions, ScreenInfo,
};
use crate::vision::config::VisionConfig;
use crate::vision::context::{VisionObservation, VisionObservationSource};
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
    server
        .upload(&file_bytes, &filename)
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn get_vision_config(
    state: State<'_, VisionWatcher>,
) -> Result<VisionConfig, KokoroError> {
    let config = state.config.read().await;
    Ok(config.clone())
}

#[tauri::command]
pub async fn list_vision_screens() -> Result<Vec<ScreenInfo>, KokoroError> {
    list_screens().map_err(KokoroError::ExternalService)
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
    let was_auto_enabled = {
        let current = state.config.read().await;
        current.vlm_enabled && current.auto_vision_enabled
    };
    *state.config.write().await = config.clone();
    let auto_enabled = config.vlm_enabled && config.auto_vision_enabled;
    if auto_enabled && !was_auto_enabled {
        state.start(app_handle.clone());
    } else if !auto_enabled && was_auto_enabled {
        state.stop();
    } else if !auto_enabled {
        state.context.clear_auto_state_on_disable().await;
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
pub async fn set_vision_text_input_focused(
    state: State<'_, VisionWatcher>,
    focused: bool,
) -> Result<(), KokoroError> {
    state.context.set_text_input_focus_state(focused).await;
    Ok(())
}

#[tauri::command]
pub async fn capture_screen_now(
    state: State<'_, VisionWatcher>,
    llm_service: State<'_, crate::llm::service::LlmService>,
) -> Result<String, KokoroError> {
    let config = state.config.read().await.clone();
    if !config.vlm_enabled {
        return Err(KokoroError::ExternalService(
            "Screen VLM is disabled".to_string(),
        ));
    }
    let captured = capture_screen_with_options(&CaptureOptions {
        display_id: config.display_id.clone(),
        region: config.vlm_region,
    })
    .map_err(|e| KokoroError::ExternalService(e.to_string()))?;
    if let Some(warning) = captured.warning.clone() {
        state.context.set_last_error(warning).await;
    }
    let client = state.client.clone();
    let captured_at = chrono::Utc::now();
    let description = crate::vision::watcher::analyze_screenshot(
        &client,
        &config,
        &captured.jpeg_bytes,
        Some(&llm_service),
    )
    .await
    .map_err(|e| KokoroError::ExternalService(e.to_string()))?;
    state
        .context
        .record_manual_observation(VisionObservation {
            id: uuid::Uuid::new_v4().to_string(),
            frame_id: None,
            captured_at,
            analyzed_at: chrono::Utc::now(),
            summary: description.clone(),
            source: VisionObservationSource::ManualTool,
        })
        .await;
    Ok(description)
}
