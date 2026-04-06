use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use edge_tts_rust::{Boundary, EdgeTtsClient, SpeakOptions, SynthesisEvent, Voice};
use futures::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;

const DEFAULT_EDGE_VOICE: &str = "zh-CN-XiaoyiNeural";

pub struct EdgeTtsProvider {
    client: EdgeTtsClient,
    provider_id: String,
    default_voice: String,
    voices: Vec<VoiceProfile>,
}

impl EdgeTtsProvider {
    pub async fn from_config(config: &ProviderConfig) -> Option<Self> {
        let client = match EdgeTtsClient::new() {
            Ok(client) => client,
            Err(err) => {
                tracing::error!(target: "tts", "[TTS] Failed to initialize edge-tts-rust client: {}", err);
                return None;
            }
        };

        let provider_id = config.id.clone();
        let default_voice = config
            .default_voice
            .clone()
            .unwrap_or_else(|| DEFAULT_EDGE_VOICE.to_string());

        let voices = match client.list_voices().await {
            Ok(voices) => {
                let mapped = voices
                    .into_iter()
                    .map(|voice| map_voice_profile(&provider_id, voice))
                    .collect::<Vec<_>>();
                if mapped.is_empty() {
                    vec![fallback_voice_profile(&provider_id, &default_voice)]
                } else {
                    mapped
                }
            }
            Err(err) => {
                tracing::error!(target: "tts", "[TTS] Failed to fetch Edge TTS voices: {}", err);
                vec![fallback_voice_profile(&provider_id, &default_voice)]
            }
        };

        Some(Self {
            client,
            provider_id,
            default_voice,
            voices,
        })
    }

    fn normalize_voice_id(&self, raw_voice: &str) -> String {
        normalize_voice_id(&self.provider_id, raw_voice)
    }

    fn build_options(&self, params: &TtsParams) -> SpeakOptions {
        let voice = params
            .voice
            .as_deref()
            .map(|voice| self.normalize_voice_id(voice))
            .unwrap_or_else(|| self.default_voice.clone());

        SpeakOptions {
            voice,
            rate: speed_to_rate(params.speed),
            volume: "+0%".to_string(),
            pitch: pitch_to_hz(params.pitch),
            boundary: Boundary::Sentence,
        }
    }
}

#[async_trait]
impl TtsProvider for EdgeTtsProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: true,
            supports_emotions: false,
            supports_speed: true,
            supports_pitch: true,
            supports_cloning: false,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        self.voices.clone()
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let options = self.build_options(&params);
        let result = self
            .client
            .synthesize(text, options)
            .await
            .map_err(map_edge_error)?;
        Ok(result.audio)
    }

    async fn synthesize_stream(
        &self,
        text: &str,
        params: TtsParams,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<Vec<u8>, TtsError>> + Send>>, TtsError>
    {
        let options = self.build_options(&params);
        let stream = self
            .client
            .stream(text.to_string(), options)
            .await
            .map_err(map_edge_error)?;

        let audio_stream = stream.filter_map(|event| async move {
            match event {
                Ok(SynthesisEvent::Audio(chunk)) => Some(Ok(chunk.to_vec())),
                Ok(SynthesisEvent::Boundary(_)) => None,
                Err(err) => Some(Err(map_edge_error(err))),
            }
        });

        Ok(Box::pin(audio_stream))
    }
}

fn map_voice_profile(provider_id: &str, voice: Voice) -> VoiceProfile {
    let mut extra_params = HashMap::new();
    if let Some(codec) = voice.suggested_codec.clone() {
        extra_params.insert("suggested_codec".to_string(), codec);
    }
    if let Some(status) = voice.status.clone() {
        extra_params.insert("status".to_string(), status);
    }
    if !voice.voice_tag.voice_personalities.is_empty() {
        extra_params.insert(
            "personalities".to_string(),
            voice.voice_tag.voice_personalities.join(", "),
        );
    }
    if !voice.voice_tag.content_categories.is_empty() {
        extra_params.insert(
            "content_categories".to_string(),
            voice.voice_tag.content_categories.join(", "),
        );
    }

    let short_name = voice.short_name;

    VoiceProfile {
        voice_id: format!("{}_{}", provider_id, short_name),
        name: voice
            .friendly_name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| short_name.clone()),
        gender: parse_gender(&voice.gender),
        language: voice.locale,
        engine: TtsEngine::Cloud,
        provider_id: provider_id.to_string(),
        extra_params,
    }
}

fn fallback_voice_profile(provider_id: &str, default_voice: &str) -> VoiceProfile {
    VoiceProfile {
        voice_id: format!("{}_{}", provider_id, default_voice),
        name: default_voice.to_string(),
        gender: Gender::Neutral,
        language: "en-US".to_string(),
        engine: TtsEngine::Cloud,
        provider_id: provider_id.to_string(),
        extra_params: HashMap::new(),
    }
}

fn parse_gender(gender: &str) -> Gender {
    match gender.to_ascii_lowercase().as_str() {
        "male" => Gender::Male,
        "female" => Gender::Female,
        _ => Gender::Neutral,
    }
}

fn speed_to_rate(speed: Option<f32>) -> String {
    let delta = ((speed.unwrap_or(1.0).clamp(0.1, 3.0) - 1.0) * 100.0).round() as i32;
    format!("{delta:+}%")
}

fn pitch_to_hz(pitch: Option<f32>) -> String {
    let delta = ((pitch.unwrap_or(1.0).clamp(0.1, 3.0) - 1.0) * 100.0).round() as i32;
    format!("{delta:+}Hz")
}

fn map_edge_error(err: edge_tts_rust::Error) -> TtsError {
    TtsError::SynthesisFailed(format!("edge-tts-rust error: {}", err))
}

fn normalize_voice_id(provider_id: &str, raw_voice: &str) -> String {
    raw_voice
        .strip_prefix(&format!("{}_", provider_id))
        .unwrap_or(raw_voice)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_is_converted_to_signed_percent() {
        assert_eq!(speed_to_rate(Some(1.5)), "+50%");
        assert_eq!(speed_to_rate(Some(0.5)), "-50%");
    }

    #[test]
    fn pitch_is_converted_to_signed_hz() {
        assert_eq!(pitch_to_hz(Some(1.2)), "+20Hz");
        assert_eq!(pitch_to_hz(Some(0.8)), "-20Hz");
    }

    #[test]
    fn provider_prefix_is_removed_before_synthesis() {
        assert_eq!(
            normalize_voice_id("edge_tts", "edge_tts_zh-CN-XiaoyiNeural"),
            "zh-CN-XiaoyiNeural"
        );
    }
}
