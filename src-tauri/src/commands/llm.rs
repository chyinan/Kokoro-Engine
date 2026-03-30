//! Tauri commands for LLM config management.

use crate::error::KokoroError;
use crate::llm::llm_config::LlmConfig;
use crate::llm::ollama::{OllamaModelInfo, OllamaProvider};
use crate::llm::service::LlmService;
use tauri::State;

#[tauri::command]
pub async fn get_llm_config(state: State<'_, LlmService>) -> Result<LlmConfig, KokoroError> {
    Ok(state.config().await)
}

#[tauri::command]
pub async fn save_llm_config(
    config: LlmConfig,
    state: State<'_, LlmService>,
) -> Result<(), KokoroError> {
    state.update_config(config).await
}

#[tauri::command]
pub async fn list_ollama_models(base_url: String) -> Result<Vec<OllamaModelInfo>, KokoroError> {
    OllamaProvider::list_models(&base_url)
        .await
        .map_err(KokoroError::Llm)
}
