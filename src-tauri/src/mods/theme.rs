use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Theme definition loaded from a MOD's theme.json.
/// Maps directly to the frontend ThemeConfig structure.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModThemeJson {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,

    /// CSS custom properties, e.g. { "--color-accent": "#00ffcc" }
    pub variables: HashMap<String, String>,

    #[serde(default)]
    pub assets: Option<ModThemeAssets>,

    /// Named animation presets for framer-motion
    #[serde(default)]
    #[specta(type = Option<HashMap<String, crate::ipc::wire::JsonValue>>)]
    pub animations: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModThemeAssets {
    #[serde(default)]
    pub fonts: Option<Vec<String>>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub noise_texture: Option<String>,
}
