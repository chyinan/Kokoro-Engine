use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;

/// Browser TTS provider — delegates actual synthesis to the frontend's
/// `window.speechSynthesis` API via a sentinel error.
///
/// The Rust backend cannot call Web APIs directly, so this provider returns
/// `TtsError::BrowserDelegate` to signal the frontend service to handle it.
pub struct BrowserTTSProvider;

impl BrowserTTSProvider {
    pub fn new() -> Self {
        Self
    }

    pub fn from_config(_config: &ProviderConfig) -> Option<Self> {
        Some(Self::new())
    }
}

#[async_trait]
impl TtsProvider for BrowserTTSProvider {
    fn id(&self) -> String {
        "browser".to_string()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: false,
            supports_speed: true,
            supports_pitch: true,
            supports_cloning: false,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        // Browser voices are dynamic and enumerated on the frontend.
        // We expose a generic entry so the system knows this provider exists.
        vec![VoiceProfile {
            voice_id: "browser_default".to_string(),
            name: "System Default".to_string(),
            gender: Gender::Neutral,
            language: "en".to_string(),
            engine: TtsEngine::Native,
            provider_id: "browser".to_string(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        true // Always available in a browser context
    }

    async fn synthesize(&self, _text: &str, _params: TtsParams) -> Result<Vec<u8>, TtsError> {
        // Signal the frontend to handle this via window.speechSynthesis
        Err(TtsError::BrowserDelegate)
    }
}
