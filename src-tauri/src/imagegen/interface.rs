use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Error Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageGenError {
    ProviderNotFound(String),
    GenerationFailed(String),
    Timeout(String),
    ConfigError(String),
    Unavailable(String),
}

impl fmt::Display for ImageGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageGenError::ProviderNotFound(msg) => write!(f, "Provider not found: {}", msg),
            ImageGenError::GenerationFailed(msg) => write!(f, "Generation failed: {}", msg),
            ImageGenError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            ImageGenError::ConfigError(msg) => write!(f, "Config error: {}", msg),
            ImageGenError::Unavailable(msg) => write!(f, "Unavailable: {}", msg),
        }
    }
}

impl std::error::Error for ImageGenError {}

impl From<ImageGenError> for String {
    fn from(e: ImageGenError) -> String {
        e.to_string()
    }
}

// ── Generation Parameters ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenParams {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub size: Option<String>,    // e.g. "1024x1024"
    pub quality: Option<String>, // e.g. "standard", "hd"
    pub style: Option<String>,   // e.g. "vivid", "natural"
    pub n: usize,                // Number of images to generate (default 1)
}

impl Default for ImageGenParams {
    fn default() -> Self {
        Self {
            prompt: "".to_string(),
            negative_prompt: None,
            size: None,
            quality: None,
            style: None,
            n: 1,
        }
    }
}

// ── Provider Response ──────────────────────────────────

#[derive(Debug, Clone)]
pub struct ImageGenResponse {
    pub format: String, // "png", "jpg"
    pub data: Vec<u8>,  // Raw image bytes
}

// ── Provider Trait ──────────────────────────────────────

#[async_trait]
pub trait ImageGenProvider: Send + Sync {
    /// Unique identifier for this provider instance
    fn id(&self) -> String;

    /// Provider type (e.g., "openai", "stable_diffusion")
    fn provider_type(&self) -> String;

    /// Check if the provider is reachable/configured
    async fn is_available(&self) -> bool;

    /// Generate an image from the prompt
    async fn generate(&self, params: ImageGenParams) -> Result<ImageGenResponse, ImageGenError>;
}
