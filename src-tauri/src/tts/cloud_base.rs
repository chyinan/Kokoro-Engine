use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;

/// Generic cloud TTS provider for bearer/header-key REST APIs.
///
/// Create instances via `CloudTTSProvider::elevenlabs_style()`.
pub struct CloudTTSProvider {
    client: Client,
    provider_id: String,
    api_key: String,
    base_url: String,
    auth_style: AuthStyle,
    default_voice: String,
    model: Option<String>,
    capabilities: ProviderCapabilities,
}

#[allow(dead_code)]
#[derive(Clone)]
enum AuthStyle {
    Bearer,               // Authorization: Bearer <key>
    CustomHeader(String), // Arbitrary header name
}

#[derive(Serialize)]
struct GenericSynthRequest {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

impl CloudTTSProvider {
    /// Create an ElevenLabs-style TTS provider.
    pub fn elevenlabs_style(config: &ProviderConfig) -> Option<Self> {
        let api_key = config.resolve_api_key()?;
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                tracing::error!(
                    target: "tts",
                    "Failed to build HTTP client for provider '{}': {}",
                    config.id,
                    e
                );
            })
            .ok()?;

        Some(Self {
            client,
            provider_id: config.id.clone(),
            api_key,
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.elevenlabs.io/v1".to_string()),
            auth_style: AuthStyle::CustomHeader("xi-api-key".to_string()),
            default_voice: config
                .default_voice
                .clone()
                .unwrap_or_else(|| "21m00Tcm4TlvDq8ikWAM".to_string()),
            model: config.model.clone(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_emotions: true,
                supports_speed: true,
                supports_pitch: false,
                supports_cloning: true,
                supports_ssml: false,
            },
        })
    }

    fn build_auth_header(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_style {
            AuthStyle::Bearer => req.header("Authorization", format!("Bearer {}", self.api_key)),
            AuthStyle::CustomHeader(header_name) => req.header(header_name, &self.api_key),
        }
    }
}

#[async_trait]
impl TtsProvider for CloudTTSProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        // Cloud providers have many voices. Return a representative subset.
        // In production, call the provider's /voices endpoint.
        vec![VoiceProfile {
            voice_id: format!("{}_default", self.provider_id),
            name: format!("{} Default", self.provider_id),
            gender: Gender::Neutral,
            language: "en".to_string(),
            engine: TtsEngine::Cloud,
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        // ElevenLabs-style: POST /text-to-speech/{voice_id}
        // Generic JSON approach for header-key/bearer REST APIs
        let voice = params.voice.unwrap_or_else(|| self.default_voice.clone());
        let url = format!("{}/text-to-speech/{}", self.base_url, voice);

        let body = GenericSynthRequest {
            text: text.to_string(),
            voice: Some(voice),
            model: self.model.clone(),
            speed: params.speed,
            extra: HashMap::new(),
        };

        let request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json");
        let request = self.build_auth_header(request);

        let response = request
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| {
                TtsError::SynthesisFailed(format!("{} request failed: {}", self.provider_id, e))
            })?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "{} API error: {}",
                self.provider_id, error_text
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            TtsError::SynthesisFailed(format!("{} bytes error: {}", self.provider_id, e))
        })?;
        Ok(bytes.to_vec())
    }
}
