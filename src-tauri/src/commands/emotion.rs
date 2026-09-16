// pattern: Imperative Shell
use crate::ai::emotion_onnx::{
    cancel_emotion_model_download as ai_cancel_emotion_model_download,
    download_emotion_model as ai_download_emotion_model,
    get_emotion_model_status as ai_get_emotion_model_status,
    import_emotion_model_package as ai_import_emotion_model_package,
    infer_emotion as ai_infer_emotion, open_emotion_model_dir as ai_open_emotion_model_dir,
    toggle_emotion_model_active as ai_toggle_emotion_model_active,
    uninstall_emotion_model as ai_uninstall_emotion_model, EmotionInferenceResult,
    EmotionModelStatus,
};
use crate::error::KokoroError;

#[tauri::command]
pub async fn get_emotion_model_status() -> Result<EmotionModelStatus, KokoroError> {
    tokio::task::spawn_blocking(ai_get_emotion_model_status)
        .await
        .map_err(|e| KokoroError::Internal(format!("Status task failed: {}", e)))
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
pub async fn cancel_emotion_model_download() -> Result<bool, KokoroError> {
    ai_cancel_emotion_model_download().map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn uninstall_emotion_model() -> Result<EmotionModelStatus, KokoroError> {
    tokio::task::spawn_blocking(ai_uninstall_emotion_model)
        .await
        .map_err(|e| KokoroError::Internal(format!("Uninstall task failed: {}", e)))?
        .map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn toggle_emotion_model(active: bool) -> Result<EmotionModelStatus, KokoroError> {
    tokio::task::spawn_blocking(move || ai_toggle_emotion_model_active(active))
        .await
        .map_err(|e| KokoroError::Internal(format!("Toggle task failed: {}", e)))?
        .map_err(KokoroError::Internal)
}

#[tauri::command]
pub async fn infer_emotion(text: String) -> Result<EmotionInferenceResult, KokoroError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || ai_infer_emotion(&text)),
    )
    .await
    .map_err(|_| KokoroError::Internal("Emotion inference timed out after 5s".to_string()))?
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
