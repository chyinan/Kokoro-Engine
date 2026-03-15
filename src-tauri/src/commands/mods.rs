use crate::mods::{ModManager, ModManifest, ModThemeJson};
use serde_json::Value as JsonValue;
use std::fs;
use std::io;
use tauri::{command, AppHandle, State};
use tokio::sync::Mutex;

#[command]
pub async fn list_mods(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Vec<ModManifest>, String> {
    let mut manager = mod_manager.lock().await;
    Ok(manager.scan_mods())
}

#[command]
pub async fn load_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    app_handle: AppHandle,
    mod_id: String,
) -> Result<(), String> {
    let mut manager = mod_manager.lock().await;
    manager.load_mod(&mod_id, &app_handle).await
}

#[command]
pub async fn get_mod_theme(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Option<ModThemeJson>, String> {
    let manager = mod_manager.lock().await;
    Ok(manager.get_active_theme().cloned())
}

#[command]
pub async fn get_mod_layout(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Option<JsonValue>, String> {
    let manager = mod_manager.lock().await;
    Ok(manager.get_active_layout().cloned())
}

#[command]
pub async fn install_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    file_path: String,
) -> Result<ModManifest, String> {
    // 先读取 mods_path，然后立即释放锁，避免在持锁状态下执行大量文件 I/O
    let mods_dir = {
        let manager = mod_manager.lock().await;
        manager.mods_path.clone()
    };

    // verify file exists
    let archive_path = std::path::Path::new(&file_path);
    if !archive_path.exists() {
        return Err("File does not exist".to_string());
    }

    // Open zip
    let file = fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Find mod.json
    let mut manifest_content = String::new();
    {
        let mut manifest_file = archive
            .by_name("mod.json")
            .map_err(|_| "mod.json not found in archive root".to_string())?;
        std::io::Read::read_to_string(&mut manifest_file, &mut manifest_content)
            .map_err(|e| e.to_string())?;
    }

    // Parse manifest
    let manifest: ModManifest =
        serde_json::from_str(&manifest_content).map_err(|e| format!("Invalid mod.json: {}", e))?;

    // Validate ID
    if manifest.id.is_empty()
        || manifest
            .id
            .chars()
            .any(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    {
        return Err("Invalid mod ID. Must be alphanumeric, underscore or dash.".to_string());
    }

    // Target directory
    let target_dir = mods_dir.join(&manifest.id);
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir).map_err(|e| format!("Failed to remove old mod: {}", e))?;
    }
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    // 文件类型白名单，禁止提取可执行文件
    const ALLOWED_EXTENSIONS: &[&str] = &[
        "html", "js", "css", "json", "png", "jpg", "jpeg", "webp",
        "svg", "gif", "woff", "woff2", "ttf", "otf", "txt", "md",
    ];
    // 单文件最大 10MB，MOD 包总解压大小最大 50MB
    const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
    const MAX_TOTAL_SIZE: u64 = 50 * 1024 * 1024;

    // Extract
    let mut total_size: u64 = 0;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => target_dir.join(path),
            None => continue,
        };

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            // 检查文件扩展名白名单
            let ext = outpath
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
                continue; // 跳过不允许的文件类型
            }

            // 检查单文件大小
            if file.size() > MAX_FILE_SIZE {
                return Err(format!(
                    "File '{}' exceeds maximum size of 10MB",
                    file.name()
                ));
            }

            // 检查总解压大小（防止 zip bomb）
            total_size += file.size();
            if total_size > MAX_TOTAL_SIZE {
                return Err("MOD package total size exceeds 50MB limit".to_string());
            }

            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }

    Ok(manifest)
}

#[command]
pub async fn dispatch_mod_event(
    mod_manager: State<'_, Mutex<ModManager>>,
    event: String,
    payload: JsonValue,
) -> Result<(), String> {
    let manager = mod_manager.lock().await;
    manager.dispatch_event(&event, payload).await
}

#[command]
pub async fn unload_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut manager = mod_manager.lock().await;
    manager.unload_mod(&app_handle);
    Ok(())
}
