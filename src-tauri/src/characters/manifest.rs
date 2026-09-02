// pattern: Functional Core

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};
use thiserror::Error;

pub const CHARACTER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("invalid character manifest JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unsupported character schema version {0}")]
    UnsupportedSchema(u32),
    #[error("missing required manifest field `{0}`")]
    MissingRequiredField(&'static str),
    #[error("invalid character id `{0}`")]
    InvalidId(String),
    #[error("invalid character version `{value}`: {source}")]
    InvalidVersion {
        value: String,
        source: semver::Error,
    },
    #[error("invalid engine version range `{value}`: {source}")]
    InvalidEngineRange {
        value: String,
        source: semver::Error,
    },
    #[error(
        "character requires engine `{required}`, but current engine is `{actual}` (incompatible)"
    )]
    IncompatibleEngine { required: String, actual: Version },
    #[error("unsafe package path `{0}`")]
    UnsafePath(String),
    #[error("unsupported asset path `{0}`")]
    UnsupportedAsset(String),
    #[error("invalid runtime value for `{0}`")]
    InvalidRuntime(&'static str),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterTemplateManifest {
    pub schema_version: u32,
    pub engine_version: String,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub license: String,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    pub persona: String,
    pub greeting: String,
    #[serde(default)]
    pub example_dialogue: Option<String>,
    #[serde(default)]
    pub assets: Option<CharacterAssets>,
    #[serde(default)]
    pub runtime: Option<CharacterRuntimeProfile>,
    #[serde(default)]
    pub recommendations: Option<CharacterRecommendations>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterAssets {
    #[serde(default)]
    pub live2d_model: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub cue_profile: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterRuntimeProfile {
    #[serde(default)]
    pub live2d_model: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub cue_profile: Option<String>,
    #[serde(default)]
    pub tts: Option<CharacterTtsProfile>,
    #[serde(default)]
    pub response_language: Option<String>,
    #[serde(default)]
    pub proactive_enabled: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterTtsProfile {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider_type: Option<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub local_preset: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub speed: Option<f64>,
    #[serde(default)]
    pub pitch: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterRecommendations {
    #[serde(default)]
    pub vision: Option<bool>,
    #[serde(default)]
    pub memory: Option<bool>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub bot_platforms: Option<Vec<String>>,
}

impl CharacterTemplateManifest {
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != CHARACTER_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema(self.schema_version));
        }

        for (field, value) in [
            ("engine_version", self.engine_version.as_str()),
            ("id", self.id.as_str()),
            ("version", self.version.as_str()),
            ("name", self.name.as_str()),
            ("description", self.description.as_str()),
            ("author", self.author.as_str()),
            ("license", self.license.as_str()),
            ("persona", self.persona.as_str()),
            ("greeting", self.greeting.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ManifestError::MissingRequiredField(field));
            }
        }

        if !is_valid_character_id(&self.id) {
            return Err(ManifestError::InvalidId(self.id.clone()));
        }

        Version::parse(&self.version).map_err(|source| ManifestError::InvalidVersion {
            value: self.version.clone(),
            source,
        })?;
        VersionReq::parse(&self.engine_version).map_err(|source| {
            ManifestError::InvalidEngineRange {
                value: self.engine_version.clone(),
                source,
            }
        })?;

        if let Some(path) = &self.avatar {
            validate_asset(path, is_supported_image)?;
        }
        if let Some(assets) = &self.assets {
            if let Some(path) = &assets.live2d_model {
                validate_asset(path, |candidate| {
                    candidate.to_ascii_lowercase().ends_with(".model3.json")
                })?;
            }
            if let Some(path) = &assets.background {
                validate_asset(path, is_supported_image)?;
            }
            if let Some(path) = &assets.cue_profile {
                validate_asset(path, |candidate| {
                    candidate.to_ascii_lowercase().ends_with(".json")
                })?;
            }
        }

        if let Some(runtime) = &self.runtime {
            validate_optional_text("runtime.response_language", &runtime.response_language)?;
            if let Some(tts) = &runtime.tts {
                validate_optional_text("runtime.tts.provider_type", &tts.provider_type)?;
                validate_optional_text("runtime.tts.provider_id", &tts.provider_id)?;
                validate_identifier("runtime.tts.local_preset", &tts.local_preset)?;
                validate_optional_text("runtime.tts.voice", &tts.voice)?;
                if tts.speed.is_some_and(|value| !value.is_finite()) {
                    return Err(ManifestError::InvalidRuntime("runtime.tts.speed"));
                }
                if tts.pitch.is_some_and(|value| !value.is_finite()) {
                    return Err(ManifestError::InvalidRuntime("runtime.tts.pitch"));
                }
            }
        }

        Ok(())
    }

    pub fn validate_for_engine(&self, engine: &Version) -> Result<(), ManifestError> {
        self.validate()?;
        let requirement = VersionReq::parse(&self.engine_version).map_err(|source| {
            ManifestError::InvalidEngineRange {
                value: self.engine_version.clone(),
                source,
            }
        })?;
        if requirement.matches(engine) {
            Ok(())
        } else {
            Err(ManifestError::IncompatibleEngine {
                required: self.engine_version.clone(),
                actual: engine.clone(),
            })
        }
    }
}

pub fn validate_package_path(path: &Path) -> Result<(), ManifestError> {
    let raw = path.to_string_lossy();
    if raw.is_empty() || raw.contains('\\') || path.is_absolute() {
        return Err(ManifestError::UnsafePath(raw.into_owned()));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ManifestError::UnsafePath(raw.into_owned()));
    }
    Ok(())
}

pub fn is_supported_package_file(path: &Path) -> bool {
    if validate_package_path(path).is_err() {
        return false;
    }

    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if normalized == "character.json"
        || file_name == "cues.json"
        || file_name == "license"
        || file_name.starts_with("license.")
    {
        return true;
    }

    [
        ".png",
        ".jpg",
        ".jpeg",
        ".webp",
        ".wav",
        ".ogg",
        ".mp3",
        ".moc3",
        ".model3.json",
        ".motion3.json",
        ".physics3.json",
        ".pose3.json",
        ".exp3.json",
        ".userdata3.json",
    ]
    .iter()
    .any(|extension| normalized.ends_with(extension))
}

fn validate_asset(path: &str, supports_role: impl Fn(&str) -> bool) -> Result<(), ManifestError> {
    validate_package_path(Path::new(path))?;
    if !supports_role(path) || !is_supported_package_file(Path::new(path)) {
        return Err(ManifestError::UnsupportedAsset(path.to_string()));
    }
    Ok(())
}

fn is_supported_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn validate_optional_text(
    field: &'static str,
    value: &Option<String>,
) -> Result<(), ManifestError> {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        Err(ManifestError::InvalidRuntime(field))
    } else {
        Ok(())
    }
}

fn validate_identifier(field: &'static str, value: &Option<String>) -> Result<(), ManifestError> {
    validate_optional_text(field, value)?;
    if value.as_ref().is_some_and(|value| {
        value.contains('/') || value.contains('\\') || value.contains(':') || value.contains("..")
    }) {
        Err(ManifestError::InvalidRuntime(field))
    } else {
        Ok(())
    }
}

fn is_valid_character_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use semver::Version;
    use serde_json::{json, Value};
    use std::path::Path;

    fn valid_manifest_json() -> Value {
        json!({
            "schema_version": 1,
            "engine_version": ">=0.3.0, <0.4.0",
            "id": "kokoro",
            "version": "1.0.0",
            "name": "Kokoro",
            "description": "A warm daily companion",
            "author": "Kokoro Project",
            "license": "CC-BY-4.0",
            "locale": "en",
            "avatar": "avatar.webp",
            "persona": "You are Kokoro.",
            "greeting": "Hello!",
            "example_dialogue": "User: Hi\nKokoro: Hello!",
            "assets": {
                "live2d_model": "live2d/kokoro.model3.json",
                "background": "background.webp",
                "cue_profile": "cues.json"
            },
            "runtime": {
                "tts": {
                    "provider_type": "edge",
                    "provider_id": "configured-edge",
                    "local_preset": "edge-default",
                    "voice": "en-US-AriaNeural",
                    "speed": 1.0,
                    "pitch": 0.0
                },
                "response_language": "en",
                "proactive_enabled": true
            },
            "recommendations": {
                "vision": false,
                "memory": true,
                "mcp_servers": ["calendar"]
            }
        })
    }

    #[test]
    fn rejects_missing_required_fields() {
        let mut value = valid_manifest_json();
        value.as_object_mut().unwrap().remove("greeting");

        let error = CharacterTemplateManifest::from_json(&value.to_string()).unwrap_err();

        assert!(error.to_string().contains("greeting"));
    }

    #[test]
    fn enforces_schema_and_semantic_engine_versions() {
        let manifest =
            CharacterTemplateManifest::from_json(&valid_manifest_json().to_string()).unwrap();

        assert!(manifest
            .validate_for_engine(&Version::parse("0.3.1").unwrap())
            .is_ok());
        assert!(manifest
            .validate_for_engine(&Version::parse("0.4.0").unwrap())
            .unwrap_err()
            .to_string()
            .contains("incompatible"));

        let mut invalid_schema = valid_manifest_json();
        invalid_schema["schema_version"] = json!(2);
        assert!(CharacterTemplateManifest::from_json(&invalid_schema.to_string()).is_err());

        let mut invalid_range = valid_manifest_json();
        invalid_range["engine_version"] = json!("not-a-range");
        assert!(CharacterTemplateManifest::from_json(&invalid_range.to_string()).is_err());
    }

    #[test]
    fn rejects_unsafe_asset_paths() {
        for unsafe_path in [
            "../avatar.webp",
            "live2d/../../secret.txt",
            "/tmp/avatar.webp",
            "C:\\secret\\avatar.webp",
            "live2d\\avatar.webp",
            "",
        ] {
            let mut value = valid_manifest_json();
            value["avatar"] = json!(unsafe_path);
            assert!(
                CharacterTemplateManifest::from_json(&value.to_string()).is_err(),
                "path should be rejected: {unsafe_path}"
            );
        }
    }

    #[test]
    fn rejects_secrets_and_custom_endpoints() {
        for (field, value) in [
            ("api_key", "secret"),
            ("token", "secret"),
            ("base_url", "https://example.com"),
            ("endpoint", "http://127.0.0.1:1234"),
            ("model_path", "C:/models/voice.bin"),
        ] {
            let mut manifest = valid_manifest_json();
            manifest["runtime"]["tts"][field] = json!(value);
            assert!(
                CharacterTemplateManifest::from_json(&manifest.to_string()).is_err(),
                "sensitive field should be rejected: {field}"
            );
        }
    }

    #[test]
    fn accepts_only_declarative_package_content() {
        for allowed in [
            "character.json",
            "LICENSE.md",
            "avatar.png",
            "background.webp",
            "cues.json",
            "live2d/kokoro.model3.json",
            "live2d/kokoro.moc3",
            "live2d/textures/texture_00.png",
            "live2d/motions/idle.motion3.json",
        ] {
            assert!(is_supported_package_file(Path::new(allowed)), "{allowed}");
        }

        for denied in [
            "script.js",
            "component.html",
            "program.exe",
            "archive.zip",
            "live2d/readme.rs",
            "other.json",
        ] {
            assert!(!is_supported_package_file(Path::new(denied)), "{denied}");
        }
    }

    proptest! {
        #[test]
        fn safe_relative_paths_remain_inside_package(
            segments in prop::collection::vec("[a-zA-Z0-9_-]{1,16}", 1..6),
            extension in prop_oneof![Just("png"), Just("webp"), Just("json"), Just("moc3")],
        ) {
            let joined = segments.join("/");
            let relative_str = format!("{joined}.{extension}");
            let relative = Path::new(&relative_str);

            prop_assert!(validate_package_path(relative).is_ok());
            let root = Path::new("package-root");
            prop_assert!(root.join(relative).starts_with(root));
        }
    }
}
