use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
