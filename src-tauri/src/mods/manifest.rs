// pattern: Functional Core

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Validation failures for a MOD manifest.  MOD packages are executable
/// content, so validation happens before any archive is extracted into the
/// installed directory.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModManifestError {
    #[error("missing required MOD field `{0}`")]
    MissingField(&'static str),
    #[error("invalid MOD id `{0}`")]
    InvalidId(String),
    #[error("invalid MOD version `{value}`: {reason}")]
    InvalidVersion { value: String, reason: String },
    #[error("invalid MOD engine version range `{value}`: {reason}")]
    InvalidEngineVersion { value: String, reason: String },
    #[error("MOD is incompatible with engine {engine}: {requirement}")]
    IncompatibleEngine { engine: String, requirement: String },
    #[error("unsafe MOD path `{0}`")]
    UnsafePath(String),
    #[error("unsupported MOD permission `{0}`")]
    InvalidPermission(String),
    #[error("invalid MOD capability `{0}`")]
    InvalidCapability(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModCapability {
    pub name: String,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,

    /// Semver constraint for engine compatibility, e.g. "^0.2.0"
    #[serde(default)]
    pub engine_version: Option<String>,

    /// Relative path to layout.json override
    #[serde(default)]
    pub layout: Option<String>,

    /// Relative path to theme.json override
    #[serde(default)]
    pub theme: Option<String>,

    /// Component slot registrations: "SlotName" -> "components/File.html"
    #[serde(default)]
    pub components: HashMap<String, String>,

    /// Script entry points, e.g. ["scripts/main.js"]
    #[serde(default)]
    pub scripts: Vec<String>,

    /// Requested permissions, e.g. ["tts", "system.info"]
    #[serde(default)]
    pub permissions: Vec<String>,

    /// Declarative capabilities for fine-grained intent (minimal model)
    #[serde(default)]
    pub capabilities: Vec<ModCapability>,

    // Legacy fields kept for transition — will be removed
    pub entry: Option<String>,
    pub ui_entry: Option<String>,
}

impl ModManifest {
    /// Validate identity, semantic versions, compatibility, and all paths
    /// before a package can be staged.  The engine range is optional for
    /// legacy local MODs, but if present it is authoritative.
    pub fn validate_for_engine(&self, engine: &Version) -> Result<(), ModManifestError> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("name", self.name.as_str()),
            ("version", self.version.as_str()),
            ("description", self.description.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ModManifestError::MissingField(field));
            }
        }
        if !is_valid_mod_id(&self.id) {
            return Err(ModManifestError::InvalidId(self.id.clone()));
        }
        Version::parse(&self.version).map_err(|error| ModManifestError::InvalidVersion {
            value: self.version.clone(),
            reason: error.to_string(),
        })?;
        if let Some(requirement) = self.engine_version.as_deref() {
            let parsed = VersionReq::parse(requirement).map_err(|error| {
                ModManifestError::InvalidEngineVersion {
                    value: requirement.to_string(),
                    reason: error.to_string(),
                }
            })?;
            if !parsed.matches(engine) {
                return Err(ModManifestError::IncompatibleEngine {
                    engine: engine.to_string(),
                    requirement: requirement.to_string(),
                });
            }
        }

        for path in self
            .layout
            .iter()
            .chain(self.theme.iter())
            .chain(self.components.values())
            .chain(self.scripts.iter())
            .chain(self.entry.iter())
            .chain(self.ui_entry.iter())
        {
            validate_relative_path(path)?;
        }
        for permission in &self.permissions {
            if !is_allowed_permission(permission) {
                return Err(ModManifestError::InvalidPermission(permission.clone()));
            }
        }
        for capability in &self.capabilities {
            if capability.name.trim().is_empty() {
                return Err(ModManifestError::InvalidCapability(capability.name.clone()));
            }
        }
        Ok(())
    }

    /// Return whether the manifest requests permissions that need an explicit
    /// user review before replacing an existing MOD.
    pub fn permission_review_required(&self) -> bool {
        !self.permissions.is_empty()
            || self
                .capabilities
                .iter()
                .any(|capability| capability.requires_confirmation)
    }
}

fn is_valid_mod_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

fn validate_relative_path(path: &str) -> Result<(), ModManifestError> {
    let candidate = Path::new(path);
    if path.trim().is_empty()
        || candidate.is_absolute()
        || path.contains('\\')
        || candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(ModManifestError::UnsafePath(path.to_string()));
    }
    Ok(())
}

fn is_allowed_permission(value: &str) -> bool {
    matches!(
        value,
        "tts"
            | "vision"
            | "memory"
            | "mcp"
            | "bot"
            | "system.info"
            | "system.notifications"
            | "filesystem.read"
            | "clipboard.read"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_manifest() {
        let json = r#"{
            "id": "demo-echo",
            "name": "Demo Echo Mod",
            "version": "0.1.0",
            "description": "A demo mod for testing",
            "engine_version": "^0.2.0",
            "layout": "layout.json",
            "theme": "theme.json",
            "components": { "DemoPanel": "components/DemoPanel.html" },
            "scripts": ["scripts/main.js"],
            "permissions": ["tts"],
            "entry": null,
            "ui_entry": null
        }"#;

        let manifest: ModManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "demo-echo");
        assert_eq!(manifest.name, "Demo Echo Mod");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.engine_version, Some("^0.2.0".to_string()));
        assert_eq!(manifest.layout, Some("layout.json".to_string()));
        assert_eq!(manifest.theme, Some("theme.json".to_string()));
        assert_eq!(manifest.components.len(), 1);
        assert_eq!(
            manifest.components.get("DemoPanel"),
            Some(&"components/DemoPanel.html".to_string())
        );
        assert_eq!(manifest.scripts, vec!["scripts/main.js"]);
        assert_eq!(manifest.permissions, vec!["tts"]);
    }

    #[test]
    fn parse_minimal_manifest() {
        let json = r#"{
            "id": "minimal",
            "name": "Minimal Mod",
            "version": "1.0.0",
            "description": "Just required fields"
        }"#;

        let manifest: ModManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "minimal");
        assert!(manifest.engine_version.is_none());
        assert!(manifest.layout.is_none());
        assert!(manifest.theme.is_none());
        assert!(manifest.components.is_empty());
        assert!(manifest.scripts.is_empty());
        assert!(manifest.permissions.is_empty());
        assert!(manifest.entry.is_none());
    }

    #[test]
    fn missing_required_fields_fails() {
        let json = r#"{ "id": "incomplete" }"#;
        let result = serde_json::from_str::<ModManifest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_with_legacy_entry() {
        let json = r#"{
            "id": "legacy-mod",
            "name": "Legacy",
            "version": "0.1.0",
            "description": "Uses legacy entry field",
            "entry": "main.js",
            "ui_entry": "index.html"
        }"#;

        let manifest: ModManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.entry, Some("main.js".to_string()));
        assert_eq!(manifest.ui_entry, Some("index.html".to_string()));
    }

    #[test]
    fn parse_capabilities_manifest_and_keep_permissions_backward_compatible() {
        let json = r#"{
          "id":"demo",
          "name":"Demo",
          "version":"0.1.0",
          "description":"demo",
          "capabilities":[
            {"name":"tts.speak","risk":"write","requires_confirmation":false},
            {"name":"system.info","risk":"read","requires_confirmation":false}
          ],
          "permissions":["tts"]
        }"#;

        let manifest: ModManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.capabilities.len(), 2);
        assert_eq!(manifest.permissions, vec!["tts"]);
        assert_eq!(manifest.capabilities[0].name, "tts.speak");
        assert_eq!(manifest.capabilities[0].risk.as_deref(), Some("write"));
        assert!(!manifest.capabilities[0].requires_confirmation);
    }
}
