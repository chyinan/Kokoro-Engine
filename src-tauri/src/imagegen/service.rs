use super::config::{ImageGenProviderConfig, ImageGenSystemConfig};
use super::interface::{ImageGenError, ImageGenParams, ImageGenProvider};
use super::google::GoogleImageGenProvider;
use super::openai::OpenAIImageGenProvider;
use super::stable_diffusion::StableDiffusionProvider;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Serialize)]
pub struct ImageGenResult {
    pub image_url: String, // file:// path
    pub prompt: String,
    pub provider_id: String,
}

#[derive(Clone)]
pub struct ImageGenService {
    providers: Arc<RwLock<HashMap<String, Box<dyn ImageGenProvider>>>>,
    default_provider: Arc<RwLock<Option<String>>>,
    output_dir: PathBuf,
}

impl ImageGenService {
    pub async fn init_from_config(config: &ImageGenSystemConfig) -> Self {
        // Determine output directory
        let app_data = dirs_next::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.chyin.kokoro");
        let output_dir = app_data.join("generated_images");

        if let Err(e) = fs::create_dir_all(&output_dir) {
            eprintln!("[ImageGen] Failed to create output directory: {}", e);
        }

        let service = Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            default_provider: Arc::new(RwLock::new(config.default_provider.clone())),
            output_dir,
        };

        if !config.enabled {
            println!("[ImageGen] Service is disabled in config");
            return service;
        }

        for provider_config in &config.providers {
            if !provider_config.enabled {
                continue;
            }

            match Self::build_provider(provider_config) {
                Some(provider) => {
                    println!("[ImageGen] Registering provider: {}", provider.id());
                    service.register_provider(provider).await;
                }
                None => {
                    eprintln!(
                        "[ImageGen] Failed to build provider '{}' (type: {})",
                        provider_config.id, provider_config.provider_type
                    );
                }
            }
        }

        service
    }

    fn build_provider(config: &ImageGenProviderConfig) -> Option<Box<dyn ImageGenProvider>> {
        match config.provider_type.as_str() {
            "openai" => {
                let api_key = config.resolve_api_key()?;
                Some(Box::new(OpenAIImageGenProvider::new(
                    config.id.clone(),
                    api_key,
                    config.base_url.clone(),
                    config.model.clone(),
                )))
            }
            "stable_diffusion" => Some(Box::new(StableDiffusionProvider::new(
                config.id.clone(),
                config.base_url.clone(),
                config.model.clone(),
            ))),
            "google" => Some(Box::new(GoogleImageGenProvider::new(config).ok()?)),
            other => {
                eprintln!("[ImageGen] Unknown provider type: {}", other);
                None
            }
        }
    }

    pub async fn register_provider(&self, provider: Box<dyn ImageGenProvider>) {
        let id = provider.id();
        let mut providers = self.providers.write().await;
        providers.insert(id, provider);
    }

    pub async fn generate(
        &self,
        prompt: String,
        provider_id: Option<String>,
        params: Option<ImageGenParams>,
    ) -> Result<ImageGenResult, ImageGenError> {
        let providers = self.providers.read().await;
        
        let target_id = if let Some(id) = provider_id {
            id
        } else {
            let default = self.default_provider.read().await;
            default.clone().ok_or(ImageGenError::ConfigError("No default provider configured".to_string()))?
        };

        let provider = providers.get(&target_id).ok_or(ImageGenError::ProviderNotFound(target_id.clone()))?;

        if !provider.is_available().await {
            return Err(ImageGenError::Unavailable(format!("Provider {} is not available", target_id)));
        }

        let mut gen_params = params.unwrap_or_default();
        if gen_params.prompt.is_empty() {
            gen_params.prompt = prompt.clone();
        }

        println!("[ImageGen] Generating with provider '{}': {}", target_id, prompt);

        let response = provider.generate(gen_params).await?;

        // Save image to disk
        let filename = format!(
            "{}_{}.{}",
            chrono::Utc::now().format("%Y%m%d_%H%M%S"),
            uuid::Uuid::new_v4(),
            response.format
        );
        let path = self.output_dir.join(&filename);

        fs::write(&path, &response.data).map_err(|e| {
            ImageGenError::GenerationFailed(format!("Failed to save image: {}", e))
        })?;

        // Construct file URL
        // In Tauri v2, we can't easily guess the "asset protocol" URL perfectly without knowing the scope,
        // but typically "file://" works if scope allows, or we use the custom protocol.
        // For now, let's return the absolute path, and frontend can convert it if needed, 
        // OR we return a `asset://` compatible URL?
        // Actually `BackgroundLayer` likely expects a browser-compatible URL.
        // For local files in Tauri, we usually need the `tauri-plugin-fs` or `convertFileSrc`.
        // Ideally we return the absolute path, and the frontend helper utilizes `convertFileSrc`.
        
        let abs_path = path.to_string_lossy().to_string();

        Ok(ImageGenResult {
            image_url: abs_path,
            prompt,
            provider_id: target_id,
        })
    }
    
    pub async fn list_providers(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        providers.keys().cloned().collect()
    }
    
    pub async fn reload_from_config(&self, config: &ImageGenSystemConfig) {
        let mut providers = self.providers.write().await;
        providers.clear();
        
        let mut default = self.default_provider.write().await;
        *default = config.default_provider.clone();
        
        if config.enabled {
             for provider_config in &config.providers {
                if !provider_config.enabled { continue; }
                if let Some(provider) = Self::build_provider(provider_config) {
                     providers.insert(provider.id(), provider);
                }
             }
        }
        println!("[ImageGen] Reloaded {} providers", providers.len());
    }
}
