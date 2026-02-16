use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Theme definition loaded from a MOD's theme.json.
/// Maps directly to the frontend ThemeConfig structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub animations: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModThemeAssets {
    pub fonts: Option<Vec<String>>,
    pub background: Option<String>,
    pub noise_texture: Option<String>,
}
