use serde::Serialize;
use std::fs;
use std::io;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Serialize)]
pub struct Live2dModelInfo {
    /// Human-friendly name (top-level folder name)
    pub name: String,
    /// Relative path to the .model3.json (used for protocol URL)
    pub path: String,
}

/// Extract a Live2D character zip package and return the path to the .model3.json file.
///
/// Official Live2D packages have a structure like:
/// ```text
/// character_name/
/// ├── runtime/
/// │   ├── *.model3.json   ← entry point we need
/// │   ├── *.moc3
/// │   ├── *.physics3.json
/// │   ├── textures/
/// │   └── motion/
/// └── (editor files, ReadMe, etc.)
/// ```
///
/// We extract the full zip into `{app_data_dir}/live2d_models/` and then
/// locate the `.model3.json` inside.
#[tauri::command]
pub async fn import_live2d_zip(app: tauri::AppHandle, zip_path: String) -> Result<String, String> {
    let archive_path = std::path::Path::new(&zip_path);
    if !archive_path.exists() {
        return Err("Zip file does not exist".to_string());
    }

    // Determine target directory
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    let models_dir = app_data.join("live2d_models");
    fs::create_dir_all(&models_dir).map_err(|e| format!("Failed to create models dir: {}", e))?;

    // Open zip archive
    let file = fs::File::open(archive_path).map_err(|e| format!("Failed to open zip: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {}", e))?;

    // Extract all entries
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match entry.enclosed_name() {
            Some(path) => models_dir.join(path),
            None => continue,
        };

        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            io::copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
    }

    // Find the .model3.json file in the extracted contents and return path relative to models_dir
    let model_json = find_model3_json(&models_dir)
        .ok_or_else(|| "No .model3.json file found in the zip archive".to_string())?;

    // Return path relative to models_dir so the frontend can construct a protocol URL
    let relative = model_json
        .strip_prefix(&models_dir)
        .map_err(|e| format!("Failed to compute relative path: {}", e))?;

    // Use forward slashes for URL compatibility
    let relative_str = relative.to_string_lossy().replace('\\', "/");

    Ok(relative_str)
}

/// List all imported Live2D models found under `{app_data_dir}/live2d_models/`.
///
/// Each top-level subdirectory is treated as a separate model. We search for a
/// `.model3.json` file inside each subdirectory.
#[tauri::command]
pub async fn list_live2d_models(app: tauri::AppHandle) -> Result<Vec<Live2dModelInfo>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    let models_dir = app_data.join("live2d_models");

    if !models_dir.exists() {
        return Ok(Vec::new());
    }

    let entries =
        fs::read_dir(&models_dir).map_err(|e| format!("Failed to read models dir: {}", e))?;

    let mut models = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Search for .model3.json inside this model folder
        if let Some(model_json) = find_model3_json(&path) {
            if let Ok(relative) = model_json.strip_prefix(&models_dir) {
                let relative_str = relative.to_string_lossy().replace('\\', "/");
                models.push(Live2dModelInfo {
                    name: folder_name,
                    path: relative_str,
                });
            }
        }
    }

    // Sort by name for consistent ordering
    models.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(models)
}

/// Delete an imported Live2D model by its folder name.
#[tauri::command]
pub async fn delete_live2d_model(app: tauri::AppHandle, model_name: String) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    let model_path = app_data.join("live2d_models").join(&model_name);

    // Security: ensure the path is inside live2d_models
    if !model_path.starts_with(app_data.join("live2d_models")) {
        return Err("Invalid model name".to_string());
    }

    if !model_path.exists() {
        return Err(format!("Model '{}' not found", model_name));
    }

    fs::remove_dir_all(&model_path)
        .map_err(|e| format!("Failed to delete model '{}': {}", model_name, e))?;

    Ok(())
}

/// Import a Live2D model from an extracted folder (by its .model3.json path).
///
/// Finds the model root directory (the folder containing .moc3), copies the
/// entire folder into `{app_data_dir}/live2d_models/{folder_name}/`, and
/// returns the relative path to the .model3.json (same format as zip import).
#[tauri::command]
pub async fn import_live2d_folder(
    app: tauri::AppHandle,
    model_json_path: String,
) -> Result<String, String> {
    let json_path = std::path::Path::new(&model_json_path);
    if !json_path.exists() {
        return Err("model3.json file does not exist".to_string());
    }

    // Walk up from the .model3.json to find the model root (directory containing a .moc3 file)
    let model_root = find_model_root(json_path)
        .ok_or_else(|| "Cannot find model root directory (no .moc3 file found in parent directories)".to_string())?;

    let folder_name = model_root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid folder name".to_string())?
        .to_string();

    // Determine target directory
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    let models_dir = app_data.join("live2d_models");
    let target_dir = models_dir.join(&folder_name);

    // If target already exists, remove it first (re-import scenario)
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)
            .map_err(|e| format!("Failed to remove existing model folder: {}", e))?;
    }

    // Copy the entire model folder
    copy_dir_recursive(&model_root, &target_dir)
        .map_err(|e| format!("Failed to copy model folder: {}", e))?;

    // Validate the copy by finding .model3.json in the target
    let model_json = find_model3_json(&target_dir)
        .ok_or_else(|| "Copied folder does not contain a .model3.json file".to_string())?;

    let relative = model_json
        .strip_prefix(&models_dir)
        .map_err(|e| format!("Failed to compute relative path: {}", e))?;

    let relative_str = relative.to_string_lossy().replace('\\', "/");

    Ok(relative_str)
}

/// Walk up from a .model3.json file to find the model root directory.
/// The root is the directory (or an ancestor) that contains a .moc3 file.
fn find_model_root(model_json: &std::path::Path) -> Option<PathBuf> {
    let mut dir = model_json.parent()?;
    loop {
        if dir_contains_moc3(dir) {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Check if a directory directly contains a .moc3 file.
fn dir_contains_moc3(dir: &std::path::Path) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".moc3") {
                    return true;
                }
            }
        }
    }
    false
}

/// Recursively copy a directory and all its contents.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn find_model3_json(dir: &std::path::Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".model3.json") {
                    return Some(path);
                }
            }
        } else if path.is_dir() {
            dirs.push(path);
        }
    }

    // Recurse into subdirectories
    for sub in dirs {
        if let Some(found) = find_model3_json(&sub) {
            return Some(found);
        }
    }

    None
}
