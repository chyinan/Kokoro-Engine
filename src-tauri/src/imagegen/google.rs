use crate::imagegen::interface::{ImageGenError, ImageGenParams, ImageGenProvider, ImageGenResponse};
use crate::imagegen::config::ImageGenProviderConfig;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct GoogleImageGenProvider {
    id: String,
    api_key: String,
    model: String, // e.g., "imagen-3.0-generate-001"
    client: Client,
}

impl GoogleImageGenProvider {
    pub fn new(config: &ImageGenProviderConfig) -> Result<Self, String> {
        let api_key = config.api_key.clone().ok_or("Google API Key is required")?;
        let model = config.model.clone().unwrap_or_else(|| "imagen-3.0-generate-001".to_string());
        
        // If empty string provided, fall back to default
        let model = if model.is_empty() { "imagen-3.0-generate-001".to_string() } else { model };

        Ok(Self {
            id: config.id.clone(),
            api_key,
            model,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client build should not fail"),
        })
    }
}

#[async_trait]
impl ImageGenProvider for GoogleImageGenProvider {
     fn id(&self) -> String {
        self.id.clone()
    }

    fn provider_type(&self) -> String {
        "google".to_string()
    }

    async fn is_available(&self) -> bool {
        // Simple verification: if we have an API key, we assume available.
        // A real check would ping a lightweight endpoint.
        !self.api_key.is_empty()
    }

    async fn generate(&self, params: ImageGenParams) -> Result<ImageGenResponse, ImageGenError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateImages?key={}",
            self.model, self.api_key
        );

        // Aspect ratio mapping
        let aspect_ratio = match params.size.as_deref() {
            Some("1024x1024") => "1:1",
            Some("16:9") => "16:9",
            Some("9:16") => "9:16",
            Some("3:4") => "3:4",
            Some("4:3") => "4:3",
            _ => "1:1", // Default
        };

        let body = json!({
            "prompt": params.prompt,
            "config": {
                "numberOfImages": 1,
                "aspectRatio": aspect_ratio,
                "outputOptions": {
                    "mimeType": "image/png"
                }
            }
        });

        let client = self.client.clone();
        let url_clone = url.clone();
        let body_clone = body.clone();

        let res = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_clone.clone();
                let body = body_clone.clone();
                async move {
                    client.post(&url)
                        .json(&body)
                        .send()
                        .await
                }
            },
            2,
        )
        .await
        .map_err(|e| ImageGenError::GenerationFailed(format!("Network Error: {}", e)))?;

        if !res.status().is_success() {
            let error_text = res.text().await.unwrap_or_default();
            return Err(ImageGenError::GenerationFailed(format!("Google API Error: {}", error_text)));
        }

        let json: serde_json::Value = res.json().await
            .map_err(|e| ImageGenError::GenerationFailed(format!("JSON Error: {}", e)))?;

        // Parse response
        // Structure: { "generatedImages": [ { "image": { "imageBytes": "..." } } ] }
        let generated_images = json.get("generatedImages")
            .and_then(|v| v.as_array())
            .ok_or(ImageGenError::GenerationFailed("Missing 'generatedImages' array".to_string()))?;

        if generated_images.is_empty() {
            return Err(ImageGenError::GenerationFailed("No images returned".to_string()));
        }

        let first_image = &generated_images[0];
        let b64_data = first_image.get("image")
            .and_then(|v| v.get("imageBytes"))
            .and_then(|v| v.as_str())
            .ok_or(ImageGenError::GenerationFailed("Missing 'image.imageBytes' field".to_string()))?;

        // Decode Base64
        use base64::{Engine as _, engine::general_purpose};
        let image_data = general_purpose::STANDARD
            .decode(b64_data)
            .map_err(|e| ImageGenError::GenerationFailed(format!("Base64 decode failed: {}", e)))?;

        Ok(ImageGenResponse {
            data: image_data,
            format: "png".to_string(), 
        })
    }
}
