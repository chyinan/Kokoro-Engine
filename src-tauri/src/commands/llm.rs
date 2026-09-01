//! Tauri commands for LLM config management.
// pattern: Imperative Shell

use crate::error::KokoroError;
use crate::llm::anthropic::{AnthropicModelInfo, AnthropicProvider};
use crate::llm::codex_runtime::{detect_codex_runtime, CodexRuntimeInfo, CodexRuntimeProvider};
use crate::llm::llama_cpp::{LlamaCppProvider, LlamaCppStatus};
use crate::llm::llm_config::LlmConfig;
use crate::llm::ollama::{OllamaModelInfo, OllamaProvider};
use crate::llm::service::{test_config_connection, LlmConnectionTestResult, LlmService};
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
pub async fn test_llm_connection(
    config: LlmConfig,
) -> Result<LlmConnectionTestResult, KokoroError> {
    test_config_connection(config).await
}

#[tauri::command]
pub async fn list_ollama_models(base_url: String) -> Result<Vec<OllamaModelInfo>, KokoroError> {
    OllamaProvider::list_models(&base_url)
        .await
        .map_err(KokoroError::Llm)
}

#[tauri::command]
pub async fn list_anthropic_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<AnthropicModelInfo>, KokoroError> {
    AnthropicProvider::list_models(&base_url, &api_key)
        .await
        .map_err(KokoroError::Llm)
}

#[tauri::command]
pub async fn get_llama_cpp_status(base_url: String) -> Result<LlamaCppStatus, KokoroError> {
    LlamaCppProvider::inspect_server(&base_url)
        .await
        .map_err(KokoroError::Llm)
}

#[tauri::command]
pub async fn get_codex_runtime_status() -> Result<CodexRuntimeInfo, KokoroError> {
    Ok(detect_codex_runtime())
}

#[tauri::command]
pub async fn list_codex_runtime_models() -> Result<Vec<String>, KokoroError> {
    CodexRuntimeProvider::new("codex-runtime-model-list".to_string(), None)
        .list_models()
        .await
        .map_err(KokoroError::Llm)
}
