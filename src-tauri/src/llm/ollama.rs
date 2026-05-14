//! Ollama provider.
//!
//! Chat traffic and model listing are routed through Ollama's OpenAI-compatible
//! `/v1` endpoints using `async-openai`.

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_openai::Client;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;

use crate::llm::provider::{
    build_openai_client, create_chat, create_chat_stream, create_chat_stream_with_tools, LlmParams,
    LlmProvider, LlmStreamEvent, LlmToolDefinition,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
}

pub struct OllamaProvider {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>, model: String) -> Self {
        let compat_base =
            normalize_ollama_chat_base_url(base_url.as_deref().unwrap_or("http://localhost:11434"));

        Self {
            client: build_openai_client("ollama".to_string(), Some(compat_base)),
            model,
        }
    }

    /// List available models from the Ollama server.
    pub async fn list_models(base_url: &str) -> Result<Vec<OllamaModelInfo>, String> {
        let endpoint = format!("{}/models", normalize_ollama_chat_base_url(base_url));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .map_err(|e| format!("Failed to build Ollama probe client: {}", e))?;
        let response = client
            .get(&endpoint)
            .send()
            .await
            .map_err(|e| format!("Failed to list Ollama models at {}: {}", base_url, e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!(
                "Ollama /v1/models returned error {}: {}",
                status, error_text
            ));
        }

        let payload = response
            .json::<OllamaModelListResponse>()
            .await
            .map_err(|e| format!("Failed to parse Ollama /v1/models response: {}", e))?;

        Ok(payload
            .data
            .into_iter()
            .filter_map(|model| non_empty_trimmed(model.id))
            .map(|name| OllamaModelInfo {
                name,
                size: None,
                modified_at: None,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct OllamaModelListResponse {
    #[serde(default)]
    data: Vec<OllamaOpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaOpenAiModel {
    id: String,
}

fn non_empty_trimmed(text: String) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
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

    fn id(&self) -> &str {
        "ollama"
    }
}

fn normalize_ollama_chat_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{}/v1", trimmed)
    }
}
