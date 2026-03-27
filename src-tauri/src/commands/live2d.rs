use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use tauri::{Emitter, Manager};

#[derive(Debug, Serialize)]
pub struct Live2dModelInfo {
    /// Human-friendly name (top-level folder name)
    pub name: String,
    /// Relative path to the .model3.json (used for protocol URL)
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Live2dCueBinding {
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub motion_group: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Live2dModelProfile {
    pub version: u32,
    pub model_path: String,
    #[serde(default)]
    pub available_expressions: Vec<String>,
    #[serde(default)]
    pub available_motion_groups: HashMap<String, usize>,
    #[serde(default)]
    pub cue_map: HashMap<String, Live2dCueBinding>,
    #[serde(default)]
    pub semantic_cue_map: HashMap<String, String>,
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
pub async fn import_live2d_zip(app: tauri::AppHandle, zip_path: String) -> Result<String, KokoroError> {
    let archive_path = std::path::Path::new(&zip_path);
    if !archive_path.exists() {
        return Err(KokoroError::NotFound("Zip file does not exist".to_string()));
    }

    let app_data = app.path().app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Cannot resolve app data dir: {}", e)))?;
    let models_dir = app_data.join("live2d_models");
    fs::create_dir_all(&models_dir).map_err(KokoroError::from)?;

    let file = fs::File::open(archive_path).map_err(KokoroError::from)?;
    let mut archive = zip::ZipArchive::new(file).map_err(KokoroError::from)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(KokoroError::from)?;
        let outpath = match entry.enclosed_name() {
            Some(path) => models_dir.join(path),
            None => continue,
        };
        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(KokoroError::from)?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(KokoroError::from)?;
                }
            }
            let mut outfile = fs::File::create(&outpath).map_err(KokoroError::from)?;
            io::copy(&mut entry, &mut outfile).map_err(KokoroError::from)?;
        }
    }

    let model_json = find_model3_json(&models_dir)
        .ok_or_else(|| KokoroError::NotFound("No .model3.json file found in the zip archive".to_string()))?;

    let relative = model_json.strip_prefix(&models_dir)
        .map_err(|e| KokoroError::Internal(format!("Failed to compute relative path: {}", e)))?;

    // Use forward slashes for URL compatibility
    let relative_str = relative.to_string_lossy().replace('\\', "/");

    ensure_profile_for_model(&models_dir, &relative_str)?;

    Ok(relative_str)
}

/// List all imported Live2D models found under `{app_data_dir}/live2d_models/`.
///
/// Each top-level subdirectory is treated as a separate model. We search for a
/// `.model3.json` file inside each subdirectory.
#[tauri::command]
pub async fn list_live2d_models(app: tauri::AppHandle) -> Result<Vec<Live2dModelInfo>, KokoroError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Cannot resolve app data dir: {}", e)))?;
    let models_dir = app_data.join("live2d_models");

    if !models_dir.exists() {
        return Ok(Vec::new());
    }

    let entries =
        fs::read_dir(&models_dir).map_err(|e| KokoroError::Internal(format!("Failed to read models dir: {}", e)))?;

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
pub async fn delete_live2d_model(app: tauri::AppHandle, model_name: String) -> Result<(), KokoroError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Cannot resolve app data dir: {}", e)))?;
    let model_path = app_data.join("live2d_models").join(&model_name);

    // Security: ensure the path is inside live2d_models
    if !model_path.starts_with(app_data.join("live2d_models")) {
        return Err(KokoroError::Validation("Invalid model name".to_string()));
    }

    if !model_path.exists() {
        return Err(KokoroError::NotFound(format!("Model '{}' not found", model_name)));
    }

    fs::remove_dir_all(&model_path)
        .map_err(|e| KokoroError::Internal(format!("Failed to delete model '{}': {}", model_name, e)))?;

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
) -> Result<String, KokoroError> {
    let json_path = std::path::Path::new(&model_json_path);
    if !json_path.exists() {
        return Err(KokoroError::NotFound("model3.json file does not exist".to_string()));
    }

    // Walk up from the .model3.json to find the model root (directory containing a .moc3 file)
    let model_root = find_model_root(json_path)
        .ok_or_else(|| KokoroError::NotFound("Cannot find model root directory (no .moc3 file found in parent directories)".to_string()))?;

    let folder_name = model_root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| KokoroError::Validation("Invalid folder name".to_string()))?
        .to_string();

    // Determine target directory
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Cannot resolve app data dir: {}", e)))?;
    let models_dir = app_data.join("live2d_models");
    let target_dir = models_dir.join(&folder_name);

    // If target already exists, remove it first (re-import scenario)
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)
            .map_err(KokoroError::from)?;
    }

    // Copy the entire model folder
    copy_dir_recursive(&model_root, &target_dir)
        .map_err(|e| KokoroError::Internal(format!("Failed to copy model folder: {}", e)))?;

    // Validate the copy by finding .model3.json in the target
    let model_json = find_model3_json(&target_dir)
        .ok_or_else(|| KokoroError::NotFound("Copied folder does not contain a .model3.json file".to_string()))?;

    let relative = model_json
        .strip_prefix(&models_dir)
        .map_err(|e| KokoroError::Internal(format!("Failed to compute relative path: {}", e)))?;

    let relative_str = relative.to_string_lossy().replace('\\', "/");

    ensure_profile_for_model(&models_dir, &relative_str)?;

    Ok(relative_str)
}

#[tauri::command]
pub async fn get_live2d_model_profile(
    app: tauri::AppHandle,
    model_path: String,
) -> Result<Live2dModelProfile, String> {
    let models_dir = get_models_dir(&app)?;
    ensure_profile_for_model(&models_dir, &model_path)
}

#[tauri::command]
pub async fn save_live2d_model_profile(
    app: tauri::AppHandle,
    profile: Live2dModelProfile,
) -> Result<Live2dModelProfile, String> {
    let models_dir = get_models_dir(&app)?;
    let discovered = discover_model_profile(&models_dir, &profile.model_path)?;
    let merged = Live2dModelProfile {
        version: 3,
        model_path: discovered.model_path,
        available_expressions: discovered.available_expressions,
        available_motion_groups: discovered.available_motion_groups,
        cue_map: profile.cue_map,
        semantic_cue_map: normalize_semantic_map(profile.semantic_cue_map),
    };

    save_model_profile(&models_dir, &merged)?;
    let _ = app.emit("live2d-profile-updated", &merged);
    Ok(merged)
}

#[tauri::command]
pub async fn set_active_live2d_model(
    app: tauri::AppHandle,
    model_path: Option<String>,
) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    fs::create_dir_all(&app_data).map_err(|e| format!("Failed to create app data dir: {}", e))?;

    let normalized = match model_path {
        Some(path) => Some(normalize_relative_model_path(&path)?),
        None => None,
    };

    let content = serde_json::json!({ "model_path": normalized });
    fs::write(active_model_state_path(), content.to_string())
        .map_err(|e| format!("Failed to persist active live2d model: {}", e))?;
    Ok(())
}

fn get_models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {}", e))?;
    Ok(app_data.join("live2d_models"))
}

fn ensure_profile_for_model(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<Live2dModelProfile, String> {
    let discovered = discover_model_profile(models_dir, model_path)?;
    let profile = match load_saved_model_profile(models_dir, model_path) {
        Ok(Some(saved)) => Live2dModelProfile {
            version: 3,
            model_path: discovered.model_path.clone(),
            available_expressions: discovered.available_expressions.clone(),
            available_motion_groups: discovered.available_motion_groups.clone(),
            cue_map: saved.cue_map,
            semantic_cue_map: normalize_semantic_map(saved.semantic_cue_map),
        },
        Ok(None) => discovered,
        Err(err) => return Err(err),
    };

    save_model_profile(models_dir, &profile)?;
    Ok(profile)
}

fn discover_model_profile(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<Live2dModelProfile, String> {
    let normalized = normalize_relative_model_path(model_path)?;
    let model_json_path = models_dir.join(&normalized);
    if !model_json_path.exists() {
        return Err(format!("Model '{}' not found", normalized));
    }

    let content = fs::read_to_string(&model_json_path)
        .map_err(|e| format!("Failed to read model json '{}': {}", normalized, e))?;
    let json: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse model json '{}': {}", normalized, e))?;

    let available_expressions = json
        .get("FileReferences")
        .and_then(|v| v.get("Expressions"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(read_expression_name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let available_motion_groups = json
        .get("FileReferences")
        .and_then(|v| v.get("Motions"))
        .and_then(Value::as_object)
        .map(|groups| {
            groups
                .iter()
                .map(|(name, motions)| {
                    let count = motions.as_array().map(|arr| arr.len()).unwrap_or(0);
                    (name.clone(), count)
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(Live2dModelProfile {
        version: 3,
        model_path: normalized,
        available_expressions,
        available_motion_groups,
        cue_map: HashMap::new(),
        semantic_cue_map: HashMap::new(),
    })
}

fn active_model_state_path() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("active_live2d_model.json")
}

pub fn load_active_live2d_model_path() -> Option<String> {
    let path = active_model_state_path();
    let content = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&content).ok()?;
    value.get("model_path")?.as_str().map(|s| s.to_string())
}

pub fn load_active_live2d_profile() -> Option<Live2dModelProfile> {
    let model_path = load_active_live2d_model_path()?;
    let models_dir = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("live2d_models");
    ensure_profile_for_model(&models_dir, &model_path).ok()
}

fn read_expression_name(value: &Value) -> Option<String> {
    if let Some(name) = value.get("Name").and_then(Value::as_str) {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(name) = value.get("name").and_then(Value::as_str) {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let file = value
        .get("File")
        .or_else(|| value.get("file"))
        .and_then(Value::as_str)?;
    let stem = std::path::Path::new(file).file_stem()?.to_str()?.trim();
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

fn profile_path_for_model(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<PathBuf, String> {
    let normalized = normalize_relative_model_path(model_path)?;
    let root = normalized
        .split('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| "Invalid model path".to_string())?;
    Ok(models_dir.join(root).join(".kokoro-live2d-profile.json"))
}

fn load_saved_model_profile(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<Option<Live2dModelProfile>, String> {
    let profile_path = profile_path_for_model(models_dir, model_path)?;
    if !profile_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&profile_path)
        .map_err(|e| format!("Failed to read model profile '{}': {}", profile_path.display(), e))?;
    let profile = serde_json::from_str::<Live2dModelProfile>(&content)
        .map_err(|e| format!("Failed to parse model profile '{}': {}", profile_path.display(), e))?;
    Ok(Some(profile))
}

fn save_model_profile(
    models_dir: &std::path::Path,
    profile: &Live2dModelProfile,
) -> Result<(), String> {
    let profile_path = profile_path_for_model(models_dir, &profile.model_path)?;
    if let Some(parent) = profile_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create profile dir '{}': {}", parent.display(), e))?;
    }

    let serialized = serde_json::to_string_pretty(profile)
        .map_err(|e| format!("Failed to serialize model profile: {}", e))?;
    fs::write(&profile_path, serialized)
        .map_err(|e| format!("Failed to write model profile '{}': {}", profile_path.display(), e))?;
    Ok(())
}

fn normalize_relative_model_path(model_path: &str) -> Result<String, String> {
    let path = std::path::Path::new(model_path);
    if path.is_absolute() {
        return Err("Absolute model paths are not allowed".to_string());
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                parts.push(part.to_string_lossy().to_string());
            }
            std::path::Component::CurDir => {}
            _ => return Err("Invalid model path".to_string()),
        }
    }

    if parts.is_empty() {
        return Err("Invalid model path".to_string());
    }

    Ok(parts.join("/"))
}

fn normalize_semantic_map(map: HashMap<String, String>) -> HashMap<String, String> {
    map.into_iter()
        .filter_map(|(raw_key, raw_cue)| {
            let cue = raw_cue.trim();
            if cue.is_empty() {
                return None;
            }

            Some((normalize_semantic_key(&raw_key), cue.to_string()))
        })
        .collect()
}

fn normalize_semantic_key(raw_key: &str) -> String {
    raw_key.trim().to_lowercase()
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
