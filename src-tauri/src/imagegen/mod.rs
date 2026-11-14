pub mod config;
pub mod google;
pub mod interface;
pub mod openai;
pub mod service;
pub mod stable_diffusion;

pub use config::{ImageGenProviderConfig, ImageGenSystemConfig};
pub use interface::{ImageGenError, ImageGenParams, ImageGenProvider, ImageGenResponse};
pub use service::{ImageGenResult, ImageGenService};

#[derive(Clone)]
pub struct WindowSizeState {
    pub width: std::sync::Arc<tokio::sync::RwLock<u32>>,
    pub height: std::sync::Arc<tokio::sync::RwLock<u32>>,
}

impl Default for WindowSizeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowSizeState {
    pub fn new() -> Self {
        Self {
            width: std::sync::Arc::new(tokio::sync::RwLock::new(1200)),
            height: std::sync::Arc::new(tokio::sync::RwLock::new(800)),
        }
    }
    pub async fn get(&self) -> (u32, u32) {
        (*self.width.read().await, *self.height.read().await)
    }
    pub async fn set(&self, w: u32, h: u32) {
        *self.width.write().await = w;
        *self.height.write().await = h;
    }
}
