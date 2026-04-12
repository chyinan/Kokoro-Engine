use crate::commands::system::WindowSizeState;
use crate::error::KokoroError;
use crate::imagegen::config::{load_config, save_config, ImageGenSystemConfig};
use crate::imagegen::{ImageGenParams, ImageGenResult, ImageGenService};
use tauri::{command, State};

#[command]
pub async fn generate_image(
    state: State<'_, ImageGenService>,
    window_size_state: State<'_, WindowSizeState>,
    prompt: String,
    provider_id: Option<String>,
    params: Option<ImageGenParams>,
) -> Result<ImageGenResult, KokoroError> {
    let window_size = window_size_state.get().await;
    state
        .generate(prompt, provider_id, params, Some(window_size))
        .await
        .map_err(KokoroError::from)
}

#[command]
pub async fn get_imagegen_config() -> Result<ImageGenSystemConfig, KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("imagegen_config.json");
    Ok(load_config(&config_path))
}

#[command]
pub async fn save_imagegen_config(
    state: State<'_, ImageGenService>,
    config: ImageGenSystemConfig,
) -> Result<(), KokoroError> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("imagegen_config.json");
    save_config(&config_path, &config).map_err(|e| KokoroError::Config(e.to_string()))?;
    state
        .reload_from_config(&config)
        .await
        .map_err(|e| KokoroError::Config(format!("failed to reload imagegen providers: {}", e)))?;
    Ok(())
}

#[command]
pub async fn test_sd_connection(base_url: String) -> Result<Vec<String>, KokoroError> {
    let mut url_str = base_url.trim().trim_end_matches('/').to_string();
    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
        url_str = format!("http://{}", url_str);
    }

    let url = format!("{}/sdapi/v1/sd-models", url_str);
    tracing::info!(target: "imagegen", "Testing SD connection: {}", url);

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| {
            KokoroError::ExternalService(format!("Failed to create HTTP client: {}", e))
        })?;

    let res = client.get(&url).send().await.map_err(|e| {
        KokoroError::ExternalService(format!("Cannot connect to SD WebUI at {}: {}", url, e))
    })?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(KokoroError::ExternalService(format!(
            "SD WebUI at {} returned error {}: {}",
            url, status, text
        )));
    }

    let models: Vec<serde_json::Value> = res.json().await.map_err(|e| {
        KokoroError::ExternalService(format!("Failed to parse SD models response: {}", e))
    })?;

    let model_names: Vec<String> = models
        .iter()
        .filter_map(|m| m.get("title").and_then(|t| t.as_str()).map(String::from))
        .collect();

    tracing::info!(
        target: "imagegen",
        "SD connection OK, {} models found",
        model_names.len()
    );
    Ok(model_names)
}
