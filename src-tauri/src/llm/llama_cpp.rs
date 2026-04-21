//! llama.cpp provider and metadata probe support.
//!
//! Chat requests use the OpenAI-compatible `/v1` endpoints, while API-page
//! metadata is read from llama.cpp-specific routes such as `/props`.

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_openai::Client;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::time::Duration;

use crate::llm::provider::{
    build_openai_client, create_chat, create_chat_stream, create_chat_stream_with_tools,
    list_model_ids, LlmParams, LlmProvider, LlmStreamEvent, LlmToolDefinition,
};

const DEFAULT_LLAMA_CPP_BASE_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlamaCppStatus {
    pub current_model: Option<String>,
    pub context_length: Option<usize>,
    #[serde(default)]
    pub available_models: Vec<String>,
}

pub struct LlamaCppProvider {
    client: Client<OpenAIConfig>,
    model: String,
    provider_id: String,
}

impl LlamaCppProvider {
    pub fn new(base_url: Option<String>, model: Option<String>, provider_id: String) -> Self {
        let compat_base = normalize_llama_cpp_chat_base_url(
            base_url.as_deref().unwrap_or(DEFAULT_LLAMA_CPP_BASE_URL),
        );

        Self {
            client: build_openai_client("llama.cpp".to_string(), Some(compat_base)),
            model: model.unwrap_or_default(),
            provider_id,
        }
    }

    pub async fn inspect_server(base_url: &str) -> Result<LlamaCppStatus, String> {
        let server_base = normalize_llama_cpp_server_base_url(base_url);
        let (models_result, props_result) = tokio::join!(
            Self::list_models(&server_base),
            fetch_server_props(&server_base)
        );

        if models_result.is_err() && props_result.is_err() {
            return Err(format!(
                "Failed to inspect llama.cpp server at {}: model listing failed ({}) and props probe failed ({})",
                server_base,
                models_result.err().unwrap_or_else(|| "unknown error".to_string()),
                props_result.err().unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        let mut available_models = models_result.unwrap_or_default();
        let props = props_result.ok();
        let current_model = props
            .as_ref()
            .and_then(extract_current_model_from_props)
            .or_else(|| available_models.first().cloned());

        if let Some(model) = &current_model {
            if !available_models.iter().any(|item| item == model) {
                available_models.push(model.clone());
            }
        }

        Ok(LlamaCppStatus {
            current_model,
            context_length: props.as_ref().and_then(extract_context_length_from_props),
            available_models,
        })
    }

    pub async fn list_models(base_url: &str) -> Result<Vec<String>, String> {
        let client = build_openai_client(
            "llama.cpp".to_string(),
            Some(normalize_llama_cpp_chat_base_url(base_url)),
        );

        let mut model_ids = list_model_ids(&client)
            .await
            .map_err(|e| format!("Failed to list llama.cpp models at {}: {}", base_url, e))?;
        model_ids.sort();
        model_ids.dedup();
        Ok(model_ids)
    }
}

#[async_trait]
impl LlmProvider for LlamaCppProvider {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        create_chat(&self.client, &self.model, messages, options).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        create_chat_stream(&self.client, &self.model, messages, options).await
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        create_chat_stream_with_tools(&self.client, &self.model, messages, options, tools).await
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}

async fn fetch_server_props(base_url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to build llama.cpp probe client: {}", e))?;

    let response = client
        .get(format!("{}/props", base_url))
        .send()
        .await
        .map_err(|e| format!("Failed to query llama.cpp /props: {}", e))?;

    let response = response
        .error_for_status()
        .map_err(|e| format!("llama.cpp /props returned error: {}", e))?;

    response
        .json::<Value>()
        .await
        .map_err(|e| format!("Failed to parse llama.cpp /props response: {}", e))
}

fn extract_current_model_from_props(props: &Value) -> Option<String> {
    extract_string_at_pointer(props, "/default_generation_settings/model")
        .or_else(|| extract_first_string_by_keys(props, &["model_name", "model_alias", "model"]))
        .or_else(|| {
            extract_first_string_by_keys(props, &["model_path"])
                .map(|path| model_name_from_path(&path))
        })
}

fn extract_context_length_from_props(props: &Value) -> Option<usize> {
    extract_number_at_pointer(props, "/default_generation_settings/n_ctx")
        .or_else(|| extract_number_at_pointer(props, "/n_ctx"))
        .or_else(|| extract_first_number_by_keys(props, &["n_ctx"]))
        .or_else(|| extract_number_at_pointer(props, "/context_length"))
        .or_else(|| {
            extract_first_number_by_keys(props, &["context_length", "ctx_size", "n_ctx_train"])
        })
}

fn extract_string_at_pointer(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(value_as_non_empty_string)
}

fn extract_number_at_pointer(value: &Value, pointer: &str) -> Option<usize> {
    value.pointer(pointer).and_then(value_as_usize)
}

fn extract_first_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_as_non_empty_string) {
                    return Some(found);
                }
            }
            for child in map.values() {
                if let Some(found) = extract_first_string_by_keys(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = extract_first_string_by_keys(item, keys) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_first_number_by_keys(value: &Value, keys: &[&str]) -> Option<usize> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_as_usize) {
                    return Some(found);
                }
            }
            for child in map.values() {
                if let Some(found) = extract_first_number_by_keys(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = extract_first_number_by_keys(item, keys) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn value_as_non_empty_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn value_as_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn model_name_from_path(path: &str) -> String {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path).trim();
    file_name
        .trim_end_matches(".gguf")
        .trim_end_matches(".bin")
        .to_string()
}

fn normalize_llama_cpp_server_base_url(base_url: &str) -> String {
    let mut normalized = base_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        normalized = DEFAULT_LLAMA_CPP_BASE_URL.to_string();
    }

    for suffix in ["/v1/chat/completions", "/chat/completions", "/v1"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            normalized = stripped.to_string();
            break;
        }
    }

    normalized.trim_end_matches('/').to_string()
}

fn normalize_llama_cpp_chat_base_url(base_url: &str) -> String {
    let server_base = normalize_llama_cpp_server_base_url(base_url);
    format!("{}/v1", server_base)
}

#[cfg(test)]
mod tests {
    use super::{
        model_name_from_path, normalize_llama_cpp_chat_base_url,
        normalize_llama_cpp_server_base_url, LlamaCppProvider,
    };
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalizes_llama_cpp_base_urls() {
        assert_eq!(
            normalize_llama_cpp_server_base_url("http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_llama_cpp_server_base_url("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_llama_cpp_chat_base_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/v1"
        );
    }

    #[test]
    fn derives_model_name_from_path() {
        assert_eq!(
            model_name_from_path("models/Qwen2.5-7B-Instruct.gguf"),
            "Qwen2.5-7B-Instruct"
        );
        assert_eq!(model_name_from_path(r"C:\models\llama3.bin"), "llama3");
    }

    #[tokio::test]
    async fn inspect_server_reads_models_and_context_length() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "Qwen2.5-7B-Instruct", "object": "model", "created": 0, "owned_by": "llama.cpp" }
                ]
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model_path": "models/Qwen2.5-7B-Instruct.gguf",
                "default_generation_settings": {
                    "n_ctx": 16384
                }
            })))
            .mount(&server)
            .await;

        let status = LlamaCppProvider::inspect_server(&server.uri())
            .await
            .expect("status probe should succeed");

        assert_eq!(status.current_model.as_deref(), Some("Qwen2.5-7B-Instruct"));
        assert_eq!(status.context_length, Some(16384));
        assert_eq!(
            status.available_models,
            vec!["Qwen2.5-7B-Instruct".to_string()]
        );
    }

    #[tokio::test]
    async fn inspect_server_falls_back_to_props_when_model_listing_fails() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model_path": "models/DeepSeek-R1.gguf",
                "n_ctx": 8192
            })))
            .mount(&server)
            .await;

        let status = LlamaCppProvider::inspect_server(&server.uri())
            .await
            .expect("props fallback should succeed");

        assert_eq!(status.current_model.as_deref(), Some("DeepSeek-R1"));
        assert_eq!(status.context_length, Some(8192));
        assert_eq!(status.available_models, vec!["DeepSeek-R1".to_string()]);
    }
}
