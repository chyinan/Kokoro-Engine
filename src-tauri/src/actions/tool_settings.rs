// pattern: Functional Core
use crate::actions::registry::{ActionPermissionLevel, ActionRiskTag};
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
    #[serde(default = "default_max_permission_level")]
    pub max_permission_level: ActionPermissionLevel,
    #[serde(default)]
    pub blocked_risk_tags: Vec<ActionRiskTag>,
}

fn default_max_tool_rounds() -> usize {
    DEFAULT_MAX_TOOL_ROUNDS
}

fn default_max_permission_level() -> ActionPermissionLevel {
    ActionPermissionLevel::Elevated
}

fn dedupe_risk_tags(tags: &mut Vec<ActionRiskTag>) {
    let mut deduped = Vec::with_capacity(tags.len());
    for tag in tags.drain(..) {
        if !deduped.contains(&tag) {
            deduped.push(tag);
        }
    }
    *tags = deduped;
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_tool_rounds(),
            enabled_tools: HashMap::new(),
            max_permission_level: default_max_permission_level(),
            blocked_risk_tags: Vec::new(),
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
        dedupe_risk_tags(&mut self.blocked_risk_tags);
        self
    }
}

pub fn load_config(path: &Path) -> ToolSettings {
    config::load_json_config::<ToolSettings>(path, "TOOLS").sanitized()
}

pub fn save_config(path: &Path, config: &ToolSettings) -> Result<(), KokoroError> {
    config::save_json_config(path, config, "TOOLS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::registry::{ActionPermissionLevel, ActionRiskTag};

    #[test]
    fn tool_settings_defaults_include_policy_fields() {
        let settings = ToolSettings::default();

        assert_eq!(settings.max_tool_rounds, DEFAULT_MAX_TOOL_ROUNDS);
        assert_eq!(settings.max_permission_level, ActionPermissionLevel::Elevated);
        assert!(settings.blocked_risk_tags.is_empty());
        assert!(settings.enabled_tools.is_empty());
    }

    #[test]
    fn tool_settings_sanitized_preserves_policy_fields() {
        let settings = ToolSettings {
            max_tool_rounds: 99,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: vec![
                ActionRiskTag::Write,
                ActionRiskTag::Write,
                ActionRiskTag::Sensitive,
            ],
        }
        .sanitized();

        assert_eq!(settings.max_tool_rounds, 20);
        assert_eq!(settings.max_permission_level, ActionPermissionLevel::Safe);
        assert_eq!(
            settings.blocked_risk_tags,
            vec![ActionRiskTag::Write, ActionRiskTag::Sensitive]
        );
    }
}
