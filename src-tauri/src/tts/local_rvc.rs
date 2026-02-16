use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Local RVC (Retrieval-based Voice Conversion) provider.
///
/// RVC is a **voice conversion** pipeline (audio → audio), not a text-to-speech engine.
/// It converts source audio to sound like a target voice model.
///
/// This provider integrates with the **RVC WebUI Gradio server**:
///   - Gradio root (GET /)         — health check
///   - POST /upload                — upload audio file to temp dir
///   - POST /api/infer_convert     — run voice conversion (vc_single)
///   - POST /api/infer_refresh     — refresh model list
///
/// Default endpoint: http://localhost:7865
pub struct LocalRVCProvider {
    client: Client,
    endpoint: String,
    _default_model: Option<String>,
    provider_id: String,

    // RVC-specific defaults (most users never touch these)
    default_f0_method: String,
    default_index_path: String,
    default_index_rate: f32,
    default_filter_radius: i32,
    default_resample_sr: i32,
    default_rms_mix_rate: f32,
    default_protect: f32,
}

/// Parameters for singing voice conversion.
#[derive(Debug, Clone, Serialize)]
pub struct SingingConvertParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_shift: Option<f32>,
    /// Whether to separate vocals from the input first
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separate_vocals: Option<bool>,
    // Advanced params (optional overrides)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f0_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_rate: Option<f32>,
}

/// Info about an available RVC model on the server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RvcModelInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Gradio API call response.
#[derive(Debug, Deserialize)]
struct GradioApiResponse {
    data: Vec<serde_json::Value>,
}

impl LocalRVCProvider {
    pub fn new(endpoint: String, default_model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            endpoint,
            _default_model: default_model,
            provider_id: "local_rvc".to_string(),
            default_f0_method: "rmvpe".to_string(),
            default_index_path: String::new(),
            default_index_rate: 0.75,
            default_filter_radius: 3,
            default_resample_sr: 0,
            default_rms_mix_rate: 0.25,
            default_protect: 0.33,
        }
    }

    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let endpoint = config
            .endpoint
            .clone()
            .or(config.base_url.clone())
            .unwrap_or_else(|| "http://localhost:7865".to_string());

        let extra = &config.extra;

        Some(Self {
            client: Client::new(),
            endpoint,
            _default_model: config.model.clone(),
            provider_id: config.id.clone(),
            default_f0_method: extra
                .get("f0_method")
                .and_then(|v| v.as_str())
                .unwrap_or("rmvpe")
                .to_string(),
            default_index_path: extra
                .get("index_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            default_index_rate: extra
                .get("index_rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.75) as f32,
            default_filter_radius: extra
                .get("filter_radius")
                .and_then(|v| v.as_i64())
                .unwrap_or(3) as i32,
            default_resample_sr: extra
                .get("resample_sr")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            default_rms_mix_rate: extra
                .get("rms_mix_rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.25) as f32,
            default_protect: extra
                .get("protect")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.33) as f32,
        })
    }
}

#[async_trait]
impl TtsProvider for LocalRVCProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: false,
            supports_speed: false,
            supports_pitch: true,
            supports_cloning: true,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        vec![VoiceProfile {
            voice_id: format!("{}_default", self.provider_id),
            name: "RVC Default".to_string(),
            gender: Gender::Neutral,
            language: "any".to_string(),
            engine: TtsEngine::Rvc,
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        self.check_health().await
    }

    async fn synthesize(&self, _text: &str, _params: TtsParams) -> Result<Vec<u8>, TtsError> {
        // RVC is voice conversion (audio→audio), not text-to-speech.
        Err(TtsError::SynthesisFailed(
            "RVC is a voice conversion tool, not a TTS engine. Use the Sing tab to convert audio."
                .to_string(),
        ))
    }
}

// ── Singing Voice Conversion (Gradio API) ───────────────

impl LocalRVCProvider {
    /// Check if the RVC WebUI Gradio server is online.
    pub async fn check_health(&self) -> bool {
        let base = self.endpoint.trim_end_matches('/');
        // Gradio serves an HTML page at root when running
        match self
            .client
            .get(base)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Upload an audio file to Gradio's temp directory.
    /// Returns the server-side temp path for use in API calls.
    async fn upload_to_gradio(
        &self,
        audio_data: Vec<u8>,
        filename: &str,
    ) -> Result<String, String> {
        let base = self.endpoint.trim_end_matches('/');
        let url = format!("{}/upload", base);

        let part = reqwest::multipart::Part::bytes(audio_data)
            .file_name(filename.to_string())
            .mime_str("audio/wav")
            .map_err(|e| format!("MIME error: {}", e))?;

        let form = reqwest::multipart::Form::new().part("files", part);

        let response = self
            .client
            .post(&url)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Gradio upload failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Gradio upload error: {}", error_text));
        }

        // Gradio returns an array of uploaded file paths
        let paths: Vec<String> = response
            .json()
            .await
            .map_err(|e| format!("Gradio upload parse error: {}", e))?;

        paths
            .into_iter()
            .next()
            .ok_or_else(|| "Gradio upload returned empty path list".to_string())
    }

    /// Convert an audio file using the RVC WebUI Gradio API.
    ///
    /// Calls `/api/infer_convert` which maps to `vc.vc_single()` with parameters:
    /// [spk_item, input_audio, f0_up_key, f0_file, f0method, file_index, file_index2,
    ///  index_rate, filter_radius, resample_sr, rms_mix_rate, protect]
    pub async fn convert_audio(
        &self,
        audio_data: Vec<u8>,
        filename: &str,
        params: SingingConvertParams,
    ) -> Result<Vec<u8>, String> {
        let base = self.endpoint.trim_end_matches('/');

        // Step 1: Upload audio to Gradio temp dir
        let temp_path = self.upload_to_gradio(audio_data, filename).await?;
        println!("[RVC] Uploaded audio → {}", temp_path);

        // Step 2: Resolve parameters (per-request overrides > provider defaults)
        let f0_up_key = params.pitch_shift.unwrap_or(0.0) as i32;
        let f0_method = params
            .f0_method
            .unwrap_or_else(|| self.default_f0_method.clone());
        let index_path = params
            .index_path
            .unwrap_or_else(|| self.default_index_path.clone());
        let index_rate = params.index_rate.unwrap_or(self.default_index_rate);

        // Step 3: Call the Gradio infer_convert API
        // vc_single params: [spk_item, input_audio, f0_up_key, f0_file,
        //   f0method, file_index, file_index2, index_rate, filter_radius,
        //   resample_sr, rms_mix_rate, protect]
        let api_url = format!("{}/api/infer_convert", base);
        let payload = serde_json::json!({
            "data": [
                0,                              // spk_item (speaker id, usually 0)
                temp_path,                      // input_audio_path
                f0_up_key,                      // f0_up_key (pitch shift in semitones)
                null,                           // f0_file (optional, not used)
                f0_method,                      // f0method
                index_path,                     // file_index
                "",                             // file_index2 (auto-detect dropdown, leave empty)
                index_rate,                     // index_rate
                self.default_filter_radius,     // filter_radius
                self.default_resample_sr,       // resample_sr
                self.default_rms_mix_rate,      // rms_mix_rate
                self.default_protect,           // protect
            ]
        });

        println!(
            "[RVC] Calling {} with pitch={}, f0={}, index_rate={}",
            api_url, f0_up_key, f0_method, index_rate
        );

        let response = self
            .client
            .post(&api_url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| format!("RVC infer_convert request failed: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("RVC infer_convert error: {}", error_text));
        }

        // Step 4: Parse Gradio response
        // Gradio returns {"data": ["info string", {"name": "path/to/audio.wav", ...}]}
        let api_response: GradioApiResponse = response
            .json()
            .await
            .map_err(|e| format!("RVC response parse error: {}", e))?;

        if api_response.data.len() < 2 {
            return Err("RVC returned incomplete response".to_string());
        }

        // Log the info message from RVC
        if let Some(info) = api_response.data[0].as_str() {
            println!("[RVC] Result info: {}", info);
        }

        // The second element contains the audio — either as a file reference or inline data
        let audio_value = &api_response.data[1];

        // Gradio audio output can be:
        // 1. {"name": "path/to/file.wav", "data": null, "is_file": true}  — file on server
        // 2. {"name": "...", "data": "data:audio/wav;base64,..."}          — inline base64
        // 3. A tuple (sample_rate, [audio_samples...])                    — raw numpy array

        if let Some(obj) = audio_value.as_object() {
            // Case: file reference — download it
            if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                let file_url = if name.starts_with("http") {
                    name.to_string()
                } else {
                    format!("{}/file={}", base, name)
                };

                let audio_resp = self
                    .client
                    .get(&file_url)
                    .timeout(std::time::Duration::from_secs(60))
                    .send()
                    .await
                    .map_err(|e| format!("Failed to download RVC output: {}", e))?;

                let bytes = audio_resp
                    .bytes()
                    .await
                    .map_err(|e| format!("Failed to read RVC output bytes: {}", e))?;
                return Ok(bytes.to_vec());
            }
        }

        // Case: array (sample_rate, samples) — convert to WAV
        if let Some(arr) = audio_value.as_array() {
            if arr.len() == 2 {
                if let (Some(sr), Some(samples)) = (arr[0].as_i64(), arr[1].as_array()) {
                    let pcm: Vec<i16> = samples
                        .iter()
                        .filter_map(|v| {
                            v.as_f64()
                                .map(|f| (f * 32767.0).clamp(-32768.0, 32767.0) as i16)
                        })
                        .collect();
                    return Ok(encode_wav_i16(&pcm, sr as u32));
                }
            }
        }

        Err(format!(
            "RVC returned unexpected audio format: {}",
            serde_json::to_string(audio_value).unwrap_or_default()
        ))
    }

    /// Query available voice models from the RVC server.
    /// Calls Gradio's infer_refresh API and parses the model dropdown choices.
    pub async fn list_models(&self) -> Result<Vec<RvcModelInfo>, String> {
        let base = self.endpoint.trim_end_matches('/');
        let url = format!("{}/api/infer_refresh", base);

        let payload = serde_json::json!({ "data": [] });

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("RVC model refresh failed: {}", e))?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        // Gradio returns {"data": [{"choices": [...], ...}, ...]}
        let api_response: GradioApiResponse = response
            .json()
            .await
            .map_err(|e| format!("RVC model list parse error: {}", e))?;

        let mut models = Vec::new();

        if let Some(first) = api_response.data.first() {
            // The response contains a Gradio Dropdown update with choices
            if let Some(choices) = first
                .as_object()
                .and_then(|o| o.get("choices"))
                .and_then(|c| c.as_array())
            {
                for choice in choices {
                    if let Some(name) = choice.as_str() {
                        models.push(RvcModelInfo {
                            name: name.to_string(),
                            description: None,
                        });
                    } else if let Some(arr) = choice.as_array() {
                        // Gradio dropdown choices can be [value, label] tuples
                        if let Some(name) = arr.first().and_then(|v| v.as_str()) {
                            models.push(RvcModelInfo {
                                name: name.to_string(),
                                description: None,
                            });
                        }
                    }
                }
            }
        }

        Ok(models)
    }

    /// Get the configured endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Encode PCM i16 samples as a WAV file.
fn encode_wav_i16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_size = samples.len() as u32 * 2;
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size as usize);
    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&num_channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}
