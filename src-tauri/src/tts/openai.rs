use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsAudioChunk, TtsAudioStream, TtsEngine, TtsError, TtsParams,
    TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Deserialize)]
struct OpenAIModelList {
    #[serde(default)]
    data: Vec<OpenAIModel>,
}

#[derive(Deserialize)]
struct OpenAIModel {
    id: String,
}

/// Query the models available to an OpenAI speech provider's API key.
pub async fn list_openai_tts_models(config: &ProviderConfig) -> Result<Vec<String>, TtsError> {
    let api_key = config
        .resolve_api_key()
        .filter(|api_key| !api_key.trim().is_empty())
        .ok_or_else(|| {
            TtsError::ConfigError(
                "OpenAI API key is missing. Add a key or configure OPENAI_API_KEY.".to_string(),
            )
        })?;

    if !matches!(config.provider_type.as_str(), "openai" | "openai_realtime") {
        return Err(TtsError::ConfigError(
            "model listing is only supported for OpenAI speech providers".to_string(),
        ));
    }

    let configured_base_url = if config.provider_type == "openai_realtime" {
        config.base_url.as_deref().or(config.endpoint.as_deref())
    } else {
        config.base_url.as_deref()
    };
    let base_url = configured_base_url
        .filter(|base_url| !base_url.trim().is_empty())
        .unwrap_or(DEFAULT_OPENAI_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{base_url}/models");
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| {
            TtsError::Unavailable(format!("failed to build OpenAI client: {error}"))
        })?;
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| TtsError::Unavailable(format!("failed to list OpenAI models: {error}")))?;

    if !response.status().is_success() {
        return Err(TtsError::Unavailable(format!(
            "OpenAI model list request failed with HTTP {}",
            response.status()
        )));
    }

    let model_list = response
        .json::<OpenAIModelList>()
        .await
        .map_err(|error| TtsError::Unavailable(format!("invalid OpenAI model list: {error}")))?;

    let mut models = model_list
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|model_id| match config.provider_type.as_str() {
            "openai" => {
                let model_id = model_id.to_ascii_lowercase();
                model_id.starts_with("tts-") || model_id.contains("-tts")
            }
            "openai_realtime" => {
                let model_id = model_id.to_ascii_lowercase();
                model_id.contains("realtime")
                    && !model_id.contains("transcri")
                    && !model_id.contains("translat")
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    models.sort_unstable();
    models.dedup();
    Ok(models)
}

#[derive(Serialize, Clone)]
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
    provider_id: String,
    api_key: String,
    base_url: String,
    model: String,
    default_voice: String,
}

impl OpenAITtsProvider {
    pub fn new(
        provider_id: String,
        api_key: String,
        base_url: Option<String>,
        model: Option<String>,
        voice: Option<String>,
    ) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        Ok(Self {
            client,
            provider_id,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "tts-1".to_string()),
            default_voice: voice.unwrap_or_else(|| "alloy".to_string()),
        })
    }

    /// Construct from a ProviderConfig entry.
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let api_key = config.resolve_api_key()?;
        Self::new(
            config.id.clone(),
            api_key,
            config.base_url.clone(),
            config.model.clone(),
            config.default_voice.clone(),
        )
        .ok()
    }

    fn normalize_voice_id(&self, raw_voice: &str) -> String {
        raw_voice
            .strip_prefix(&format!("{}_", self.provider_id))
            .unwrap_or(raw_voice)
            .to_string()
    }
}

#[async_trait]
impl TtsProvider for OpenAITtsProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
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
        let voices = vec![
            ("alloy", Gender::Neutral),
            ("ash", Gender::Neutral),
            ("ballad", Gender::Neutral),
            ("coral", Gender::Neutral),
            ("echo", Gender::Male),
            ("fable", Gender::Male),
            ("onyx", Gender::Male),
            ("nova", Gender::Female),
            ("sage", Gender::Neutral),
            ("shimmer", Gender::Female),
            ("verse", Gender::Neutral),
            ("marin", Gender::Neutral),
            ("cedar", Gender::Neutral),
        ];
        let is_legacy_model = matches!(self.model.as_str(), "tts-1" | "tts-1-hd");
        let legacy_voices = [
            "alloy", "ash", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer",
        ];

        voices
            .into_iter()
            .filter(|(name, _)| !is_legacy_model || legacy_voices.contains(name))
            .map(|(name, gender)| VoiceProfile {
                voice_id: format!("{}_{}", self.provider_id, name),
                name: name.to_string(),
                gender,
                language: "en".to_string(),
                engine: TtsEngine::Cloud,
                provider_id: self.provider_id.clone(),
                extra_params: Default::default(),
            })
            .collect()
    }

    fn cache_key_salt(&self) -> Option<String> {
        Some(format!("base_url={};model={}", self.base_url, self.model))
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let url = format!("{}/audio/speech", self.base_url);
        let raw_voice = params.voice.unwrap_or_else(|| self.default_voice.clone());
        let request_body = TtsRequest {
            model: self.model.clone(),
            input: text.to_string(),
            voice: self.normalize_voice_id(&raw_voice),
            response_format: "mp3".to_string(),
            speed: params.speed,
        };

        let client = self.client.clone();
        let url_clone = url.clone();
        let api_key = self.api_key.clone();
        let body = request_body.clone();

        let response = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_clone.clone();
                let body = body.clone();
                let api_key = api_key.clone();
                async move {
                    client
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", api_key))
                        .header("Content-Type", "application/json")
                        .json(&body)
                        .send()
                        .await
                }
            },
            2,
        )
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

    async fn synthesize_stream(
        &self,
        text: &str,
        params: TtsParams,
    ) -> Result<TtsAudioStream, TtsError> {
        let url = format!("{}/audio/speech", self.base_url);
        let raw_voice = params.voice.unwrap_or_else(|| self.default_voice.clone());
        let request_body = TtsRequest {
            model: self.model.clone(),
            input: text.to_string(),
            voice: self.normalize_voice_id(&raw_voice),
            response_format: "mp3".to_string(),
            speed: params.speed,
        };

        let client = self.client.clone();
        let url_clone = url.clone();
        let api_key = self.api_key.clone();
        let body = request_body.clone();

        let response = crate::utils::http::request_with_retry(
            move || {
                let client = client.clone();
                let url = url_clone.clone();
                let body = body.clone();
                let api_key = api_key.clone();
                async move {
                    client
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", api_key))
                        .header("Content-Type", "application/json")
                        .json(&body)
                        .send()
                        .await
                }
            },
            2,
        )
        .await
        .map_err(|e| TtsError::SynthesisFailed(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "OpenAI API error: {}",
                error_text
            )));
        }

        use futures::StreamExt;
        let stream = response.bytes_stream().map(|chunk_res| match chunk_res {
            Ok(bytes) => Ok(TtsAudioChunk {
                data: bytes.to_vec(),
                format: None,
                response_id: None,
            }),
            Err(e) => Err(TtsError::SynthesisFailed(format!("Stream error: {}", e))),
        });

        Ok(Box::pin(stream))
    }
}
