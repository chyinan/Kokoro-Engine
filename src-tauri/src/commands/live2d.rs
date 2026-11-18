use crate::error::KokoroError;
use crate::live2d::{Live2dModelInfo, Live2dModelProfile, Live2dStore};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

#[tauri::command]
pub async fn import_live2d_zip(app: AppHandle, zip_path: String) -> Result<String, KokoroError> {
    model_store(&app)
        .map_err(KokoroError::Internal)?
        .import_zip(zip_path)
}

#[tauri::command]
pub async fn list_live2d_models(app: AppHandle) -> Result<Vec<Live2dModelInfo>, KokoroError> {
    model_store(&app)
        .map_err(KokoroError::Internal)?
        .list_models()
}

#[tauri::command]
pub async fn delete_live2d_model(app: AppHandle, model_name: String) -> Result<(), KokoroError> {
    model_store(&app)
        .map_err(KokoroError::Internal)?
        .delete_model(model_name)
}

#[tauri::command]
pub async fn rename_live2d_model(
    app: AppHandle,
    model_path: String,
    new_name: String,
) -> Result<String, String> {
    model_store(&app)?.rename_model(model_path, new_name)
}

#[tauri::command]
pub async fn import_live2d_folder(
    app: AppHandle,
    model_json_path: String,
) -> Result<String, KokoroError> {
    model_store(&app)
        .map_err(KokoroError::Internal)?
        .import_folder(model_json_path)
}

#[tauri::command]
pub async fn export_live2d_model(
    app: AppHandle,
    model_path: String,
    export_path: String,
) -> Result<String, String> {
    model_store(&app)?.export_model(model_path, export_path)
}

#[tauri::command]
pub async fn get_live2d_model_profile(
    app: AppHandle,
    model_path: String,
) -> Result<Live2dModelProfile, String> {
    model_store(&app)?.profile(&model_path)
}

#[tauri::command]
pub async fn save_live2d_model_profile(
    app: AppHandle,
    profile: Live2dModelProfile,
) -> Result<Live2dModelProfile, String> {
    let merged = model_store(&app)?.save_profile(profile)?;
    let _ = app.emit("live2d-profile-updated", &merged);
    Ok(merged)
}

#[tauri::command]
pub async fn set_active_live2d_model(
    app: AppHandle,
    model_path: Option<String>,
) -> Result<(), String> {
    model_store(&app)?.set_active_model(model_path.as_deref())
}

fn model_store(app: &AppHandle) -> Result<Live2dStore, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Cannot resolve app data dir: {error}"))?;
    Ok(Live2dStore::new(app_data)
        .with_active_model_state_path(legacy_app_data_dir().join("active_live2d_model.json")))
}

pub(crate) fn legacy_app_data_dir() -> PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
}
