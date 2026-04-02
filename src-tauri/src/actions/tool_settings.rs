use crate::config;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_MAX_TOOL_ROUNDS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSettings {
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    #[serde(default)]
    pub enabled_tools: HashMap<String, bool>,
}

fn default_max_tool_rounds() -> usize {
    DEFAULT_MAX_TOOL_ROUNDS
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_tool_rounds(),
            enabled_tools: HashMap::new(),
        }
    }
}

impl ToolSettings {
    pub fn is_enabled(&self, tool_id: &str) -> bool {
        self.enabled_tools.get(tool_id).copied().unwrap_or(true)
    }

    pub fn set_enabled(&mut self, tool_id: String, enabled: bool) {
        self.enabled_tools.insert(tool_id, enabled);
    }

    pub fn sanitized(mut self) -> Self {
        self.max_tool_rounds = self.max_tool_rounds.clamp(1, 20);
        self
    }
}

pub fn load_config(path: &Path) -> ToolSettings {
    config::load_json_config::<ToolSettings>(path, "TOOLS").sanitized()
}

pub fn save_config(path: &Path, config: &ToolSettings) -> Result<(), KokoroError> {
    config::save_json_config(path, config, "TOOLS")
}
