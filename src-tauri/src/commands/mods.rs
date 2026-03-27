use crate::mods::{ModManager, ModManifest, ModThemeJson};
use serde_json::Value as JsonValue;
use std::fs;
use std::io;
use tauri::{command, AppHandle, State};
use tokio::sync::Mutex;

/// Validate mod ID format: must be non-empty and contain only alphanumeric, underscore, or dash
fn is_valid_mod_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// Check if a file extension is allowed for MOD extraction
fn is_allowed_mod_file(ext: &str) -> bool {
    const ALLOWED_EXTENSIONS: &[&str] = &[
        "html", "js", "css", "json", "png", "jpg", "jpeg", "webp",
        "svg", "gif", "woff", "woff2", "ttf", "otf", "txt", "md",
    ];
    ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

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
    if !is_valid_mod_id(&manifest.id) {
        return Err("Invalid mod ID. Must be alphanumeric, underscore or dash.".to_string());
    }

    // Target directory
    let target_dir = mods_dir.join(&manifest.id);
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir).map_err(|e| format!("Failed to remove old mod: {}", e))?;
    }
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

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
            if !is_allowed_mod_file(&ext) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_mod_id_empty() {
        assert!(!is_valid_mod_id(""), "Empty ID should be invalid");
    }

    #[test]
    fn test_is_valid_mod_id_valid_alphanumeric() {
        assert!(is_valid_mod_id("mymod"), "Alphanumeric ID should be valid");
        assert!(is_valid_mod_id("MyMod123"), "Mixed case alphanumeric should be valid");
    }

    #[test]
    fn test_is_valid_mod_id_valid_with_underscore() {
        assert!(
            is_valid_mod_id("my_mod"),
            "ID with underscore should be valid"
        );
        assert!(
            is_valid_mod_id("_private_mod"),
            "ID starting with underscore should be valid"
        );
    }

    #[test]
    fn test_is_valid_mod_id_valid_with_dash() {
        assert!(is_valid_mod_id("my-mod"), "ID with dash should be valid");
        assert!(
            is_valid_mod_id("my-mod-123"),
            "ID with multiple dashes should be valid"
        );
    }

    #[test]
    fn test_is_valid_mod_id_invalid_special_chars() {
        assert!(
            !is_valid_mod_id("my.mod"),
            "ID with dot should be invalid"
        );
        assert!(
            !is_valid_mod_id("my@mod"),
            "ID with @ should be invalid"
        );
        assert!(
            !is_valid_mod_id("my mod"),
            "ID with space should be invalid"
        );
        assert!(
            !is_valid_mod_id("my/mod"),
            "ID with slash should be invalid"
        );
    }

    #[test]
    fn test_is_allowed_mod_file_allowed_extensions() {
        assert!(is_allowed_mod_file("html"), "html should be allowed");
        assert!(is_allowed_mod_file("js"), "js should be allowed");
        assert!(is_allowed_mod_file("css"), "css should be allowed");
        assert!(is_allowed_mod_file("json"), "json should be allowed");
        assert!(is_allowed_mod_file("png"), "png should be allowed");
        assert!(is_allowed_mod_file("jpg"), "jpg should be allowed");
        assert!(is_allowed_mod_file("jpeg"), "jpeg should be allowed");
        assert!(is_allowed_mod_file("webp"), "webp should be allowed");
        assert!(is_allowed_mod_file("svg"), "svg should be allowed");
        assert!(is_allowed_mod_file("gif"), "gif should be allowed");
        assert!(is_allowed_mod_file("woff"), "woff should be allowed");
        assert!(is_allowed_mod_file("woff2"), "woff2 should be allowed");
        assert!(is_allowed_mod_file("ttf"), "ttf should be allowed");
        assert!(is_allowed_mod_file("otf"), "otf should be allowed");
        assert!(is_allowed_mod_file("txt"), "txt should be allowed");
        assert!(is_allowed_mod_file("md"), "md should be allowed");
    }

    #[test]
    fn test_is_allowed_mod_file_case_insensitive() {
        assert!(is_allowed_mod_file("HTML"), "HTML uppercase should be allowed");
        assert!(is_allowed_mod_file("Js"), "Js mixed case should be allowed");
        assert!(is_allowed_mod_file("JSON"), "JSON uppercase should be allowed");
    }

    #[test]
    fn test_is_allowed_mod_file_disallowed_extensions() {
        assert!(
            !is_allowed_mod_file("exe"),
            "exe should not be allowed"
        );
        assert!(
            !is_allowed_mod_file("sh"),
            "sh should not be allowed"
        );
        assert!(
            !is_allowed_mod_file("bat"),
            "bat should not be allowed"
        );
        assert!(
            !is_allowed_mod_file("dll"),
            "dll should not be allowed"
        );
        assert!(
            !is_allowed_mod_file("so"),
            "so should not be allowed"
        );
        assert!(
            !is_allowed_mod_file("zip"),
            "zip should not be allowed"
        );
    }

    #[test]
    fn test_is_allowed_mod_file_empty_extension() {
        assert!(
            !is_allowed_mod_file(""),
            "empty extension should not be allowed"
        );
    }
}
