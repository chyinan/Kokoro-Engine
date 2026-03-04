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
                .expect("HTTP client build should not fail"),
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

        // 检查 choices 数组是否存在且非空
        let choices = body["choices"]
            .as_array()
            .ok_or_else(|| "API response missing 'choices' array".to_string())?;

        let first_choice = choices
            .first()
            .ok_or_else(|| "API response 'choices' array is empty".to_string())?;

        let content = first_choice["message"]["content"]
            .as_str()
            .ok_or_else(|| "API response missing 'message.content'".to_string())?
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
                            Err(e) => {
                                // 区分空行/keep-alive 和真正的解析错误
                                let trimmed = event.data.trim();
                                if trimmed.is_empty() {
                                    Ok(None)
                                } else if trimmed.starts_with('{') {
                                    // 看起来是 JSON 但解析失败，可能是 API 错误响应
                                    eprintln!("[LLM/OpenAI] Stream JSON parse error: {} — data: {}", e, &trimmed[..trimmed.len().min(200)]);
                                    Err(format!("Stream parse error: {}", e))
                                } else {
                                    // 非 JSON 数据，可能是 keep-alive 或注释
                                    Ok(None)
                                }
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_variant_returns_string() {
        let c = MessageContent::Text("hello".to_string());
        assert_eq!(c.text(), "hello");
    }

    #[test]
    fn test_parts_variant_concatenates_text_parts() {
        let c = MessageContent::Parts(vec![
            ContentPart::Text { text: "foo".to_string() },
            ContentPart::Text { text: "bar".to_string() },
        ]);
        assert_eq!(c.text(), "foobar");
    }

    #[test]
    fn test_parts_variant_ignores_image_parts() {
        let c = MessageContent::Parts(vec![
            ContentPart::Text { text: "hello".to_string() },
            ContentPart::ImageUrl {
                image_url: ImageUrlDetail { url: "http://example.com/img.png".to_string() },
            },
        ]);
        assert_eq!(c.text(), "hello");
    }

    #[test]
    fn test_parts_variant_empty_returns_empty_string() {
        let c = MessageContent::Parts(vec![]);
        assert_eq!(c.text(), "");
    }

    #[test]
    fn test_with_images_creates_parts_variant() {
        let c = MessageContent::with_images("desc".to_string(), vec!["http://img".to_string()]);
        assert!(matches!(c, MessageContent::Parts(_)));
    }

    #[test]
    fn test_with_images_no_urls_only_text_part() {
        let c = MessageContent::with_images("only text".to_string(), vec![]);
        assert_eq!(c.text(), "only text");
        if let MessageContent::Parts(parts) = &c {
            assert_eq!(parts.len(), 1);
        } else {
            panic!("expected Parts variant");
        }
    }

    #[test]
    fn test_with_images_multiple_urls() {
        let c = MessageContent::with_images(
            "caption".to_string(),
            vec!["url1".to_string(), "url2".to_string()],
        );
        if let MessageContent::Parts(parts) = &c {
            assert_eq!(parts.len(), 3); // 1 text + 2 images
        } else {
            panic!("expected Parts variant");
        }
    }

    #[test]
    fn test_text_variant_serializes_as_string() {
        let c = MessageContent::Text("hi".to_string());
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"hi\"");
    }

    #[test]
    fn test_text_variant_deserializes_from_string() {
        let c: MessageContent = serde_json::from_str("\"world\"").unwrap();
        assert_eq!(c.text(), "world");
    }
}
