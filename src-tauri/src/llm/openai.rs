use super::provider::LlmParams;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text content (serializes as a JSON string)
    Text(String),
    /// Array of content parts for multimodal messages (text + images)
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlDetail },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    pub url: String,
}

impl MessageContent {
    /// Extract the text content, ignoring any image parts.
    pub fn text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Create a multimodal content with text and image URLs.
    pub fn with_images(text: String, image_urls: Vec<String>) -> Self {
        let mut parts = vec![ContentPart::Text { text }];
        for url in image_urls {
            parts.push(ContentPart::ImageUrl {
                image_url: ImageUrlDetail { url },
            });
        }
        MessageContent::Parts(parts)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    _finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

pub struct OpenAIClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAIClient {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
        }
    }

    /// Non-streaming chat completion for internal tool-use (e.g. memory extraction).
    pub async fn chat(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let opts = options.unwrap_or_default();
        let request_body = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: false,
            temperature: opts.temperature.or(Some(0.3)),
            max_tokens: opts.max_tokens,
            top_p: opts.top_p,
            frequency_penalty: opts.frequency_penalty,
            presence_penalty: opts.presence_penalty,
            stop: opts.stop,
        };

        let client = self.client.clone();
        let url_clone = url.clone();
        let api_key = self.api_key.clone();
        let body = request_body.clone();

        let response = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_clone.clone();
                let body = body.clone();
                let api_key = api_key.clone();
                async move {
                    client
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", api_key))
                        .header("Content-Type", "application/json")
                        .json(&body)
                        .send()
                        .await
                }
            },
            2,
        )
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("API Error: {}", error_text));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<Message>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let opts = options.unwrap_or_default();
        let request_body = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            temperature: opts.temperature.or(Some(0.7)),
            max_tokens: opts.max_tokens,
            top_p: opts.top_p,
            frequency_penalty: opts.frequency_penalty,
            presence_penalty: opts.presence_penalty,
            stop: opts.stop,
        };

        let client = self.client.clone();
        let url_clone = url.clone();
        let api_key = self.api_key.clone();
        let body = request_body.clone();

        let response = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_clone.clone();
                let body = body.clone();
                let api_key = api_key.clone();
                async move {
                    client
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", api_key))
                        .header("Content-Type", "application/json")
                        .json(&body)
                        .send()
                        .await
                }
            },
            2,
        )
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("API Error: {}", error_text));
        }

        let stream = response
            .bytes_stream()
            .eventsource()
            .map(|result| {
                match result {
                    Ok(event) => {
                        if event.data == "[DONE]" {
                            return Ok(None);
                        }

                        match serde_json::from_str::<OpenAIStreamResponse>(&event.data) {
                            Ok(parsed) => {
                                if let Some(choice) = parsed.choices.first() {
                                    if let Some(content) = &choice.delta.content {
                                        return Ok(Some(content.clone()));
                                    }
                                }
                                Ok(None)
                            }
                            Err(_) => Ok(None), // Ignore parse errors for keep-alives etc
                        }
                    }
                    Err(e) => Err(format!("Stream error: {}", e)),
                }
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
}
