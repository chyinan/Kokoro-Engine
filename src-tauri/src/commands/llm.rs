//! Tauri commands for LLM config management.

use crate::llm::llm_config::LlmConfig;
use crate::llm::ollama::{OllamaModelInfo, OllamaProvider};
use crate::llm::service::LlmService;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn get_llm_config(state: State<'_, LlmService>) -> Result<LlmConfig, String> {
    Ok(state.config().await)
}

#[tauri::command]
pub async fn save_llm_config(
    config: LlmConfig,
    state: State<'_, LlmService>,
) -> Result<(), String> {
    state.update_config(config).await
}

#[tauri::command]
pub async fn list_ollama_models(base_url: String) -> Result<Vec<OllamaModelInfo>, String> {
    OllamaProvider::list_models(&base_url).await
}

#[tauri::command]
pub async fn pull_ollama_model(
    app_handle: AppHandle,
    base_url: String,
    model: String,
) -> Result<(), String> {
    OllamaProvider::pull_model(&base_url, &model, app_handle).await
}
