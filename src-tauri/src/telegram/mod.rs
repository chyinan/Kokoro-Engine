//! Telegram Bot module — lifecycle management and re-exports.

pub mod bot;
pub mod config;

pub use config::{load_config, save_config, TelegramConfig};

use crate::bot::{lifecycle::BotWorker, BotChatServices, BotConfig, BotRuntimeService};
use crate::lifecycle::{TaskScope, WorkLease};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application-owned Telegram polling service.
#[derive(Clone)]
pub struct TelegramService {
    config: Arc<RwLock<BotConfig>>,
    worker: BotWorker,
}

impl TelegramService {
    pub(crate) fn new(runtime: &BotRuntimeService, work: TaskScope, service: TaskScope) -> Self {
        Self {
            config: runtime.shared_config(),
            worker: BotWorker::new("Telegram bot", work, service),
        }
    }

    /// Whether the bot polling loop is currently running.
    pub async fn is_running(&self) -> bool {
        self.worker.is_running().await
    }

    /// Get a snapshot of the current config.
    pub async fn get_config(&self) -> TelegramConfig {
        self.config.read().await.telegram.clone()
    }

    /// Start the bot polling loop.
    pub async fn start(&self, services: BotChatServices) -> Result<(), String> {
        let token = self
            .config
            .read()
            .await
            .telegram
            .resolve_bot_token()
            .ok_or("No bot token configured")?;
        let config = self.config.clone();
        self.worker
            .start(move |ingress| Ok(bot::run_polling(token, config, services, ingress)))
            .await
    }

    pub(crate) fn close_ingress(&self) {
        self.worker.close_ingress();
    }

    pub(crate) fn admit_control(&self, action: &str) -> Result<WorkLease, String> {
        self.worker.admit_control(action)
    }

    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.close_ingress();
        self.stop().await
    }

    /// Stop polling and join the dispatcher after its admitted handlers return.
    pub async fn stop(&self) -> Result<(), String> {
        self.worker.stop().await
    }
}

#[cfg(test)]
impl TelegramService {
    pub(crate) async fn assert_dispatched_text_chats(&self, chat_ids: &[i64], expected: &[i64]) {
        bot::tests::assert_dispatched_text_chats(&self.config, chat_ids, expected).await;
    }
}
