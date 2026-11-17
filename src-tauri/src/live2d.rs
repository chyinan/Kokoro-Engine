//! Live2D model profiles and active desktop model selection.
mod library;

pub use library::Live2dModelInfo;

use crate::characters::activation::{BackendRuntimeSnapshot, PackageAssetReference};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub const BUILTIN_LIVE2D_MODEL_PATH: &str = "__builtin__/hiyori/hiyori_pro_t11.model3.json";
const LEGACY_BUILTIN_HARU_MODEL_PATH: &str = "__builtin__/haru/haru_greeter_t03.model3.json";

pub(crate) type Live2dCueMap = HashMap<String, Live2dCueBinding>;

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

pub(crate) struct Live2dStore {
    app_data: PathBuf,
    active_model_state_path: PathBuf,
}

impl Live2dStore {
    pub(crate) fn new(app_data: PathBuf) -> Self {
        let active_model_state_path = app_data.join("active_live2d_model.json");
        Self {
            app_data,
            active_model_state_path,
        }
    }

    // The desktop adapter can provide the existing selection file separately from
    // the model library directory resolved by Tauri.
    pub(crate) fn with_active_model_state_path(mut self, path: PathBuf) -> Self {
        self.active_model_state_path = path;
        self
    }

    fn models_dir(&self) -> PathBuf {
        self.app_data.join("live2d_models")
    }

    fn load_active_model_path(&self) -> Option<String> {
        let content = fs::read_to_string(&self.active_model_state_path).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        value.get("model_path")?.as_str().map(str::to_owned)
    }

    pub(crate) fn active_profile(&self) -> Option<Live2dModelProfile> {
        let model_path = self.load_active_model_path()?;
        self.profile(&model_path).ok()
    }

    pub(crate) fn profile(&self, model_path: &str) -> Result<Live2dModelProfile, String> {
        ensure_profile_for_model(&self.models_dir(), model_path)
    }

    pub(crate) fn save_profile(
        &self,
        profile: Live2dModelProfile,
    ) -> Result<Live2dModelProfile, String> {
        let models_dir = self.models_dir();
        let discovered = discover_model_profile(&models_dir, &profile.model_path)?;
        let merged = Live2dModelProfile {
            version: 3,
            model_path: discovered.model_path,
            available_expressions: discovered.available_expressions,
            available_motion_groups: discovered.available_motion_groups,
            available_hit_areas: discovered.available_hit_areas,
            cue_map: profile.cue_map,
            semantic_cue_map: normalize_semantic_map(profile.semantic_cue_map),
        };
        save_model_profile(&models_dir, &merged)?;
        Ok(merged)
    }

    pub(crate) fn set_active_model(&self, model_path: Option<&str>) -> Result<(), String> {
        fs::create_dir_all(&self.app_data)
            .map_err(|error| format!("Failed to create app data dir: {error}"))?;
        let normalized = model_path.map(normalize_relative_model_path).transpose()?;
        let content = serde_json::json!({ "model_path": normalized });
        fs::write(&self.active_model_state_path, content.to_string())
            .map_err(|error| format!("Failed to persist active Live2D model: {error}"))
    }

    /// An explicit role asset owns its cue map, including a missing or empty map.
    pub(crate) fn runtime_cue_map(&self, runtime: &BackendRuntimeSnapshot) -> Option<Live2dCueMap> {
        if let Some(reference) = &runtime.cue_profile {
            let path = match reference {
                PackageAssetReference::Package { path, .. } => path,
                PackageAssetReference::Library { model_id } => model_id,
            };
            return Some(
                fs::read_to_string(path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|value| serde_json::from_value(value["cues"].clone()).ok())
                    .unwrap_or_default(),
            );
        }
        match &runtime.live2d_model {
            Some(PackageAssetReference::Library { model_id }) => Some(
                self.profile(model_id)
                    .map(|profile| profile.cue_map)
                    .unwrap_or_default(),
            ),
            Some(PackageAssetReference::Package { .. }) => Some(HashMap::new()),
            None => None,
        }
    }
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

fn discover_model_profile(
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

fn load_saved_model_profile(
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
    fs::write(&profile_path, serialized).map_err(|e| {
        format!(
            "Failed to write model profile '{}': {}",
            profile_path.display(),
            e
        )
    })?;
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

    let normalized = parts.join("/");
    if normalized == LEGACY_BUILTIN_HARU_MODEL_PATH {
        return Ok(BUILTIN_LIVE2D_MODEL_PATH.to_string());
    }

    Ok(normalized)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_selection_reads_and_updates_profiles_in_the_injected_root() {
        let root = tempfile::tempdir().unwrap();
        let store = Live2dStore::new(root.path().to_path_buf());
        assert!(store.active_profile().is_none());

        let mut profile = store.profile(LEGACY_BUILTIN_HARU_MODEL_PATH).unwrap();
        profile.available_motion_groups.clear();
        profile.semantic_cue_map = HashMap::from([(" Emotion:Happy ".into(), " smile ".into())]);
        let saved = store.save_profile(profile).unwrap();
        assert_eq!(saved.available_motion_groups.get("Idle"), Some(&3));
        assert_eq!(saved.model_path, BUILTIN_LIVE2D_MODEL_PATH);

        store
            .set_active_model(Some(LEGACY_BUILTIN_HARU_MODEL_PATH))
            .unwrap();
        let active = store.active_profile().unwrap();
        assert_eq!(active.model_path, BUILTIN_LIVE2D_MODEL_PATH);
        assert_eq!(
            active
                .semantic_cue_map
                .get("emotion:happy")
                .map(String::as_str),
            Some("smile")
        );

        store.set_active_model(None).unwrap();
        assert!(store.active_profile().is_none());
        let raw = fs::read_to_string(root.path().join("active_live2d_model.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap(),
            serde_json::json!({ "model_path": null })
        );
    }

    #[test]
    fn an_explicit_role_cue_asset_never_falls_back_to_the_active_model() {
        let root = tempfile::tempdir().unwrap();
        let store = Live2dStore::new(root.path().to_path_buf());
        store
            .set_active_model(Some(BUILTIN_LIVE2D_MODEL_PATH))
            .unwrap();
        assert!(!store.active_profile().unwrap().cue_map.is_empty());
        let mut runtime = BackendRuntimeSnapshot::default();
        assert!(store.runtime_cue_map(&runtime).is_none());

        runtime.live2d_model = Some(PackageAssetReference::Library {
            model_id: BUILTIN_LIVE2D_MODEL_PATH.into(),
        });
        assert!(!store.runtime_cue_map(&runtime).unwrap().is_empty());

        let asset = root.path().join("cues.json");
        runtime.cue_profile = Some(PackageAssetReference::Package {
            template_id: "test-role".into(),
            template_version: "1.0.0".into(),
            path: asset.to_string_lossy().into_owned(),
        });
        assert!(store.runtime_cue_map(&runtime).unwrap().is_empty());

        fs::write(&asset, r#"{"cues":{"role_only":{"expression":"smile"}}}"#).unwrap();
        let cues = store.runtime_cue_map(&runtime).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues["role_only"].expression.as_deref(), Some("smile"));
    }
}
