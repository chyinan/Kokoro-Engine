//! Live2D model profiles and active desktop model selection.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub const BUILTIN_LIVE2D_MODEL_PATH: &str = "__builtin__/hiyori/hiyori_pro_t11.model3.json";
const LEGACY_BUILTIN_HARU_MODEL_PATH: &str = "__builtin__/haru/haru_greeter_t03.model3.json";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Live2dCueBinding {
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub motion_group: Option<String>,
    #[serde(default)]
    pub exclude_from_prompt: bool,
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
    pub available_hit_areas: Vec<String>,
    #[serde(default)]
    pub cue_map: HashMap<String, Live2dCueBinding>,
    #[serde(default)]
    pub semantic_cue_map: HashMap<String, String>,
}

pub(crate) fn ensure_profile_for_model(
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
            available_hit_areas: discovered.available_hit_areas.clone(),
            cue_map: saved.cue_map,
            semantic_cue_map: normalize_semantic_map(saved.semantic_cue_map),
        },
        Ok(None) => discovered,
        Err(err) => return Err(err),
    };

    save_model_profile(models_dir, &profile)?;
    Ok(profile)
}

pub(crate) fn discover_model_profile(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<Live2dModelProfile, String> {
    let normalized = normalize_relative_model_path(model_path)?;
    if normalized == BUILTIN_LIVE2D_MODEL_PATH {
        return Ok(builtin_hiyori_profile());
    }
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

    let available_hit_areas = json
        .get("HitAreas")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.get("Name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(Live2dModelProfile {
        version: 3,
        model_path: normalized,
        available_expressions,
        available_motion_groups,
        available_hit_areas,
        cue_map: HashMap::new(),
        semantic_cue_map: HashMap::new(),
    })
}

pub(crate) fn active_model_state_path() -> PathBuf {
    crate::utils::paths::data_dir()
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
    let models_dir = crate::utils::paths::data_dir()
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
    if normalized == BUILTIN_LIVE2D_MODEL_PATH {
        return Ok(models_dir
            .join("__builtin__")
            .join("hiyori")
            .join(".kokoro-live2d-profile.json"));
    }
    let root = normalized
        .split('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| "Invalid model path".to_string())?;
    Ok(models_dir.join(root).join(".kokoro-live2d-profile.json"))
}

fn builtin_hiyori_profile() -> Live2dModelProfile {
    Live2dModelProfile {
        version: 3,
        model_path: BUILTIN_LIVE2D_MODEL_PATH.to_string(),
        available_expressions: Vec::new(),
        available_motion_groups: HashMap::from([
            ("Idle".to_string(), 3usize),
            ("Flick".to_string(), 1usize),
            ("FlickDown".to_string(), 1usize),
            ("FlickUp".to_string(), 1usize),
            ("Tap".to_string(), 2usize),
            ("Tap@Body".to_string(), 1usize),
            ("Flick@Body".to_string(), 1usize),
        ]),
        available_hit_areas: vec!["Body".to_string()],
        cue_map: HashMap::from([
            (
                "惊讶".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("FlickUp".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "害羞".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("FlickDown".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "tap".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("Tap".to_string()),
                    exclude_from_prompt: true,
                },
            ),
            (
                "笑".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("Tap".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "微笑".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("Idle".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "平静".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("Idle".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "悲".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("FlickDown".to_string()),
                    exclude_from_prompt: false,
                },
            ),
            (
                "疑惑".to_string(),
                Live2dCueBinding {
                    expression: None,
                    motion_group: Some("Flick".to_string()),
                    exclude_from_prompt: false,
                },
            ),
        ]),
        semantic_cue_map: HashMap::from([
            ("emotion:very_sad".to_string(), "悲".to_string()),
            ("emotion:sulking".to_string(), "平静".to_string()),
            ("interaction:*_*".to_string(), "tap".to_string()),
            ("emotion:mood_swing".to_string(), "疑惑".to_string()),
            ("emotion:ecstatic".to_string(), "笑".to_string()),
            ("emotion:very_happy".to_string(), "微笑".to_string()),
        ]),
    }
}

pub(crate) fn load_saved_model_profile(
    models_dir: &std::path::Path,
    model_path: &str,
) -> Result<Option<Live2dModelProfile>, String> {
    let profile_path = profile_path_for_model(models_dir, model_path)?;
    if !profile_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&profile_path).map_err(|e| {
        format!(
            "Failed to read model profile '{}': {}",
            profile_path.display(),
            e
        )
    })?;
    let profile = serde_json::from_str::<Live2dModelProfile>(&content).map_err(|e| {
        format!(
            "Failed to parse model profile '{}': {}",
            profile_path.display(),
            e
        )
    })?;
    Ok(Some(profile))
}

pub(crate) fn save_model_profile(
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
    fs::write(&profile_path, serialized).map_err(|e| {
        format!(
            "Failed to write model profile '{}': {}",
            profile_path.display(),
            e
        )
    })?;
    Ok(())
}

pub(crate) fn normalize_relative_model_path(model_path: &str) -> Result<String, String> {
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

    let normalized = parts.join("/");
    if normalized == LEGACY_BUILTIN_HARU_MODEL_PATH {
        return Ok(BUILTIN_LIVE2D_MODEL_PATH.to_string());
    }

    Ok(normalized)
}

pub(crate) fn normalize_semantic_map(map: HashMap<String, String>) -> HashMap<String, String> {
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

pub(crate) fn normalize_semantic_key(raw_key: &str) -> String {
    raw_key.trim().to_lowercase()
}
