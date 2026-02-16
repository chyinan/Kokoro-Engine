use crate::imagegen::{ImageGenError, ImageGenParams, ImageGenProvider, ImageGenResponse};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

pub struct OpenAIImageGenProvider {
    id: String,
    api_key: String,
    base_url: String, // Defaults to "https://api.openai.com/v1"
    model: String,    // Defaults to "dall-e-3"
    client: Client,
}

impl OpenAIImageGenProvider {
    pub fn new(id: String, api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            id,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "dall-e-3".to_string()),
            client: Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    prompt: String,
    n: usize,
    size: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    response_format: String, // Always "b64_json"
}

#[async_trait]
impl ImageGenProvider for OpenAIImageGenProvider {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn provider_type(&self) -> String {
        "openai".to_string()
    }

    async fn is_available(&self) -> bool {
        // Simple check (could try listing models, but for now just assume true if key exists)
        !self.api_key.is_empty()
    }

    async fn generate(&self, params: ImageGenParams) -> Result<ImageGenResponse, ImageGenError> {
        let url = format!("{}/images/generations", self.base_url.trim_end_matches('/'));

        let body = OpenAIRequest {
            model: self.model.clone(),
            prompt: params.prompt,
            n: 1, // DALL-E 3 only supports n=1
            size: params.size.unwrap_or_else(|| "1024x1024".to_string()),
            quality: params.quality, // e.g. "standard" or "hd"
            style: params.style,     // e.g. "vivid" or "natural"
            response_format: "b64_json".to_string(),
        };

        let res = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| ImageGenError::GenerationFailed(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ImageGenError::GenerationFailed(format!(
                "OpenAI API Error {}: {}",
                status, text
            )));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| ImageGenError::GenerationFailed(format!("Invalid JSON: {}", e)))?;

        // Extract b64_json
        // Response format: { "created": ..., "data": [ { "b64_json": "..." } ] }
        if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            if let Some(first) = data.first() {
                if let Some(b64) = first.get("b64_json").and_then(|v| v.as_str()) {
                    // Decode base64
                    let bytes = general_purpose::STANDARD
                        .decode(b64)
                        .map_err(|e| ImageGenError::GenerationFailed(format!("Base64 decode error: {}", e)))?;

                    return Ok(ImageGenResponse {
                        format: "png".to_string(), // DALL-E returns PNG
                        data: bytes,
                    });
                }
            }
        }

        Err(ImageGenError::GenerationFailed(
            "Response missing 'data[0].b64_json'".to_string(),
        ))
    }
}
