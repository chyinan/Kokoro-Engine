//! Ollama provider — native streaming via `/api/chat`.
//!
//! Ollama streams newline-delimited JSON objects:
//! ```json
//! {"model":"llama3","message":{"role":"assistant","content":"Hi"},"done":false}
//! ```

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tauri::Emitter;

use crate::llm::openai::Message;
use crate::llm::provider::LlmProvider;

/// Ollama-native message format.
#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessageResponse>,
    done: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaMessageResponse {
    content: Option<String>,
}

/// Response from `GET /api/tags`.
#[derive(Debug, Deserialize)]
pub struct OllamaTagsResponse {
    pub models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
}

/// Progress update from `POST /api/pull`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPullProgress {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
}

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>, model: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
            model,
        }
    }

    /// List available models from the Ollama server.
    pub async fn list_models(base_url: &str) -> Result<Vec<OllamaModelInfo>, String> {
        let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
        let client = Client::new();

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Ollama at {}: {}", base_url, e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Ollama API error: {}", error_text));
        }

        let tags: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

        Ok(tags.models)
    }

    /// Pull (download) a model from Ollama library with streaming progress.
    /// Emits `ollama:pull-progress` events to the frontend.
    pub async fn pull_model(
        base_url: &str,
        model: &str,
        app_handle: tauri::AppHandle,
    ) -> Result<(), String> {
        let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
        let client = Client::new();

        let response = client
            .post(&url)
            .json(&serde_json::json!({ "model": model, "stream": true }))
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Ollama at {}: {}", base_url, e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Ollama pull error: {}", error_text));
        }

        // Stream newline-delimited JSON progress
        use futures::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            let bytes = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete lines
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Ok(progress) = serde_json::from_str::<OllamaPullProgress>(&line) {
                    let _ = app_handle.emit("ollama:pull-progress", &progress);
                }
            }
        }

        // Process any remaining data in buffer
        let remaining = buffer.trim().to_string();
        if !remaining.is_empty() {
            if let Ok(progress) = serde_json::from_str::<OllamaPullProgress>(&remaining) {
                let _ = app_handle.emit("ollama:pull-progress", &progress);
            }
        }

        Ok(())
    }

    fn convert_messages(messages: Vec<Message>) -> Vec<OllamaMessage> {
        messages
            .into_iter()
            .map(|m| OllamaMessage {
                role: m.role,
                content: m.content.text(),
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn chat(&self, messages: Vec<Message>) -> Result<String, String> {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let request_body = OllamaChatRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            stream: false,
        };

        let response = self
            .client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Ollama request failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Ollama API error: {}", error_text));
        }

        let chunk: OllamaStreamChunk = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

        Ok(chunk.message.and_then(|m| m.content).unwrap_or_default())
    }

    async fn chat_stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let request_body = OllamaChatRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            stream: true,
        };

        let response = self
            .client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Ollama request failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Ollama API error: {}", error_text));
        }

        // Ollama streams newline-delimited JSON
        let stream = response
            .bytes_stream()
            .map(|chunk_result| match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut contents = Vec::new();
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(chunk) = serde_json::from_str::<OllamaStreamChunk>(line) {
                            if chunk.done {
                                break;
                            }
                            if let Some(msg) = chunk.message {
                                if let Some(content) = msg.content {
                                    if !content.is_empty() {
                                        contents.push(content);
                                    }
                                }
                            }
                        }
                    }
                    if contents.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(contents.join("")))
                    }
                }
                Err(e) => Err(format!("Stream error: {}", e)),
            })
            .filter_map(|res| async {
                match res {
                    Ok(Some(content)) => Some(Ok(content)),
                    Ok(None) => None,
                    Err(e) => Some(Err(e)),
                }
            });

        Ok(Box::pin(stream))
    }

    fn id(&self) -> &str {
        "ollama"
    }
}
