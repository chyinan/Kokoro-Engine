use crate::imagegen::{ImageGenError, ImageGenParams, ImageGenProvider, ImageGenResponse};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

pub struct StableDiffusionProvider {
    id: String,
    base_url: String,       // Defaults to "http://127.0.0.1:7860"
    _model: Option<String>, // Optional checkpoint override (not always supported nicely via API without extra call, so maybe just ignored or used for SDXL refiner)
    client: Client,
}

impl StableDiffusionProvider {
    pub fn new(id: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            id,
            base_url: base_url.unwrap_or_else(|| "http://127.0.0.1:7860".to_string()),
            _model: model,
            client: Client::builder().no_proxy().build().unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct SdTxt2ImgRequest {
    prompt: String,
    negative_prompt: String,
    seed: i64,
    styles: Vec<String>,
    width: u32,
    height: u32,
    steps: u32,
    cfg_scale: f32,
    sampler_name: Option<String>,
    batch_size: usize,
}

#[async_trait]
impl ImageGenProvider for StableDiffusionProvider {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn provider_type(&self) -> String {
        "stable_diffusion".to_string()
    }

    async fn is_available(&self) -> bool {
        // Try pinging the API
        let url = format!("{}/sdapi/v1/progress", self.base_url.trim_end_matches('/'));
        self.client.get(&url).send().await.is_ok()
    }

    async fn generate(&self, params: ImageGenParams) -> Result<ImageGenResponse, ImageGenError> {
        let url = format!("{}/sdapi/v1/txt2img", self.base_url.trim_end_matches('/'));

        // Parse size string "1024x1024" -> (1024, 1024)
        let (width, height) = parse_size(&params.size).unwrap_or((512, 512));

        let body = SdTxt2ImgRequest {
            prompt: params.prompt,
            negative_prompt: params.negative_prompt.unwrap_or_default(),
            seed: -1,
            styles: vec![],
            width,
            height,
            steps: 25,      // Reasonable default
            cfg_scale: 7.0, // Reasonable default
            sampler_name: Some("Euler a".to_string()),
            batch_size: 1,
        };

        let body_json = serde_json::to_string(&body).unwrap_or_default();
        println!("[ImageGen] Sending SD request: {}", body_json);

        let res = self
            .client
            .post(&url)
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
                "SD WebUI API Error {}: {}",
                status, text
            )));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| ImageGenError::GenerationFailed(format!("Invalid JSON: {}", e)))?;

        // Response format: { "images": [ "base64..." ], "parameters": { ... }, "info": "..." }
        if let Some(images) = json.get("images").and_then(|v| v.as_array()) {
            if let Some(first) = images.first().and_then(|v| v.as_str()) {
                // Decode base64
                let bytes = general_purpose::STANDARD.decode(first).map_err(|e| {
                    ImageGenError::GenerationFailed(format!("Base64 decode error: {}", e))
                })?;

                return Ok(ImageGenResponse {
                    format: "png".to_string(), // SD WebUI usually returns PNG
                    data: bytes,
                });
            }
        }

        Err(ImageGenError::GenerationFailed(
            "Response missing 'images[0]'".to_string(),
        ))
    }
}

fn parse_size(size_str: &Option<String>) -> Option<(u32, u32)> {
    if let Some(s) = size_str {
        let parts: Vec<&str> = s.split('x').collect();
        if parts.len() == 2 {
            if let (Ok(w), Ok(h)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return Some((w, h));
            }
        }
    }
    None
}
