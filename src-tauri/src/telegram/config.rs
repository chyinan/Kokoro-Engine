//! Telegram Bot configuration — load/save from app data directory.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Whether the Telegram bot is enabled (auto-start on app launch).
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (direct value).
    #[serde(default)]
    pub bot_token: Option<String>,
    /// Or read token from this environment variable.
    #[serde(default)]
    pub bot_token_env: Option<String>,
    /// Chat ID whitelist — only these chats can interact with the bot.
    /// Empty list = reject all.
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
    /// Whether to also send a voice message alongside text replies (requires TTS).
    #[serde(default)]
    pub send_voice_reply: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            bot_token_env: Some("TELEGRAM_BOT_TOKEN".to_string()),
            allowed_chat_ids: Vec::new(),
            send_voice_reply: false,
        }
    }
}

impl TelegramConfig {
    /// Resolve the bot token: check direct field first, then env var.
    pub fn resolve_bot_token(&self) -> Option<String> {
        crate::config::resolve_api_key(&self.bot_token, &self.bot_token_env)
    }
}

pub fn load_config(path: &Path) -> TelegramConfig {
    crate::config::load_json_config(path, "TELEGRAM")
}

pub fn save_config(path: &Path, config: &TelegramConfig) -> Result<(), String> {
    crate::config::save_json_config(path, config, "TELEGRAM")
}
