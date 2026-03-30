use crate::config;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for EmotionSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub fn load_config(path: &Path) -> EmotionSettings {
    config::load_json_config::<EmotionSettings>(path, "EMOTION")
}

pub fn save_config(path: &Path, config: &EmotionSettings) -> Result<(), KokoroError> {
    config::save_json_config(path, config, "EMOTION")
}
