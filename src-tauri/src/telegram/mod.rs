//! Telegram Bot module — lifecycle management and re-exports.

pub mod bot;
pub mod config;

pub use config::{load_config, save_config, TelegramConfig};

use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

/// Managed Tauri state for the Telegram bot service.
#[derive(Clone)]
pub struct TelegramService {
    config: Arc<RwLock<TelegramConfig>>,
    /// Sender half of the shutdown signal. `Some` = bot is running.
    shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
}

impl TelegramService {
    pub fn new(config: TelegramConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Whether the bot polling loop is currently running.
    pub async fn is_running(&self) -> bool {
        self.shutdown_tx.read().await.is_some()
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

    /// Start the bot polling loop. Returns Err if already running or no token.
    pub async fn start(&self, app: tauri::AppHandle) -> Result<(), String> {
        if self.is_running().await {
            return Err("Telegram bot is already running".to_string());
        }

        let config = self.config.read().await.clone();
        let token = config
            .resolve_bot_token()
            .ok_or("No bot token configured")?;

        let (tx, rx) = oneshot::channel::<()>();
        {
            let mut shutdown = self.shutdown_tx.write().await;
            *shutdown = Some(tx);
        }

        let shutdown_flag = self.shutdown_tx.clone();

        tauri::async_runtime::spawn(async move {
            println!("[Telegram] Bot polling started");
            bot::run_polling(token, config, app, rx).await;
            println!("[Telegram] Bot polling stopped");
            // Clear the shutdown sender so is_running() returns false
            let mut guard = shutdown_flag.write().await;
            *guard = None;
        });

        Ok(())
    }

    /// Stop the bot polling loop gracefully.
    pub async fn stop(&self) -> Result<(), String> {
        let mut shutdown = self.shutdown_tx.write().await;
        if let Some(tx) = shutdown.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("Telegram bot is not running".to_string())
        }
    }
}
