use crate::ai::context::AIOrchestrator;
use crate::commands::live2d::load_active_live2d_profile;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{command, Manager, State};
use tokio::sync::RwLock;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/chyinan/Kokoro-Engine/releases/latest";

#[derive(Clone)]
pub struct WindowSizeState {
    pub width: Arc<RwLock<u32>>,
    pub height: Arc<RwLock<u32>>,
}

impl Default for WindowSizeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowSizeState {
    pub fn new() -> Self {
        Self {
            width: Arc::new(RwLock::new(800)),
            height: Arc::new(RwLock::new(600)),
        }
    }
    pub async fn get(&self) -> (u32, u32) {
        (*self.width.read().await, *self.height.read().await)
    }
    pub async fn set(&self, w: u32, h: u32) {
        *self.width.write().await = w;
        *self.height.write().await = h;
    }
}

#[command]
pub async fn set_window_size(
    state: State<'_, WindowSizeState>,
    width: u32,
    height: u32,
) -> Result<(), KokoroError> {
    state.set(width, height).await;
    Ok(())
}

#[derive(Serialize)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
}

#[derive(Serialize)]
pub struct ReleaseUpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub html_url: String,
    pub update_available: bool,
}

#[derive(Deserialize)]
struct GitHubReleaseResponse {
    tag_name: String,
    html_url: String,
}

#[derive(Serialize)]
pub struct SystemStatus {
    pub engine_running: bool,
    pub active_modules: Vec<String>,
    pub memory_usage_mb: f64,
}

/// Returns basic engine metadata for the frontend to display.
#[tauri::command]
pub fn get_engine_info() -> EngineInfo {
    EngineInfo {
        name: "Kokoro Engine".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
    }
}

#[tauri::command]
pub async fn check_latest_release() -> Result<ReleaseUpdateInfo, KokoroError> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("Kokoro-Engine/{}", current_version))
        .build()?;

    let release = client
        .get(GITHUB_LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubReleaseResponse>()
        .await?;

    if release.tag_name.trim().is_empty() {
        return Err(KokoroError::ExternalService(
            "GitHub latest release response did not include a tag name".to_string(),
        ));
    }

    Ok(ReleaseUpdateInfo {
        update_available: compare_release_versions(&release.tag_name, &current_version) > 0,
        current_version,
        latest_version: release.tag_name,
        html_url: release.html_url,
    })
}

/// Returns the current system status including active modules.
#[tauri::command]
pub fn get_system_status(app: tauri::AppHandle, state: State<'_, AIOrchestrator>) -> SystemStatus {
    let mut active_modules = Vec::new();

    if app.try_state::<AIOrchestrator>().is_some() {
        active_modules.push("core".to_string());
    }
    if app.try_state::<crate::tts::TtsService>().is_some() {
        active_modules.push("tts".to_string());
    }
    if app.try_state::<crate::stt::SttService>().is_some() {
        active_modules.push("stt".to_string());
    }
    if app
        .try_state::<crate::imagegen::ImageGenService>()
        .is_some()
    {
        active_modules.push("imagegen".to_string());
    }
    if app
        .try_state::<std::sync::Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>()
        .is_some()
    {
        active_modules.push("mcp".to_string());
    }
    if let Some(mod_manager) = app.try_state::<tokio::sync::Mutex<crate::mods::ModManager>>() {
        if let Ok(manager) = mod_manager.try_lock() {
            if let Some(module_tag) = manager.runtime_status_module_tag() {
                active_modules.push(module_tag);
            }
        } else {
            active_modules.push("mods:unknown".to_string());
        }
    }
    if app
        .try_state::<std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>>()
        .is_some()
    {
        active_modules.push("vision".to_string());
    }
    if load_active_live2d_profile().is_some() {
        active_modules.push("live2d".to_string());
    }
    if state.is_memory_enabled() {
        active_modules.push("memory".to_string());
    }
    if state.is_proactive_enabled() {
        active_modules.push("proactive".to_string());
    }

    SystemStatus {
        engine_running: !active_modules.is_empty(),
        active_modules,
        memory_usage_mb: 0.0,
    }
}

fn compare_release_versions(left: &str, right: &str) -> i8 {
    let left_parts = release_version_parts(left);
    let right_parts = release_version_parts(right);
    let max_len = left_parts.len().max(right_parts.len());

    for i in 0..max_len {
        let left_part = *left_parts.get(i).unwrap_or(&0);
        let right_part = *right_parts.get(i).unwrap_or(&0);
        if left_part != right_part {
            return if left_part > right_part { 1 } else { -1 };
        }
    }

    0
}

fn release_version_parts(version: &str) -> Vec<u64> {
    let numeric = version
        .trim()
        .trim_start_matches(|ch: char| !ch.is_ascii_digit());

    numeric
        .split(|ch: char| !ch.is_ascii_digit())
        .take_while(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{compare_release_versions, release_version_parts};

    #[test]
    fn release_version_parts_ignores_tag_prefix_and_suffix() {
        assert_eq!(release_version_parts("v0.2.9"), vec![0, 2, 9]);
        assert_eq!(
            release_version_parts("release-v1.4.0-beta.1"),
            vec![1, 4, 0]
        );
    }

    #[test]
    fn compare_release_versions_detects_minor_and_patch_updates() {
        assert_eq!(compare_release_versions("v0.3.0", "0.2.9"), 1);
        assert_eq!(compare_release_versions("0.2.10", "0.2.9"), 1);
        assert_eq!(compare_release_versions("0.2.9", "v0.2.10"), -1);
    }

    #[test]
    fn compare_release_versions_treats_missing_parts_as_zero() {
        assert_eq!(compare_release_versions("v1.2.0", "1.2"), 0);
        assert_eq!(compare_release_versions("1", "1.0.1"), -1);
    }
}
