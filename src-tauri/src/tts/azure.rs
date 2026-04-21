use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;

pub struct AzureTtsProvider {
    client: Client,
    provider_id: String,
    api_key: String,
    endpoint: String,
    default_voice: String,
}

impl AzureTtsProvider {
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let api_key = config.resolve_api_key()?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            tracing::error!(target: "tts", "Azure provider '{}' missing API key", config.id);
            return None;
        }

        let endpoint = config.base_url.clone().unwrap_or_else(|| {
            "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1".to_string()
        });
        let endpoint = endpoint.trim().to_string();
        if endpoint.is_empty() {
            tracing::error!(target: "tts", "Azure provider '{}' missing endpoint", config.id);
            return None;
        }
        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
            tracing::error!(
                target: "tts",
                "Azure provider '{}' endpoint must start with http:// or https://: {}",
                config.id,
                endpoint
            );
            return None;
        }

        let endpoint = endpoint.trim_end_matches('/').to_string();

        let endpoint = if endpoint.ends_with("/cognitiveservices/v1") {
            endpoint
        } else {
            format!("{}/cognitiveservices/v1", endpoint)
        };

        tracing::info!(
            target: "tts",
            "Azure provider '{}' initialized with endpoint {}",
            config.id,
            endpoint
        );

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                tracing::error!(
                    target: "tts",
                    "Failed to build HTTP client for Azure provider '{}': {}",
                    config.id,
                    e
                );
            })
            .ok()?;

        Some(Self {
            client,
            provider_id: config.id.clone(),
            api_key,
            endpoint,
            default_voice: config
                .default_voice
                .clone()
                .unwrap_or_else(|| "en-US-JennyNeural".to_string()),
        })
    }
}

fn escape_ssml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[async_trait]
impl TtsProvider for AzureTtsProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: true,
            supports_speed: false,
            supports_pitch: false,
            supports_cloning: false,
            supports_ssml: true,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        vec![VoiceProfile {
            voice_id: format!("{}_{}", self.provider_id, self.default_voice),
            name: self.default_voice.clone(),
            gender: Gender::Neutral,
            language: "en-US".to_string(),
            engine: TtsEngine::Cloud,
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty() && !self.endpoint.is_empty()
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let voice = params.voice.unwrap_or_else(|| self.default_voice.clone());
        let voice_name = voice
            .strip_prefix(&format!("{}_", self.provider_id))
            .unwrap_or(&voice)
            .to_string();

        let escaped_text = escape_ssml_text(text);
        let ssml = format!(
            "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"en-US\"><voice name=\"{}\">{}</voice></speak>",
            voice_name, escaped_text
        );

        let response = self
            .client
            .post(&self.endpoint)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("Content-Type", "application/ssml+xml")
            .header(
                "X-Microsoft-OutputFormat",
                "audio-24khz-96kbitrate-mono-mp3",
            )
            .header("User-Agent", "kokoro-engine")
            .body(ssml)
            .send()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("azure request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "azure api error: {}",
                error_text
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("azure bytes error: {}", e)))?;
        Ok(bytes.to_vec())
    }
}
