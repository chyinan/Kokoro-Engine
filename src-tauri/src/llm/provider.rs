//! LLM Provider trait — common interface for all LLM backends.

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

pub use crate::llm::openai::{Message, MessageContent};

// ── Common Parameters ──────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct LlmParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub stop: Option<Vec<String>>,
}

/// Common interface for LLM providers (OpenAI, Ollama, etc.)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Non-streaming chat completion.
    async fn chat(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<String, String>;

    /// Streaming chat completion — yields content deltas.
    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String>;

    /// Provider identifier (e.g. "openai", "ollama").
    fn id(&self) -> &str;
}

// ── OpenAI adapter ─────────────────────────────────────

use crate::llm::openai::OpenAIClient;

/// Wraps the existing `OpenAIClient` to implement `LlmProvider`.
pub struct OpenAIProvider {
    client: OpenAIClient,
    provider_id: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: OpenAIClient::new(api_key, base_url, model),
            provider_id: "openai".to_string(),
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.provider_id = id;
        self
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        self.client.chat(messages, options).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        self.client.chat_stream(messages, options).await
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}
