use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

/// Local VITS provider — sends HTTP requests to a local VITS inference server.
///
/// Compatible with common VITS HTTP APIs (e.g., vits-simple-api, MoeGoe-Server).
/// The server must expose:
///   POST /synthesize  — accepts JSON, returns audio bytes
///   GET  /health      — returns 200 if server is ready
pub struct LocalVITSProvider {
    client: Client,
    endpoint: String,
    model_id: Option<String>,
    provider_id: String,
}

#[derive(Serialize)]
struct VitsSynthRequest {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emotion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
}

impl LocalVITSProvider {
    pub fn new(endpoint: String, model_id: Option<String>) -> Self {
        Self {
            client: Client::new(),
            endpoint,
            model_id,
            provider_id: "local_vits".to_string(),
        }
    }

    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let endpoint = config
            .endpoint
            .clone()
            .or(config.base_url.clone())
            .unwrap_or_else(|| "http://localhost:5000".to_string());
        Some(Self {
            client: Client::new(),
            endpoint,
            model_id: config.model.clone(),
            provider_id: config.id.clone(),
        })
    }
}

#[async_trait]
impl TtsProvider for LocalVITSProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: true,
            supports_speed: true,
            supports_pitch: false,
            supports_cloning: false,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        // VITS voices depend on the loaded model. Return a placeholder.
        // In production, query the server's /speakers endpoint.
        vec![VoiceProfile {
            voice_id: format!("{}_default", self.provider_id),
            name: "VITS Default".to_string(),
            gender: Gender::Neutral,
            language: "ja".to_string(),
            engine: TtsEngine::Vits,
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/health", self.endpoint);
        match self.client.get(&url).timeout(std::time::Duration::from_secs(3)).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let url = format!("{}/synthesize", self.endpoint);
        let body = VitsSynthRequest {
            text: text.to_string(),
            speaker_id: params.voice,
            speed: params.speed,
            emotion: params.emotion,
            model_id: self.model_id.clone(),
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("VITS request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "VITS server error: {}",
                error_text
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("VITS bytes error: {}", e)))?;
        Ok(bytes.to_vec())
    }
}
