use crate::tts::local_rvc::{LocalRVCProvider, RvcModelInfo, SingingConvertParams};
use serde::Serialize;
use tauri::{command, AppHandle, Emitter};

/// Result of a singing voice conversion.
#[derive(Debug, Clone, Serialize)]
pub struct SingingResult {
    /// Path to the converted audio file
    pub output_path: String,
    /// Duration in seconds (estimated from file size)
    pub duration_secs: f32,
}

/// Check if the RVC server is online and reachable.
#[command]
pub async fn check_rvc_status(app: AppHandle) -> Result<bool, String> {
    let provider = get_rvc_provider(&app)?;
    Ok(provider.check_health().await)
}

/// List available voice models on the RVC server.
#[command]
pub async fn list_rvc_models(app: AppHandle) -> Result<Vec<RvcModelInfo>, String> {
    let provider = get_rvc_provider(&app)?;
    provider.list_models().await
}

/// Convert a song/audio file to the character's voice using RVC.
///
/// Reads the source audio file, sends it to the RVC server for voice conversion,
/// saves the result to a temporary file, and emits progress events.
#[command]
pub async fn convert_singing(
    app: AppHandle,
    audio_path: String,
    model_name: Option<String>,
    pitch_shift: Option<f32>,
    separate_vocals: Option<bool>,
    // Advanced RVC params (optional overrides)
    f0_method: Option<String>,
    index_path: Option<String>,
    index_rate: Option<f32>,
) -> Result<SingingResult, String> {
    // Emit start event
    app.emit(
        "singing:progress",
        serde_json::json!({
            "stage": "reading",
            "progress": 0.0,
        }),
    )
    .ok();

    // Read the source audio file
    let audio_data = tokio::fs::read(&audio_path)
        .await
        .map_err(|e| format!("Failed to read audio file: {}", e))?;

    let filename = std::path::Path::new(&audio_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.mp3")
        .to_string();

    // Emit converting event
    app.emit(
        "singing:progress",
        serde_json::json!({
            "stage": "converting",
            "progress": 0.2,
        }),
    )
    .ok();

    // Get or create RVC provider
    let provider = get_rvc_provider(&app)?;

    // Convert
    let params = SingingConvertParams {
        model_name,
        pitch_shift,
        separate_vocals,
        f0_method,
        index_path,
        index_rate,
    };

    let converted = provider
        .convert_audio(audio_data, &filename, params)
        .await?;

    // Save to temporary file
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("singing");

    tokio::fs::create_dir_all(&app_data)
        .await
        .map_err(|e| format!("Failed to create singing dir: {}", e))?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let output_filename = format!("singing_{}.wav", timestamp);
    let output_path = app_data.join(&output_filename);

    tokio::fs::write(&output_path, &converted)
        .await
        .map_err(|e| format!("Failed to write output: {}", e))?;

    let output_path_str = output_path.to_string_lossy().to_string();

    // Rough duration estimate (assuming ~176kbps WAV at 44.1kHz stereo 16-bit)
    let estimated_duration = converted.len() as f32 / (44100.0 * 2.0 * 2.0);

    // Emit done event
    app.emit(
        "singing:progress",
        serde_json::json!({
            "stage": "done",
            "progress": 1.0,
            "output_path": &output_path_str,
        }),
    )
    .ok();

    Ok(SingingResult {
        output_path: output_path_str,
        duration_secs: estimated_duration,
    })
}

/// Helper: create an RVC provider from stored config or defaults.
fn get_rvc_provider(_app: &AppHandle) -> Result<LocalRVCProvider, String> {
    // Try to get endpoint from localStorage-stored config
    // For now, use sensible defaults. The user configures this in TTS provider settings.
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("tts_config.json");

    let config = crate::tts::load_config(&config_path);

    // Find an RVC provider in the config
    for provider_config in &config.providers {
        if provider_config.provider_type == "local_rvc" && provider_config.enabled {
            if let Some(provider) = LocalRVCProvider::from_config(provider_config) {
                return Ok(provider);
            }
        }
    }

    // Fallback: default endpoint
    Ok(LocalRVCProvider::new(
        "http://localhost:7865".to_string(),
        None,
    ))
}
