use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

const BUBBLE_WIDTH: i32 = 320;
const BUBBLE_HEIGHT: i32 = 240;
const BUBBLE_GAP: i32 = 8;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PetConfig {
    pub enabled: bool,
    pub position_x: i32,
    pub position_y: i32,
    pub shortcut: String,
    pub model_url: Option<String>,
    #[serde(default)]
    pub window_width: u32,
    #[serde(default)]
    pub window_height: u32,
    #[serde(default)]
    pub model_scale: f32,
    #[serde(default = "default_render_fps")]
    pub render_fps: u32,
}

fn default_render_fps() -> u32 {
    60
}

impl Default for PetConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            position_x: 100,
            position_y: 100,
            shortcut: "CmdOrCtrl+Shift+Space".to_string(),
            model_url: None,
            window_width: 0,
            window_height: 0,
            model_scale: 0.0,
            render_fps: default_render_fps(),
        }
    }
}

fn app_data_dir() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
}

pub fn load_pet_config() -> PetConfig {
    let path = app_data_dir().join("pet_config.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<PetConfig>(&content) {
            return cfg;
        }
    }
    PetConfig::default()
}

fn save_pet_config_to_disk(config: &PetConfig) -> Result<(), KokoroError> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir).map_err(KokoroError::from)?;
    let path = dir.join("pet_config.json");
    let content = serde_json::to_string_pretty(config).map_err(KokoroError::from)?;
    std::fs::write(&path, content).map_err(KokoroError::from)
}

fn bubble_position_for_pet_bounds(pet_x: i32, pet_y: i32, pet_width: u32) -> (i32, i32) {
    let x = pet_x + (pet_width as i32 - BUBBLE_WIDTH) / 2;
    let y = pet_y - BUBBLE_HEIGHT - BUBBLE_GAP;
    (x, y)
}

fn bubble_position_from_pet_window(
    app: &tauri::AppHandle,
) -> Result<Option<(i32, i32)>, KokoroError> {
    let Some(pet) = app.get_webview_window("pet") else {
        return Ok(None);
    };

    if !pet.is_visible().unwrap_or(false) {
        return Ok(None);
    }

    let pos = pet
        .outer_position()
        .map_err(|e| KokoroError::Internal(e.to_string()))?;
    let size = pet
        .inner_size()
        .map_err(|e| KokoroError::Internal(e.to_string()))?;

    Ok(Some(bubble_position_for_pet_bounds(
        pos.x, pos.y, size.width,
    )))
}

pub fn sync_bubble_window_to_pet(app: &tauri::AppHandle) -> Result<(), KokoroError> {
    let Some(bubble) = app.get_webview_window("bubble") else {
        return Ok(());
    };

    let Some((bx, by)) = bubble_position_from_pet_window(app)? else {
        return Ok(());
    };

    bubble
        .set_position(tauri::PhysicalPosition::new(bx, by))
        .map_err(|e| KokoroError::Internal(e.to_string()))?;

    Ok(())
}

pub fn hide_bubble_window_if_open(app: &tauri::AppHandle) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("bubble") {
        win.hide()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn show_pet_window(app: tauri::AppHandle) -> Result<(), KokoroError> {
    tracing::info!(target: "pet", "show_pet_window called");
    let windows: Vec<String> = app.webview_windows().keys().cloned().collect();
    tracing::info!(target: "pet", "available windows: {:?}", windows);

    if let Some(win) = app.get_webview_window("pet") {
        tracing::info!(target: "pet", "found existing pet window, showing...");
        let cfg = load_pet_config();
        let x = if cfg.position_x != 0 {
            cfg.position_x
        } else {
            100
        };
        let y = if cfg.position_y != 0 {
            cfg.position_y
        } else {
            100
        };
        win.set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        let w = if cfg.window_width >= 100 {
            cfg.window_width
        } else {
            400
        };
        let h = if cfg.window_height >= 100 {
            cfg.window_height
        } else {
            600
        };
        win.set_size(tauri::PhysicalSize::new(w, h))
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        win.show()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        win.set_focus()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        sync_bubble_window_to_pet(&app)?;
        tracing::info!(target: "pet", "pet window shown successfully");
    } else {
        tracing::info!(target: "pet", "pet window not found, creating new one...");
        let cfg = load_pet_config();
        let x = if cfg.position_x != 0 {
            cfg.position_x
        } else {
            100
        };
        let y = if cfg.position_y != 0 {
            cfg.position_y
        } else {
            100
        };
        let w = if cfg.window_width >= 100 {
            cfg.window_width
        } else {
            400
        };
        let h = if cfg.window_height >= 100 {
            cfg.window_height
        } else {
            600
        };

        let url = if cfg!(debug_assertions) {
            tauri::WebviewUrl::External(
                "http://localhost:1420/src/windows/pet.html"
                    .parse()
                    .unwrap(),
            )
        } else {
            tauri::WebviewUrl::App("src/windows/pet.html".into())
        };

        let win = tauri::WebviewWindowBuilder::new(&app, "pet", url)
            .title("Kokoro Pet")
            .inner_size(w as f64, h as f64)
            .position(x as f64, y as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .build()
            .map_err(|e: tauri::Error| KokoroError::Internal(e.to_string()))?;

        win.show()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        win.set_focus()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        sync_bubble_window_to_pet(&app)?;
        tracing::info!(target: "pet", "pet window created and shown successfully");
    }
    Ok(())
}

#[tauri::command]
pub async fn hide_pet_window(app: tauri::AppHandle) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("pet") {
        win.hide()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        hide_bubble_window_if_open(&app)?;

        // Update config to reflect window is closed
        let mut cfg = load_pet_config();
        cfg.enabled = false;
        save_pet_config_to_disk(&cfg)?;

        // Emit event to notify main window
        app.emit("pet-window-closed", ())
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_pet_drag_mode(app: tauri::AppHandle, _enabled: bool) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("pet") {
        win.set_ignore_cursor_events(false)
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_pet_config(_app: tauri::AppHandle) -> Result<PetConfig, KokoroError> {
    Ok(load_pet_config())
}

#[tauri::command]
pub async fn save_pet_config(_app: tauri::AppHandle, config: PetConfig) -> Result<(), KokoroError> {
    save_pet_config_to_disk(&config)
}

#[tauri::command]
pub async fn move_pet_window(app: tauri::AppHandle, x: i32, y: i32) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("pet") {
        win.set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        sync_bubble_window_to_pet(&app)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn resize_pet_window(
    app: tauri::AppHandle,
    width: u32,
    height: u32,
) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("pet") {
        win.set_size(tauri::PhysicalSize::new(width, height))
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        sync_bubble_window_to_pet(&app)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn show_bubble_window(app: tauri::AppHandle, text: String) -> Result<(), KokoroError> {
    if app.get_webview_window("pet").is_none() {
        return Err(KokoroError::NotFound("Pet window not found".to_string()));
    }

    let Some((bx, by)) = bubble_position_from_pet_window(&app)? else {
        return Ok(());
    };

    if let Some(existing) = app.get_webview_window("bubble") {
        existing
            .set_position(tauri::PhysicalPosition::new(bx, by))
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        existing
            .show()
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        existing
            .emit("bubble-text-update", &text)
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
    } else {
        let url = if cfg!(debug_assertions) {
            tauri::WebviewUrl::External(
                "http://localhost:1420/src/windows/bubble.html"
                    .parse()
                    .unwrap(),
            )
        } else {
            tauri::WebviewUrl::App("src/windows/bubble.html".into())
        };

        let win = tauri::WebviewWindowBuilder::new(&app, "bubble", url)
            .title("")
            .inner_size(BUBBLE_WIDTH as f64, BUBBLE_HEIGHT as f64)
            .position(bx as f64, by as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .build()
            .map_err(|e: tauri::Error| KokoroError::Internal(e.to_string()))?;

        win.set_ignore_cursor_events(false)
            .map_err(|e: tauri::Error| KokoroError::Internal(e.to_string()))?;

        let win_clone = win.clone();
        let text_clone = text.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = win_clone.emit("bubble-text-update", &text_clone);
        });
    }

    Ok(())
}

#[tauri::command]
pub async fn update_bubble_text(app: tauri::AppHandle, text: String) -> Result<(), KokoroError> {
    if let Some(win) = app.get_webview_window("bubble") {
        sync_bubble_window_to_pet(&app)?;
        win.emit("bubble-text-update", &text)
            .map_err(|e| KokoroError::Internal(e.to_string()))?;
        Ok(())
    } else {
        Err(KokoroError::NotFound("bubble window not found".to_string()))
    }
}

#[tauri::command]
pub async fn hide_bubble_window(app: tauri::AppHandle) -> Result<(), KokoroError> {
    hide_bubble_window_if_open(&app)
}

#[cfg(test)]
mod tests {
    use super::{bubble_position_for_pet_bounds, BUBBLE_GAP, BUBBLE_HEIGHT, BUBBLE_WIDTH};

    #[test]
    fn bubble_position_is_centered_above_pet_window() {
        let (x, y) = bubble_position_for_pet_bounds(100, 400, 500);

        assert_eq!(x, 100 + (500 - BUBBLE_WIDTH) / 2);
        assert_eq!(y, 400 - BUBBLE_HEIGHT - BUBBLE_GAP);
    }

    #[test]
    fn bubble_position_supports_windows_narrower_than_bubble() {
        let (x, _) = bubble_position_for_pet_bounds(50, 200, 200);

        assert_eq!(x, 50 + (200 - BUBBLE_WIDTH) / 2);
    }
}
