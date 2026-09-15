// pattern: Imperative Shell
use crate::ai::emotion_onnx::{
    download_emotion_model as ai_download_emotion_model,
    get_emotion_model_status as ai_get_emotion_model_status,
    import_emotion_model_package as ai_import_emotion_model_package,
    infer_emotion as ai_infer_emotion,
    open_emotion_model_dir as ai_open_emotion_model_dir,
    toggle_emotion_model_active as ai_toggle_emotion_model_active,
    uninstall_emotion_model as ai_uninstall_emotion_model, EmotionInferenceResult,
    EmotionModelStatus,
};
use crate::error::KokoroError;

#[tauri::command]
pub async fn get_emotion_model_status() -> Result<EmotionModelStatus, KokoroError> {
    Ok(ai_get_emotion_model_status())
}

#[tauri::command]
pub async fn download_emotion_model(
    app: tauri::AppHandle,
) -> Result<EmotionModelStatus, KokoroError> {
    use tauri::Emitter;

    ai_download_emotion_model(move |progress| {
        app.emit("emotion:model-progress", &progress)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn uninstall_emotion_model() -> Result<EmotionModelStatus, KokoroError> {
    ai_uninstall_emotion_model().map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn toggle_emotion_model(active: bool) -> Result<EmotionModelStatus, KokoroError> {
    ai_toggle_emotion_model_active(active).map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn infer_emotion(text: String) -> Result<EmotionInferenceResult, KokoroError> {
    tokio::task::spawn_blocking(move || ai_infer_emotion(&text))
        .await
        .map_err(|e| KokoroError::Internal(format!("Task execution error: {}", e)))?
        .map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn open_emotion_model_dir() -> Result<String, KokoroError> {
    ai_open_emotion_model_dir().map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn import_emotion_model_package(
    source_path: String,
) -> Result<EmotionModelStatus, KokoroError> {
    tokio::task::spawn_blocking(move || ai_import_emotion_model_package(&source_path))
        .await
        .map_err(|e| KokoroError::Internal(format!("Import task failed: {}", e)))?
        .map_err(KokoroError::Internal)
}

