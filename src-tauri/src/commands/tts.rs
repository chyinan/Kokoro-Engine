use crate::tts::config::{save_config, TtsSystemConfig};
use crate::tts::{ProviderStatus, TtsParams, TtsService, VoiceProfile};
use tauri::{command, AppHandle, State};

#[derive(serde::Deserialize)]
pub struct TtsConfig {
    pub provider_id: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f32>,
    pub pitch: Option<f32>,
    pub emotion: Option<String>,
}

#[command]
pub async fn synthesize(
    app: AppHandle,
    state: State<'_, TtsService>,
    text: String,
    config: TtsConfig,
) -> Result<(), String> {
    let params = TtsParams {
        voice: config.voice,
        speed: config.speed,
        pitch: config.pitch,
        emotion: config.emotion,
        required_capabilities: None,
        extra_params: None,
    };

    state
        .speak(app, text, config.provider_id, Some(params))
        .await
}

#[command]
pub async fn list_tts_providers(
    state: State<'_, TtsService>,
) -> Result<Vec<ProviderStatus>, String> {
    Ok(state.list_providers().await)
}

#[command]
pub async fn list_tts_voices(state: State<'_, TtsService>) -> Result<Vec<VoiceProfile>, String> {
    Ok(state.list_voices().await)
}

#[command]
pub async fn get_tts_provider_status(
    state: State<'_, TtsService>,
    provider_id: String,
) -> Result<Option<ProviderStatus>, String> {
    Ok(state.get_provider_status(&provider_id).await)
}

#[command]
pub async fn clear_tts_cache(state: State<'_, TtsService>) -> Result<(), String> {
    state.clear_cache().await;
    Ok(())
}

/// Return the current TTS config from disk.
#[command]
pub async fn get_tts_config() -> Result<TtsSystemConfig, String> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("tts_config.json");
    Ok(crate::tts::load_config(&config_path))
}

/// Save TTS config to disk and hot-reload providers.
#[command]
pub async fn save_tts_config(
    state: State<'_, TtsService>,
    config: TtsSystemConfig,
) -> Result<(), String> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("tts_config.json");

    // Write to disk
    save_config(&config_path, &config)?;

    // Hot-reload providers
    state.reload_from_config(&config).await;

    Ok(())
}

/// Scan a GPT-SoVITS install directory for available GPT and SoVITS model files.
#[command]
pub async fn list_gpt_sovits_models(install_path: String) -> Result<GptSovitsModels, String> {
    let root = std::path::Path::new(&install_path);
    if !root.is_dir() {
        return Err(format!("Directory not found: {}", install_path));
    }

    let mut gpt_models = Vec::new();
    let mut sovits_models = Vec::new();

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !entry.file_type().map_or(false, |ft| ft.is_dir()) {
                continue;
            }

            let is_gpt = name.starts_with("GPT_weights");
            let is_sovits = name.starts_with("SoVITS_weights");
            if !is_gpt && !is_sovits {
                continue;
            }

            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().to_string();
                    let rel_path = format!("{}/{}", name, fname);

                    if is_gpt && fname.ends_with(".ckpt") {
                        gpt_models.push(rel_path);
                    } else if is_sovits && fname.ends_with(".pth") {
                        sovits_models.push(rel_path);
                    }
                }
            }
        }
    }

    gpt_models.sort();
    sovits_models.sort();

    Ok(GptSovitsModels {
        gpt_models,
        sovits_models,
    })
}

#[derive(serde::Serialize)]
pub struct GptSovitsModels {
    pub gpt_models: Vec<String>,
    pub sovits_models: Vec<String>,
}
