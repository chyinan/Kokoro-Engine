// pattern: Functional Core

use serde::{Deserialize, Serialize};

pub const QQ_APP_ID_ENV: &str = "QQBOT_APP_ID";
pub const QQ_APP_SECRET_ENV: &str = "QQBOT_APP_SECRET";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct QQBotConfig {
    pub enabled: bool,
    pub app_id: Option<String>,
    pub app_id_env: Option<String>,
    pub app_secret: Option<String>,
    pub app_secret_env: Option<String>,
    pub allow_c2c: bool,
    pub allowed_user_openids: Vec<String>,
    pub allowed_group_openids: Vec<String>,
    pub character_id: Option<String>,
}

impl Default for QQBotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: None,
            app_id_env: Some(QQ_APP_ID_ENV.to_string()),
            app_secret: None,
            app_secret_env: Some(QQ_APP_SECRET_ENV.to_string()),
            allow_c2c: true,
            allowed_user_openids: Vec::new(),
            allowed_group_openids: Vec::new(),
            character_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_disabled_with_official_environment_names() {
        let config = QQBotConfig::default();

        assert!(!config.enabled);
        assert!(config.allow_c2c);
        assert_eq!(config.app_id_env.as_deref(), Some(QQ_APP_ID_ENV));
        assert_eq!(config.app_secret_env.as_deref(), Some(QQ_APP_SECRET_ENV));
    }
}
