pub mod azure;
pub mod browser;
pub mod cache;
pub mod cloud_base;
pub mod config;
pub mod edge;
pub mod interface;
pub mod local_gpt_sovits;
pub mod local_vits;
pub mod manager;
pub mod omnivoice;
pub mod openai;
pub mod queue;
pub mod router;
pub mod voice_registry;

pub use config::{load_config, TtsSystemConfig};
pub use interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
pub use manager::{ProviderStatus, TtsService};
