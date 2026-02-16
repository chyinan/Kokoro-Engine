use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

/// Local GPT-SoVITS provider — sends HTTP requests to a local GPT-SoVITS inference server.
///
/// Compatible with RVC-Boss/GPT-SoVITS `api_v2.py`.
/// Endpoints used:
///   POST /tts               — synthesis
///   GET  /set_gpt_weights   — switch GPT model
///   GET  /set_sovits_weights — switch SoVITS model
pub struct LocalGPTSoVITSProvider {
    client: Client,
    endpoint: String,
    provider_id: String,
    base_url: String,
    // GPT-SoVITS-specific defaults (from provider config `extra`)
    default_ref_audio: Option<String>,
    default_prompt_text: Option<String>,
    default_prompt_lang: Option<String>,
    default_text_lang: String,
    // Model weight paths
    gpt_weights: Option<String>,
    sovits_weights: Option<String>,
}

#[derive(Serialize)]
struct GPTSoVITSRequest {
    text: String,
    text_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_audio_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_split_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    speed_factor: f32,
}

impl LocalGPTSoVITSProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            endpoint: format!("{}/tts", base_url.trim_end_matches('/')),
            provider_id: "gpt_sovits".to_string(),
            base_url,
            default_ref_audio: None,
            default_prompt_text: None,
            default_prompt_lang: None,
            default_text_lang: "zh".to_string(),
            gpt_weights: None,
            sovits_weights: None,
        }
    }

    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let base_url = config
            .base_url
            .clone()
            .or(config.endpoint.clone())
            .unwrap_or_else(|| "http://127.0.0.1:9880".to_string());

        let default_ref_audio = config
            .extra
            .get("ref_audio_path")
            .and_then(|v| v.as_str())
            .map(String::from);

        let default_prompt_text = config
            .extra
            .get("prompt_text")
            .and_then(|v| v.as_str())
            .map(String::from);

        let default_prompt_lang = config
            .extra
            .get("prompt_lang")
            .and_then(|v| v.as_str())
            .map(String::from);

        let default_text_lang = config
            .extra
            .get("text_lang")
            .and_then(|v| v.as_str())
            .unwrap_or("zh")
            .to_string();

        let gpt_weights = config
            .extra
            .get("gpt_weights")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let sovits_weights = config
            .extra
            .get("sovits_weights")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        Some(Self {
            client: Client::new(),
            endpoint: format!("{}/tts", base_url.trim_end_matches('/')),
            provider_id: config.id.clone(),
            base_url,
            default_ref_audio,
            default_prompt_text,
            default_prompt_lang,
            default_text_lang,
            gpt_weights,
            sovits_weights,
        })
    }
}

#[async_trait]
impl TtsProvider for LocalGPTSoVITSProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: false, // GPT-SoVITS uses ref audio for emotion, not explicit param yet
            supports_speed: true,
            supports_pitch: false,
            supports_cloning: true, // It's literally a voice cloning model
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        // GPT-SoVITS doesn't have a fixed list of voices in the same way.
        // It uses reference audio. We return a generic voice.
        vec![VoiceProfile {
            voice_id: "gpt_sovits_default".to_string(),
            name: "GPT-SoVITS Default".to_string(),
            gender: Gender::Neutral,
            language: "auto".to_string(),
            engine: TtsEngine::Vits, // It's generic/VITS-like
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        // Ping /tts endpoint — a running api_v2.py will respond (even 400 for missing params).
        // Any HTTP response means the server is reachable.
        let url = format!("{}/tts", self.base_url.trim_end_matches('/'));
        match self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        // Switch models if configured (server handles idempotent load).
        let base = self.base_url.trim_end_matches('/');
        if let Some(gpt) = &self.gpt_weights {
            let url = format!("{}/set_gpt_weights?weights_path={}", base, gpt);
            if let Err(e) = self
                .client
                .get(&url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                eprintln!("[GPT-SoVITS] Failed to set GPT weights: {}", e);
            }
        }
        if let Some(sovits) = &self.sovits_weights {
            let url = format!("{}/set_sovits_weights?weights_path={}", base, sovits);
            if let Err(e) = self
                .client
                .get(&url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                eprintln!("[GPT-SoVITS] Failed to set SoVITS weights: {}", e);
            }
        }

        // Use per-request extra_params if provided, otherwise fall back to provider defaults.
        let ref_audio_path = params
            .extra_params
            .as_ref()
            .and_then(|p| p.get("ref_audio_path"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.default_ref_audio.clone());

        let prompt_text = params
            .extra_params
            .as_ref()
            .and_then(|p| p.get("prompt_text"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.default_prompt_text.clone());

        let prompt_lang = params
            .extra_params
            .as_ref()
            .and_then(|p| p.get("prompt_lang"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.default_prompt_lang.clone());

        let text_lang = params
            .extra_params
            .as_ref()
            .and_then(|p| p.get("text_lang"))
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_text_lang)
            .to_string();

        let text_split_method = params
            .extra_params
            .as_ref()
            .and_then(|p| p.get("text_split_method"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let body = GPTSoVITSRequest {
            text: text.to_string(),
            text_lang,
            ref_audio_path,
            prompt_text,
            prompt_lang,
            text_split_method,
            top_k: None,
            top_p: None,
            temperature: None,
            speed_factor: params.speed.unwrap_or(1.0),
        };

        let response = self
            .client
            .post(&self.endpoint)
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("GPT-SoVITS request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TtsError::SynthesisFailed(format!(
                "GPT-SoVITS server error: {}",
                error_text
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("GPT-SoVITS bytes error: {}", e)))?;
        Ok(bytes.to_vec())
    }
}
