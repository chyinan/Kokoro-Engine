use crate::imagegen::config::{load_config, save_config, ImageGenSystemConfig};
use crate::imagegen::{ImageGenParams, ImageGenResult, ImageGenService};
use tauri::{command, State};

#[command]
pub async fn generate_image(
    state: State<'_, ImageGenService>,
    prompt: String,
    provider_id: Option<String>,
    params: Option<ImageGenParams>,
) -> Result<ImageGenResult, String> {
    state
        .generate(prompt, provider_id, params)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_imagegen_config() -> Result<ImageGenSystemConfig, String> {
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
) -> Result<(), String> {
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let config_path = app_data.join("imagegen_config.json");

    // Write to disk
    save_config(&config_path, &config).map_err(|e| e.to_string())?;

    // Hot-reload service
    state.reload_from_config(&config).await;

    Ok(())
}

/// Test connectivity to a Stable Diffusion WebUI instance.
/// Returns the list of available SD models (checkpoints) on success.
#[command]
pub async fn test_sd_connection(base_url: String) -> Result<Vec<String>, String> {
    let mut url_str = base_url.trim().trim_end_matches('/').to_string();
    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
        url_str = format!("http://{}", url_str);
    }

    let url = format!("{}/sdapi/v1/sd-models", url_str);
    println!("[ImageGen] Testing SD connection: {}", url);

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let res = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Cannot connect to SD WebUI at {}: {}", url, e))?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(format!(
            "SD WebUI at {} returned error {}: {}",
            url, status, text
        ));
    }

    let models: Vec<serde_json::Value> = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse SD models response: {}", e))?;

    let model_names: Vec<String> = models
        .iter()
        .filter_map(|m| m.get("title").and_then(|t| t.as_str()).map(String::from))
        .collect();

    println!(
        "[ImageGen] SD connection OK, {} models found",
        model_names.len()
    );
    Ok(model_names)
}
