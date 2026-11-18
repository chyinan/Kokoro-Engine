//! Tauri commands for LLM config management.
// pattern: Imperative Shell

use crate::error::KokoroError;
use crate::ipc::wire::{JsonPayload, Response};
use crate::llm::anthropic::{AnthropicModelInfo, AnthropicProvider};
use crate::llm::codex_runtime::{detect_codex_runtime, CodexRuntimeInfo, CodexRuntimeProvider};
use crate::llm::llama_cpp::{LlamaCppProvider, LlamaCppStatus};
use crate::llm::llm_config::LlmConfig;
use crate::llm::ollama::{OllamaModelInfo, OllamaProvider};
use crate::llm::service::{test_config_connection, LlmConnectionTestResult, LlmService};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn get_llm_config(state: State<'_, LlmService>) -> Result<Response<LlmConfig>, KokoroError> {
    Ok(Response(state.config().await))
}

#[tauri::command]
#[specta::specta]
pub async fn save_llm_config(
    config: LlmConfig,
    state: State<'_, LlmService>,
) -> Result<(), KokoroError> {
    state.update_config(config).await
}

#[tauri::command]
#[specta::specta]
pub async fn test_llm_connection(
    config: LlmConfig,
) -> Result<LlmConnectionTestResult, KokoroError> {
    test_config_connection(config).await
}

/// MOD messages carry JSON configuration from the sandbox boundary. Apply the
/// same Serde defaults and connection test used by the typed settings command.
#[tauri::command]
#[specta::specta]
pub async fn test_mod_llm_connection(
    config: JsonPayload,
) -> Result<LlmConnectionTestResult, KokoroError> {
    let config = serde_json::from_value(config.0)
        .map_err(|error| KokoroError::Validation(format!("Invalid LLM configuration: {error}")))?;
    test_llm_connection(config).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_ollama_models(base_url: String) -> Result<Response<Vec<OllamaModelInfo>>, KokoroError> {
    OllamaProvider::list_models(&base_url)
        .await
        .map_err(KokoroError::Llm)
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn list_anthropic_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<AnthropicModelInfo>, KokoroError> {
    AnthropicProvider::list_models(&base_url, &api_key)
        .await
        .map_err(KokoroError::Llm)
}

#[tauri::command]
#[specta::specta]
pub async fn get_llama_cpp_status(base_url: String) -> Result<Response<LlamaCppStatus>, KokoroError> {
    LlamaCppProvider::inspect_server(&base_url)
        .await
        .map_err(KokoroError::Llm)
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn get_codex_runtime_status() -> Result<CodexRuntimeInfo, KokoroError> {
    Ok(detect_codex_runtime())
}

#[tauri::command]
#[specta::specta]
pub async fn list_codex_runtime_models() -> Result<Vec<String>, KokoroError> {
    CodexRuntimeProvider::new("codex-runtime-model-list".to_string(), None)
        .list_models()
        .await
        .map_err(KokoroError::Llm)
}

#[cfg(test)]
mod tests {
    use super::test_mod_llm_connection;
    use crate::error::KokoroError;
    use crate::ipc::wire::JsonPayload;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn mod_llm_test_uses_nested_serde_defaults_before_connecting() {
        let malformed = test_mod_llm_connection(JsonPayload(json!({"providers": [42]}))).await;
        assert!(matches!(malformed, Err(KokoroError::Validation(_))));

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "fixture", "object": "chat.completion", "created": 1, "model": "fixture",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop", "logprobs": null}],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result = test_mod_llm_connection(JsonPayload(json!({
            "active_provider": "fixture",
            "providers": [{
                "id": "fixture", "provider_type": "openai", "model": "fixture",
                "base_url": format!("{}/v1", server.uri()), "api_key": "local-test-fixture"
            }]
        })))
        .await
        .unwrap();
        assert_eq!(result.tested_targets[0].provider_id, "fixture");
    }
}
