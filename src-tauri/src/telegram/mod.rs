//! Telegram Bot module — lifecycle management and re-exports.

pub mod bot;
pub mod config;

pub use config::{load_config, save_config, TelegramConfig};

use crate::bot::{lifecycle::BotWorker, BotChatServices};
use crate::lifecycle::{TaskScope, WorkLease};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application-owned Telegram polling service.
#[derive(Clone)]
pub struct TelegramService {
    config: Arc<RwLock<TelegramConfig>>,
    worker: BotWorker,
}

impl TelegramService {
    pub(crate) fn new(config: TelegramConfig, work: TaskScope, service: TaskScope) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            worker: BotWorker::new("Telegram bot", work, service),
        }
    }

    /// Whether the bot polling loop is currently running.
    pub async fn is_running(&self) -> bool {
        self.worker.is_running().await
    }

    /// Get a snapshot of the current config.
    pub async fn get_config(&self) -> TelegramConfig {
        self.config.read().await.clone()
    }

    /// Update the in-memory config (caller is responsible for persisting to disk).
    pub async fn update_config(&self, new_config: TelegramConfig) {
        let mut cfg = self.config.write().await;
        *cfg = new_config;
    }

    /// Start the bot polling loop.
    pub async fn start(&self, services: BotChatServices) -> Result<(), String> {
        let token = self
            .config
            .read()
            .await
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
