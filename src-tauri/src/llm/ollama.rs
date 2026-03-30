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

use crate::llm::provider::{
    build_openai_client, create_chat, create_chat_stream, create_chat_stream_with_tools,
    list_model_ids, LlmParams, LlmProvider, LlmStreamEvent, LlmToolDefinition,
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
        let client = build_openai_client(
            "ollama".to_string(),
            Some(normalize_ollama_chat_base_url(base_url)),
        );

        let model_ids = list_model_ids(&client)
            .await
            .map_err(|e| format!("Failed to list Ollama models at {}: {}", base_url, e))?;

        Ok(model_ids
            .into_iter()
            .map(|model_id| OllamaModelInfo {
                name: model_id,
                size: None,
                modified_at: None,
            })
            .collect())
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
