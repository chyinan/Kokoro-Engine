use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use super::config::ProviderConfig;
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

#[derive(Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
}

pub struct OpenAITtsProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    default_voice: String,
}

impl OpenAITtsProvider {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        model: Option<String>,
        voice: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "tts-1".to_string()),
            default_voice: voice.unwrap_or_else(|| "alloy".to_string()),
        }
    }

    /// Construct from a ProviderConfig entry.
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let api_key = config.resolve_api_key()?;
        Some(Self::new(
            api_key,
            config.base_url.clone(),
            config.model.clone(),
            config.default_voice.clone(),
        ))
    }
}

#[async_trait]
impl TtsProvider for OpenAITtsProvider {
    fn id(&self) -> String {
        "openai".to_string()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: false,
            supports_speed: true,
            supports_pitch: false,
            supports_cloning: false,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        // OpenAI's built-in voices
        let voices = vec![
            ("alloy", Gender::Neutral),
            ("echo", Gender::Male),
            ("fable", Gender::Male),
            ("onyx", Gender::Male),
            ("nova", Gender::Female),
            ("shimmer", Gender::Female),
        ];

        voices
            .into_iter()
            .map(|(name, gender)| VoiceProfile {
                voice_id: format!("openai_{}", name),
                name: name.to_string(),
                gender,
                language: "en".to_string(),
                engine: TtsEngine::Cloud,
                provider_id: "openai".to_string(),
                extra_params: Default::default(),
            })
            .collect()
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let url = format!("{}/audio/speech", self.base_url);
        let request_body = TtsRequest {
            model: self.model.clone(),
            input: text.to_string(),
            voice: params.voice.unwrap_or_else(|| self.default_voice.clone()),
            response_format: "mp3".to_string(),
            speed: params.speed,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "OpenAI API error: {}",
                error_text
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("Bytes error: {}", e)))?;
        Ok(bytes.to_vec())
    }
}
