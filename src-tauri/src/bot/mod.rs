//! External platform configuration, lifecycle, and chat adapters.

use crate::ai::context::AIOrchestrator;
use crate::chat::application::{ChatAudioInput, ChatPolicy, ChatRequest, ChatService};
#[cfg(test)]
use crate::chat::persistence::ExternalTurn;
use crate::chat::persistence::{self, ExternalReplyImage as BotReplyImage};
use crate::error::KokoroError;
use crate::imagegen::{ImageGenResult, ImageGenService};
use crate::stt::{AudioSource, SttService};
use crate::telegram::TelegramService;
use crate::tts::TtsService;
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use warp::{http::StatusCode, Filter, Reply};

type HmacSha256 = Hmac<Sha256>;

/// Notifications from external platform workflows, delivered by the application host.
pub(crate) enum BotEvent {
    QqAuthorizationRequested(QqAuthorizationPrompt),
    QqAuthorizationExpired(QqAuthorizationPrompt),
    ImageGenerated {
        result: ImageGenResult,
        character_id: String,
        conversation_id: String,
        client_request_id: String,
    },
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub(crate) struct QqAuthorizationPrompt {
    pub request_id: String,
    pub user_openid: String,
    pub conversation_kind: String,
    pub group_openid: Option<String>,
}

pub(crate) trait BotEvents: Send + Sync {
    fn emit(&self, event: BotEvent) -> Result<(), String>;
}

impl<F> BotEvents for F
where
    F: Fn(BotEvent) -> Result<(), String> + Send + Sync,
{
    fn emit(&self, event: BotEvent) -> Result<(), String> {
        self(event)
    }
}

/// Application-owned services shared by all external platform workers.
#[derive(Clone)]
pub struct BotChatServices {
    chat: Option<BotChatAccess>,
    pub(crate) stt: Option<SttService>,
    pub(crate) tts: Option<TtsService>,
    imagegen: Option<ImageGenService>,
    pub(crate) events: Arc<dyn BotEvents>,
}

#[derive(Clone)]
pub(crate) struct BotChatAccess {
    pub(crate) service: ChatService,
    pub(crate) orchestrator: Arc<AIOrchestrator>,
}

#[derive(Debug, thiserror::Error)]
#[error("Kokoro Engine chat is unavailable.")]
pub(crate) struct BotChatUnavailable;

impl From<BotChatUnavailable> for KokoroError {
    fn from(error: BotChatUnavailable) -> Self {
        Self::ExternalService(error.to_string())
    }
}

impl BotChatServices {
    pub(crate) fn new(
        chat: Option<(ChatService, Arc<AIOrchestrator>)>,
        stt: Option<SttService>,
        tts: Option<TtsService>,
        imagegen: Option<ImageGenService>,
        events: Arc<dyn BotEvents>,
    ) -> Self {
        Self {
            chat: chat.map(|(service, orchestrator)| BotChatAccess {
                service,
                orchestrator,
            }),
            stt,
            tts,
            imagegen,
            events,
        }
    }

    pub(crate) fn chat(&self) -> Result<&BotChatAccess, BotChatUnavailable> {
        self.chat.as_ref().ok_or(BotChatUnavailable)
    }
}

pub(crate) const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
const LINE_CHANNEL_ACCESS_TOKEN_ENV: &str = "LINE_CHANNEL_ACCESS_TOKEN";
const LINE_CHANNEL_SECRET_ENV: &str = "LINE_CHANNEL_SECRET";
const WEBHOOK_BEARER_TOKEN_ENV: &str = "KOKORO_WEBHOOK_TOKEN";
pub(crate) const MAX_GENERIC_WEBHOOK_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(default)]
pub struct BotConfig {
    pub revision: u64,
    pub selected_platform: String,
    pub telegram: crate::telegram::TelegramConfig,
    pub qq: crate::qqbot::QQBotConfig,
    pub discord: DiscordBotConfig,
    pub line: LineBotConfig,
    pub webhook: WebhookBotConfig,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            revision: 0,
            selected_platform: "telegram".to_string(),
            telegram: crate::telegram::TelegramConfig::default(),
            qq: crate::qqbot::QQBotConfig::default(),
            discord: DiscordBotConfig::default(),
            line: LineBotConfig::default(),
            webhook: WebhookBotConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(default)]
pub struct DiscordBotConfig {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub bot_token_env: Option<String>,
    pub allowed_channel_ids: Vec<String>,
    pub allow_direct_messages: bool,
    pub send_voice_reply: bool,
    pub character_id: Option<String>,
}

impl Default for DiscordBotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            bot_token_env: Some(DISCORD_BOT_TOKEN_ENV.to_string()),
            allowed_channel_ids: Vec::new(),
            allow_direct_messages: true,
            send_voice_reply: false,
            character_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(default)]
pub struct LineBotConfig {
    pub enabled: bool,
    pub channel_access_token: Option<String>,
    pub channel_access_token_env: Option<String>,
    pub channel_secret: Option<String>,
    pub channel_secret_env: Option<String>,
    pub webhook_path: String,
    pub allowed_user_ids: Vec<String>,
    pub character_id: Option<String>,
}

impl Default for LineBotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channel_access_token: None,
            channel_access_token_env: Some(LINE_CHANNEL_ACCESS_TOKEN_ENV.to_string()),
            channel_secret: None,
            channel_secret_env: Some(LINE_CHANNEL_SECRET_ENV.to_string()),
            webhook_path: "/line/webhook".to_string(),
            allowed_user_ids: Vec::new(),
            character_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(default)]
pub struct WebhookBotConfig {
    pub enabled: bool,
    pub bind_host: String,
    pub port: u16,
    pub endpoint_path: String,
    pub bearer_token: Option<String>,
    pub bearer_token_env: Option<String>,
    pub send_voice_reply: bool,
    pub character_id: Option<String>,
}

impl Default for WebhookBotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_host: "127.0.0.1".to_string(),
            port: 8787,
            endpoint_path: "/webhook/message".to_string(),
            bearer_token: None,
            bearer_token_env: Some(WEBHOOK_BEARER_TOKEN_ENV.to_string()),
            send_voice_reply: false,
            character_id: None,
        }
    }
}

#[derive(Clone)]
pub struct BotRuntimeService {
    config: Arc<RwLock<BotConfig>>,
    qq_authorization_broker: crate::qqbot::QQAuthorizationBroker,
    qq_runtime: Arc<Mutex<Option<QQRuntimeHandle>>>,
    qq_runtime_generation: Arc<AtomicU64>,
    qq_connected_generation: Arc<AtomicU64>,
    discord_shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    http_shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
}

struct QQRuntimeHandle {
    id: u64,
    shutdown_tx: oneshot::Sender<()>,
}

impl BotRuntimeService {
    pub fn new(config: BotConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            qq_authorization_broker: crate::qqbot::QQAuthorizationBroker::default(),
            qq_runtime: Arc::new(Mutex::new(None)),
            qq_runtime_generation: Arc::new(AtomicU64::new(0)),
            qq_connected_generation: Arc::new(AtomicU64::new(0)),
            discord_shutdown_tx: Arc::new(RwLock::new(None)),
            http_shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn save_config(&self, mut config: BotConfig) -> Result<BotConfig, String> {
        let mut current = self.config.write().await;
        if config.revision != current.revision {
            return Err("Bot configuration changed; reload settings before saving".to_string());
        }
        config.revision = current.revision.saturating_add(1);
        save_bot_config_file(&config).map_err(|error| error.to_string())?;
        *current = config.clone();
        Ok(config)
    }

    pub async fn current_config(&self) -> BotConfig {
        self.config.read().await.clone()
    }

    pub async fn respond_to_qq_authorization(
        &self,
        request_id: &str,
        user_openid: &str,
        group_openid: Option<&str>,
        approved: bool,
    ) -> Result<(u64, Option<String>, Option<String>), String> {
        let request_id = request_id.trim();
        let user_openid = user_openid.trim();
        if request_id.is_empty() || request_id.len() > 256 {
            return Err("invalid QQ authorization request ID".to_string());
        }
        if user_openid.is_empty()
            || user_openid.len() > 128
            || !user_openid.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
        {
            return Err("invalid QQ user OpenID".to_string());
        }
        let group_openid = group_openid
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if group_openid.is_some_and(|value| {
            value.len() > 128
                || !value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        }) {
            return Err("invalid QQ group OpenID".to_string());
        }

        let decision_tx = self
            .qq_authorization_broker
            .take(request_id, user_openid, group_openid)
            .await
            .ok_or_else(|| "QQ authorization request is no longer pending".to_string())?;

        let mut approved_user_openid = None;
        let mut approved_group_openid = None;
        let revision = if approved {
            let mut current = self.config.write().await;
            let mut next = current.clone();
            if let Some(group_openid) = group_openid {
                approved_group_openid = Some(group_openid.to_string());
                if !next
                    .qq
                    .allowed_group_openids
                    .iter()
                    .any(|allowed| allowed == group_openid)
                {
                    next.qq.allowed_group_openids.push(group_openid.to_string());
                }
                if !next.qq.allowed_user_openids.is_empty() {
                    approved_user_openid = Some(user_openid.to_string());
                    if !next
                        .qq
                        .allowed_user_openids
                        .iter()
                        .any(|allowed| allowed == user_openid)
                    {
                        next.qq.allowed_user_openids.push(user_openid.to_string());
                    }
                }
            } else {
                approved_user_openid = Some(user_openid.to_string());
                if !next
                    .qq
                    .allowed_user_openids
                    .iter()
                    .any(|allowed| allowed == user_openid)
                {
                    next.qq.allowed_user_openids.push(user_openid.to_string());
                }
            }
            if next != *current {
                next.revision = current.revision.saturating_add(1);
                if let Err(error) = save_bot_config_file(&next) {
                    let _ = decision_tx.send(false);
                    return Err(error.to_string());
                }
                *current = next;
            }
            current.revision
        } else {
            self.qq_authorization_broker
                .mark_rejected(user_openid, group_openid)
                .await;
            self.config.read().await.revision
        };
        let _ = decision_tx.send(approved);
        Ok((revision, approved_user_openid, approved_group_openid))
    }

    pub async fn is_discord_running(&self) -> bool {
        self.discord_shutdown_tx.read().await.is_some()
    }

    pub async fn is_qq_started(&self) -> bool {
        self.qq_runtime.lock().await.is_some()
    }

    pub async fn is_qq_connected(&self) -> bool {
        let runtime_id = self
            .qq_runtime
            .lock()
            .await
            .as_ref()
            .map(|runtime| runtime.id);
        runtime_id.is_some_and(|id| self.qq_connected_generation.load(Ordering::SeqCst) == id)
    }

    pub async fn is_http_running(&self) -> bool {
        self.http_shutdown_tx.read().await.is_some()
    }

    pub async fn start_enabled(&self, services: BotChatServices) {
        let config = self.config.read().await.clone();
        if config.qq.enabled {
            if let Err(error) = self.start_qq(services.clone()).await {
                tracing::error!(target: "bot::qq", "failed to auto-start QQ bot: {}", error);
            }
        }
        if config.discord.enabled {
            if let Err(error) = self.start_discord(services.clone()).await {
                tracing::error!(target: "bot::discord", "failed to auto-start Discord bot: {}", error);
            }
        }
        if config.line.enabled || config.webhook.enabled {
            if let Err(error) = self.start_http(services).await {
                tracing::error!(target: "bot::http", "failed to auto-start Bot HTTP server: {}", error);
            }
        }
    }

    pub async fn start_platform(
        &self,
        platform: &str,
        services: BotChatServices,
    ) -> Result<(), String> {
        match platform {
            "qq" => self.start_qq(services).await,
            "discord" => self.start_discord(services).await,
            "line" | "webhook" => self.start_http(services).await,
            other => Err(format!("Unsupported bot platform runtime: {}", other)),
        }
    }

    pub async fn stop_platform(&self, platform: &str) -> Result<(), String> {
        match platform {
            "qq" => self.stop_qq().await,
            "discord" => self.stop_discord().await,
            "line" | "webhook" => self.stop_http().await,
            other => Err(format!("Unsupported bot platform runtime: {}", other)),
        }
    }

    async fn start_qq(&self, services: BotChatServices) -> Result<(), String> {
        let config = self.config.read().await.clone();
        if !config.qq.enabled {
            return Err("QQ bot is not enabled".to_string());
        }
        if !has_secret(&config.qq.app_id, &config.qq.app_id_env) {
            return Err("No QQ AppID configured".to_string());
        }
        if !has_secret(&config.qq.app_secret, &config.qq.app_secret_env) {
            return Err("No QQ AppSecret configured".to_string());
        }

        let mut runtime_guard = self.qq_runtime.lock().await;
        if runtime_guard.is_some() {
            return Err("QQ bot is already running or connecting".to_string());
        }
        let runtime_id = self
            .qq_runtime_generation
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let (tx, rx) = oneshot::channel();
        *runtime_guard = Some(QQRuntimeHandle {
            id: runtime_id,
            shutdown_tx: tx,
        });
        drop(runtime_guard);

        let config_ref = self.config.clone();
        let authorization_broker = self.qq_authorization_broker.clone();
        let runtime_ref = self.qq_runtime.clone();
        let connected_generation = self.qq_connected_generation.clone();
        tokio::spawn(async move {
            crate::qqbot::run_qq_gateway(
                config_ref,
                authorization_broker,
                services,
                runtime_id,
                connected_generation.clone(),
                rx,
            )
            .await;
            let _ = connected_generation.compare_exchange(
                runtime_id,
                0,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            let mut guard = runtime_ref.lock().await;
            if guard
                .as_ref()
                .is_some_and(|runtime| runtime.id == runtime_id)
            {
                *guard = None;
            }
        });
        Ok(())
    }

    async fn stop_qq(&self) -> Result<(), String> {
        let mut guard = self.qq_runtime.lock().await;
        if let Some(runtime) = guard.take() {
            let _ = runtime.shutdown_tx.send(());
            Ok(())
        } else {
            Err("QQ bot is not running".to_string())
        }
    }

    async fn start_discord(&self, services: BotChatServices) -> Result<(), String> {
        if self.is_discord_running().await {
            return Err("Discord bot is already running".to_string());
        }
        let config = self.config.read().await.clone();
        if !config.discord.enabled {
            return Err("Discord bot is not enabled".to_string());
        }
        let token = resolve_secret(&config.discord.bot_token, &config.discord.bot_token_env)
            .ok_or("No Discord bot token configured")?;

        let (tx, rx) = oneshot::channel();
        *self.discord_shutdown_tx.write().await = Some(tx);
        let config_ref = self.config.clone();
        let shutdown_ref = self.discord_shutdown_tx.clone();
        tokio::spawn(async move {
            run_discord_gateway(token, config_ref, services, rx).await;
            *shutdown_ref.write().await = None;
        });
        Ok(())
    }

    async fn stop_discord(&self) -> Result<(), String> {
        let mut guard = self.discord_shutdown_tx.write().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("Discord bot is not running".to_string())
        }
    }

    async fn start_http(&self, services: BotChatServices) -> Result<(), String> {
        if self.is_http_running().await {
            return Err("Bot HTTP server is already running".to_string());
        }
        let config = self.config.read().await.clone();
        if !config.line.enabled && !config.webhook.enabled {
            return Err("LINE and Webhook bots are not enabled".to_string());
        }
        let host: IpAddr = config
            .webhook
            .bind_host
            .parse()
            .map_err(|e| format!("Invalid bind host: {}", e))?;
        let addr = SocketAddr::new(host, config.webhook.port);

        let (tx, rx) = oneshot::channel();
        *self.http_shutdown_tx.write().await = Some(tx);

        let config_ref = self.config.clone();
        let shutdown_ref = self.http_shutdown_tx.clone();
        tokio::spawn(async move {
            run_http_bot_server(addr, config_ref, services, rx).await;
            *shutdown_ref.write().await = None;
        });
        Ok(())
    }

    async fn stop_http(&self) -> Result<(), String> {
        let mut guard = self.http_shutdown_tx.write().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("Bot HTTP server is not running".to_string())
        }
    }
}

#[derive(Debug, Serialize, specta::Type)]
pub struct BotStatus {
    pub telegram: BotPlatformStatus,
    pub qq: BotPlatformStatus,
    pub discord: BotPlatformStatus,
    pub line: BotPlatformStatus,
    pub webhook: BotPlatformStatus,
}

#[derive(Debug, Serialize, specta::Type)]
pub struct BotPlatformStatus {
    pub enabled: bool,
    pub configured: bool,
    pub running: bool,
    pub connected: bool,
}

fn app_data_dir() -> PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
}

pub(crate) fn bot_config_path() -> PathBuf {
    app_data_dir().join("bot_config.json")
}

fn legacy_telegram_config_path() -> PathBuf {
    app_data_dir().join("telegram_config.json")
}

fn load_legacy_telegram_config(
    path: &Path,
) -> Result<Option<crate::telegram::TelegramConfig>, KokoroError> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let config = serde_json::from_str::<crate::telegram::TelegramConfig>(&content)
                .map_err(|error| {
                    KokoroError::Config(format!(
                        "Failed to parse legacy Telegram config {}: {}",
                        path.display(),
                        error
                    ))
                })?;
            tracing::debug!(target: "config", "[TELEGRAM] Loaded config from {}", path.display());
            Ok(Some(config))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(KokoroError::Config(format!(
            "Failed to read legacy Telegram config {}: {}",
            path.display(),
            error
        ))),
    }
}

fn telegram_config_has_user_values(config: &crate::telegram::TelegramConfig) -> bool {
    config.enabled
        || config
            .bot_token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
        || !config.allowed_chat_ids.is_empty()
        || config.send_voice_reply
        || config
            .character_id
            .as_ref()
            .is_some_and(|character_id| !character_id.is_empty())
}

fn remove_legacy_telegram_config(path: &Path) -> Result<(), KokoroError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(KokoroError::Config(format!(
            "Failed to remove legacy Telegram config {}: {}",
            path.display(),
            error
        ))),
    }
}

pub(crate) fn normalize_telegram_config_env(
    mut config: crate::telegram::TelegramConfig,
) -> crate::telegram::TelegramConfig {
    config.bot_token_env = Some(TELEGRAM_BOT_TOKEN_ENV.to_string());
    config
}

pub(crate) fn normalize_bot_config_envs(mut config: BotConfig) -> BotConfig {
    config.telegram = normalize_telegram_config_env(config.telegram);
    config.qq.app_id_env = Some(crate::qqbot::config::QQ_APP_ID_ENV.to_string());
    config.qq.app_secret_env = Some(crate::qqbot::config::QQ_APP_SECRET_ENV.to_string());
    config.discord.bot_token_env = Some(DISCORD_BOT_TOKEN_ENV.to_string());
    config.line.channel_access_token_env = Some(LINE_CHANNEL_ACCESS_TOKEN_ENV.to_string());
    config.line.channel_secret_env = Some(LINE_CHANNEL_SECRET_ENV.to_string());
    config.webhook.bearer_token_env = Some(WEBHOOK_BEARER_TOKEN_ENV.to_string());
    config
}

pub(crate) fn load_bot_config() -> BotConfig {
    let path = bot_config_path();
    let legacy_path = legacy_telegram_config_path();
    load_bot_config_from_paths(&path, &legacy_path)
}

fn load_bot_config_from_paths(path: &Path, legacy_path: &Path) -> BotConfig {
    let bot_file_exists = path.exists();
    let mut config: BotConfig = crate::config::load_json_config(path, "BOT");
    let mut migrated = false;

    if !bot_file_exists || !telegram_config_has_user_values(&config.telegram) {
        match load_legacy_telegram_config(legacy_path) {
            Ok(Some(legacy_telegram)) => {
                config.telegram = legacy_telegram;
                migrated = true;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    target: "bot",
                    "failed to load legacy telegram_config.json for migration: {}",
                    error
                );
            }
        }
    }

    let config = normalize_bot_config_envs(config);
    if migrated {
        if let Err(error) = save_bot_config_file_to_path(path, &config) {
            tracing::warn!(
                target: "bot",
                "failed to migrate telegram_config.json into bot_config.json: {}",
                error
            );
        } else if let Err(error) = remove_legacy_telegram_config(legacy_path) {
            tracing::warn!(
                target: "bot",
                "failed to remove migrated telegram_config.json: {}",
                error
            );
        } else {
            tracing::info!(
                target: "bot",
                "migrated telegram_config.json into bot_config.json"
            );
        }
    }

    config
}

pub(crate) fn save_bot_config_file(config: &BotConfig) -> Result<(), KokoroError> {
    save_bot_config_file_to_path(&bot_config_path(), config)
}

fn save_bot_config_file_to_path(path: &Path, config: &BotConfig) -> Result<(), KokoroError> {
    let config = normalize_bot_config_envs(config.clone());
    crate::config::save_json_config(path, &config, "BOT")
}

fn has_secret(value: &Option<String>, env: &Option<String>) -> bool {
    crate::config::resolve_api_key(value, env).is_some()
}

fn resolve_secret(value: &Option<String>, env: &Option<String>) -> Option<String> {
    crate::config::resolve_api_key(value, env)
}

pub(crate) async fn get_bot_status(
    state: &TelegramService,
    runtime: &BotRuntimeService,
) -> BotStatus {
    let config = runtime.current_config().await;
    let telegram_config = state.get_config().await;
    let qq_running = runtime.is_qq_started().await;
    let qq_connected = runtime.is_qq_connected().await;
    let discord_running = runtime.is_discord_running().await;
    let http_running = runtime.is_http_running().await;
    BotStatus {
        telegram: BotPlatformStatus {
            enabled: telegram_config.enabled,
            configured: telegram_config.resolve_bot_token().is_some(),
            running: state.is_running().await,
            connected: state.is_running().await,
        },
        qq: BotPlatformStatus {
            enabled: config.qq.enabled,
            configured: has_secret(&config.qq.app_id, &config.qq.app_id_env)
                && has_secret(&config.qq.app_secret, &config.qq.app_secret_env),
            running: qq_running,
            connected: qq_connected,
        },
        discord: BotPlatformStatus {
            enabled: config.discord.enabled,
            configured: has_secret(&config.discord.bot_token, &config.discord.bot_token_env),
            running: discord_running,
            connected: discord_running,
        },
        line: BotPlatformStatus {
            enabled: config.line.enabled,
            configured: has_secret(
                &config.line.channel_access_token,
                &config.line.channel_access_token_env,
            ) && has_secret(
                &config.line.channel_secret,
                &config.line.channel_secret_env,
            ),
            running: http_running && config.line.enabled,
            connected: http_running && config.line.enabled,
        },
        webhook: BotPlatformStatus {
            enabled: config.webhook.enabled,
            configured: !config.webhook.endpoint_path.trim().is_empty(),
            running: http_running && config.webhook.enabled,
            connected: http_running && config.webhook.enabled,
        },
    }
}

pub(crate) async fn save_bot_config(
    state: &TelegramService,
    runtime: &BotRuntimeService,
    config: BotConfig,
) -> Result<BotConfig, KokoroError> {
    let saved = runtime
        .save_config(normalize_bot_config_envs(config))
        .await
        .map_err(KokoroError::Config)?;
    state.update_config(saved.telegram.clone()).await;
    Ok(saved)
}

pub(crate) async fn save_telegram_config(
    state: &TelegramService,
    config: crate::telegram::TelegramConfig,
) -> Result<(), KokoroError> {
    let config = normalize_telegram_config_env(config);
    let mut bot_config = load_bot_config();
    bot_config.telegram = config.clone();
    save_bot_config_file(&bot_config)?;
    state.update_config(config).await;
    Ok(())
}

#[derive(Debug, Serialize)]
struct BotReply {
    character_id: String,
    conversation_id: String,
    request_id: String,
    #[serde(skip)]
    speech_snapshot: Option<Arc<crate::tts::manager::TtsExecutionSnapshot>>,
    reply: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<BotReplyImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<BotReplyAudio>,
    #[serde(skip)]
    generated_image_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    output_warnings: Vec<String>,
    #[serde(skip)]
    image_prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BotReplyAudio {
    mime_type: String,
    file_name: String,
    data_base64: String,
}

#[derive(Debug, Clone)]
struct GeneratedBotImage {
    prompt: String,
    mime_type: String,
    file_name: String,
    data: Vec<u8>,
}

fn reply_text_with_translation(reply: &BotReply) -> String {
    let mut text = reply.reply.clone();
    if let Some(translation) = reply
        .translation
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        text.push_str(&format!("\n\nTranslation: {translation}"));
    }
    for warning in &reply.output_warnings {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(warning);
    }
    text
}

/// Bind identity before transport preparation and retain the activation/database
/// guards until generation or replay has completed.
struct BotRequestBinding {
    character_id: String,
    conversation_id: String,
    request_id: String,
    previous_request_ids: Vec<String>,
}

impl BotRequestBinding {
    async fn new(
        state: &AIOrchestrator,
        platform: &str,
        character_id: Option<&str>,
        conversation_key: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<Self, KokoroError> {
        let _guard = state
            .enter_chat_turn()
            .map_err(|e| KokoroError::Chat(format!("Character activation is in progress: {e}")))?;
        let character_id = match character_id.filter(|id| !id.trim().is_empty()) {
            Some(id) => resolve_webhook_character_id(Some(id), None, None),
            None => {
                let id = state.get_character_id().await;
                if id.is_empty() || id == "default" {
                    AIOrchestrator::load_active_character_id().unwrap_or_else(|| "default".into())
                } else {
                    id
                }
            }
        };
        let key = conversation_key
            .filter(|key| !key.trim().is_empty())
            .unwrap_or(platform);
        let previous_request_ids = request_id
            .map(|id| {
                vec![persistence::legacy_external_identity(
                    platform,
                    conversation_key.unwrap_or(platform),
                    id,
                )]
            })
            .unwrap_or_default();
        let request_id = request_id
            .map(|id| persistence::external_identity(platform, &[key, id]))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        persistence::validate_external_request_owner(
            &state.db,
            &character_id,
            platform,
            &request_id,
            &previous_request_ids,
        )
        .await?;
        Ok(Self {
            conversation_id: if platform == "webhook" {
                webhook_conversation_id(&character_id, Some(key))
            } else {
                persistence::external_identity(platform, &[&character_id, key])
            },
            request_id,
            previous_request_ids,
            character_id,
        })
    }
}

struct BotMediaInput {
    text: String,
    images: Vec<String>,
    input_text: Option<String>,
}

async fn prepare_bot_media_input<F, Fut>(
    state: &AIOrchestrator,
    binding: &BotRequestBinding,
    input_text: &str,
    prepare: F,
) -> Result<BotMediaInput, KokoroError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(String, Vec<String>), KokoroError>>,
{
    persistence::validate_user_input(input_text)?;
    let saved: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.content, m.metadata FROM conversation_messages m JOIN conversations c ON c.id=m.conversation_id
         WHERE c.character_id=? AND m.role='user'
         AND CASE WHEN json_valid(m.metadata) THEN json_extract(m.metadata,'$.chat_save_id') END=?",
    ).bind(&binding.character_id).bind(format!("{}:user", binding.request_id))
        .fetch_optional(&state.db).await.map_err(KokoroError::chat_persistence)?;
    if let Some((text, metadata)) = saved {
        let original_input = metadata
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|value| value["external_input_text"].as_str().map(str::to_owned));
        let matches = original_input.as_ref().map_or_else(
            || {
                input_text.trim().is_empty()
                    || text == input_text
                    || text.starts_with(&format!("{input_text}\n\nVoice message transcript: "))
            },
            |original| original == input_text,
        );
        if !matches {
            return Err(KokoroError::Validation(
                "Request identity was reused with different text".into(),
            ));
        }
        return Ok(BotMediaInput {
            text,
            images: Vec::new(),
            input_text: original_input,
        });
    }
    let (text, images) = prepare().await?;
    Ok(BotMediaInput {
        text,
        images,
        input_text: Some(input_text.to_owned()),
    })
}

#[derive(Default)]
struct BotInput {
    text: String,
    images: Vec<String>,
    audio: Option<ChatAudioInput>,
}

async fn generate_bot_reply(
    services: &BotChatServices,
    platform: &'static str,
    input: BotInput,
    character_id: Option<&str>,
    conversation_key: Option<&str>,
    message_id: Option<&str>,
) -> Result<BotReply, KokoroError> {
    let state = services.chat()?.orchestrator.as_ref();
    let binding =
        BotRequestBinding::new(state, platform, character_id, conversation_key, message_id).await?;
    submit_bound_bot_reply(services, platform, input, &binding, None).await
}

async fn submit_bound_bot_reply(
    services: &BotChatServices,
    platform: &'static str,
    input: BotInput,
    binding: &BotRequestBinding,
    input_text: Option<&str>,
) -> Result<BotReply, KokoroError> {
    let text = if input.text.trim().is_empty() && !input.images.is_empty() {
        "The user sent an image:"
    } else {
        &input.text
    };
    if text.is_empty() && input.audio.is_none() {
        return Err(KokoroError::Validation("Message text is empty".into()));
    }
    let response = services
        .chat()?
        .service
        .submit(
            ChatRequest {
                message: text.to_owned(),
                character_id: Some(binding.character_id.clone()),
                conversation_id: Some(binding.conversation_id.clone()),
                client_request_id: Some(binding.request_id.clone()),
                previous_request_ids: binding.previous_request_ids.clone(),
                external_input_text: input_text.map(str::to_owned),
                images: Some(input.images),
                audio: input.audio,
                ..Default::default()
            },
            ChatPolicy::External {
                create_conversation: true,
                image_output: platform != "qq",
                source: platform,
            },
        )
        .await?;
    if response.status.as_deref() == Some("cancelled") {
        return Err(KokoroError::Chat("Chat turn cancelled".into()));
    }
    let images = response.external_media.images;
    Ok(BotReply {
        character_id: binding.character_id.clone(),
        conversation_id: response.conversation_id,
        request_id: binding.request_id.clone(),
        speech_snapshot: response.speech_snapshot,
        reply: response.text,
        translation: response.translation,
        generated_image_count: images.len(),
        images,
        audio: None,
        image_prompts: response.external_media.image_prompts,
        output_warnings: response.output_warnings,
    })
}

pub(crate) async fn generate_bot_text_reply(
    services: &BotChatServices,
    platform: &'static str,
    text: &str,
    character_id: Option<&str>,
    conversation_key: &str,
    message_id: &str,
) -> Result<String, String> {
    let reply = generate_bot_reply(
        services,
        platform,
        BotInput {
            text: text.into(),
            ..Default::default()
        },
        character_id,
        Some(conversation_key),
        Some(message_id),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(reply_text_with_translation(&reply))
}

fn image_mime_from_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

fn audio_format_from_mime_or_name(mime_type: Option<&str>, name: Option<&str>) -> String {
    let candidate = mime_type.or(name).unwrap_or_default().to_ascii_lowercase();
    if candidate.contains("ogg") || candidate.contains("opus") {
        "ogg".to_string()
    } else if candidate.contains("mpeg") || candidate.contains("mp3") || candidate.ends_with(".mp3")
    {
        "mp3".to_string()
    } else if candidate.contains("wav") || candidate.ends_with(".wav") {
        "wav".to_string()
    } else if candidate.contains("webm") || candidate.ends_with(".webm") {
        "webm".to_string()
    } else if candidate.contains("mp4")
        || candidate.contains("m4a")
        || candidate.ends_with(".m4a")
        || candidate.ends_with(".mp4")
    {
        "m4a".to_string()
    } else {
        "ogg".to_string()
    }
}

fn clean_mime_type(mime_type: &str) -> &str {
    mime_type.split(';').next().unwrap_or(mime_type).trim()
}

fn is_image_media(mime_type: Option<&str>, name: Option<&str>) -> bool {
    mime_type.is_some_and(|value| clean_mime_type(value).starts_with("image/"))
        || name.is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            lower.ends_with(".png")
                || lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".webp")
                || lower.ends_with(".gif")
        })
}

fn is_audio_media(mime_type: Option<&str>, name: Option<&str>) -> bool {
    mime_type.is_some_and(|value| clean_mime_type(value).starts_with("audio/"))
        || name.is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            lower.ends_with(".ogg")
                || lower.ends_with(".opus")
                || lower.ends_with(".mp3")
                || lower.ends_with(".wav")
                || lower.ends_with(".webm")
                || lower.ends_with(".m4a")
                || lower.ends_with(".mp4")
        })
}

fn image_data_url(data: &[u8], mime_type: &str) -> String {
    format!(
        "data:{};base64,{}",
        clean_mime_type(mime_type),
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

fn normalize_image_reference(value: &str, fallback_mime: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("data:image/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        Some(trimmed.to_string())
    } else {
        Some(format!("data:{};base64,{}", fallback_mime, trimmed))
    }
}

fn validate_image_reference(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Invalid image base64: empty payload".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(());
    }
    let payload = if let Some((header, data)) = trimmed.split_once(',') {
        if !header.starts_with("data:image/") || !header.contains(";base64") {
            return Err("Invalid image base64: unsupported data URL".to_string());
        }
        data
    } else {
        trimmed
    };
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("Invalid image base64: {error}"))?;
    Ok(())
}

fn validate_generic_webhook_body(body: &[u8]) -> Result<(), String> {
    if body.len() > MAX_GENERIC_WEBHOOK_BODY_BYTES {
        return Err(format!(
            "Webhook body exceeds the {} byte limit",
            MAX_GENERIC_WEBHOOK_BODY_BYTES
        ));
    }
    Ok(())
}

fn decode_base64_payload(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    let payload = if let Some((_, data)) = trimmed.split_once(',') {
        data
    } else {
        trimmed
    };
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("Invalid base64 payload: {}", error))
}

fn bot_reply_image_bytes(image: &BotReplyImage) -> Result<Vec<u8>, String> {
    decode_base64_payload(&image.data_base64)
}

fn text_with_transcription(text: String, transcription: Option<String>) -> String {
    match transcription.filter(|value| !value.trim().is_empty()) {
        Some(transcription) if text.trim().is_empty() => transcription,
        Some(transcription) => format!("{}\n\nVoice message transcript: {}", text, transcription),
        None => text,
    }
}

async fn transcribe_bot_audio(
    services: &BotChatServices,
    platform: &str,
    data: Vec<u8>,
    format: String,
) -> Result<Option<String>, String> {
    let Some(stt_service) = services.stt.as_ref() else {
        return Err("STT service not available".to_string());
    };
    let audio_source = AudioSource::Encoded { data, format };
    match stt_service.transcribe(&audio_source, None).await {
        Ok(result) => {
            let text = result.text.trim().to_string();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text))
            }
        }
        Err(error) => {
            tracing::error!(target: "bot::stt", "[{}] STT error: {}", platform, error);
            Err(format!("Failed to transcribe audio: {}", error))
        }
    }
}

async fn synthesize_bot_audio(
    text: &str,
    speech: Option<&crate::tts::manager::TtsExecutionSnapshot>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(speech) = speech else {
        return Ok(None);
    };
    speech
        .synthesize(text)
        .await
        .map(|audio| (!audio.is_empty()).then_some(audio))
}

async fn generate_bot_images(
    services: &BotChatServices,
    platform: &str,
    reply: &BotReply,
) -> Vec<GeneratedBotImage> {
    let prompts = &reply.image_prompts;
    if prompts.is_empty() {
        return Vec::new();
    }
    let Some(imagegen) = services.imagegen.as_ref() else {
        tracing::warn!(
            target: "bot::imagegen",
            "[{}] image generation requested but ImageGenService is not available",
            platform
        );
        return Vec::new();
    };

    let mut images = Vec::new();
    for prompt in prompts {
        match imagegen.generate(prompt.clone(), None, None, None).await {
            Ok(result) => match tokio::fs::read(&result.image_url).await {
                Ok(data) => {
                    let file_name = Path::new(&result.image_url)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("image.png")
                        .to_string();
                    let _ = services.events.emit(BotEvent::ImageGenerated {
                        result,
                        character_id: reply.character_id.clone(),
                        conversation_id: reply.conversation_id.clone(),
                        client_request_id: reply.request_id.clone(),
                    });
                    images.push(GeneratedBotImage {
                        prompt: prompt.clone(),
                        mime_type: image_mime_from_name(&file_name).to_string(),
                        file_name,
                        data,
                    });
                }
                Err(error) => {
                    tracing::error!(
                        target: "bot::imagegen",
                        "[{}] failed to read generated image: {}",
                        platform,
                        error
                    );
                }
            },
            Err(error) => {
                tracing::error!(
                    target: "bot::imagegen",
                    "[{}] image generation failed: {}",
                    platform,
                    error
                );
            }
        }
    }
    images
}

fn bot_reply_image_from_generated(image: GeneratedBotImage) -> BotReplyImage {
    BotReplyImage {
        prompt: image.prompt,
        mime_type: image.mime_type,
        file_name: image.file_name,
        data_base64: base64::engine::general_purpose::STANDARD.encode(image.data),
    }
}

async fn attach_generated_images_to_reply(
    services: &BotChatServices,
    platform: &str,
    reply: &mut BotReply,
) {
    let images = generate_bot_images(services, platform, reply).await;
    let new_images = images
        .into_iter()
        .map(bot_reply_image_from_generated)
        .collect::<Vec<_>>();
    reply.generated_image_count += new_images.len();
    reply.images.extend(new_images);
}

async fn attach_voice_reply_to_webhook(reply: &mut BotReply) {
    if reply.reply.is_empty() {
        return;
    }
    match synthesize_bot_audio(&reply.reply, reply.speech_snapshot.as_deref()).await {
        Ok(Some(data)) => {
            let (mime_type, extension) = audio_metadata(&data);
            reply.audio = Some(BotReplyAudio {
                mime_type: mime_type.to_string(),
                file_name: format!("reply.{extension}"),
                data_base64: base64::engine::general_purpose::STANDARD.encode(data),
            });
        }
        Ok(None) => reply.output_warnings.push("TTS returned no audio.".into()),
        Err(error) => {
            tracing::error!(target: "bot::webhook", "failed to synthesize voice reply: {}", error);
            reply
                .output_warnings
                .push("Kokoro Engine could not synthesize the voice reply.".into());
        }
    }
}

fn audio_metadata(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(b"OggS") {
        return ("audio/ogg", "ogg");
    }
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WAVE" {
        return ("audio/wav", "wav");
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
    {
        return ("audio/mpeg", "mp3");
    }
    ("application/octet-stream", "bin")
}

pub(crate) fn audio_metadata_for_external(bytes: &[u8]) -> (&'static str, &'static str) {
    audio_metadata(bytes)
}

async fn download_url_bytes(
    client: &reqwest::Client,
    url: &str,
    bearer_token: Option<&str>,
) -> Result<(Vec<u8>, Option<String>), String> {
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("download returned {}", response.status()));
    }
    const MAX_BOT_MEDIA_BYTES: u64 = 32 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BOT_MEDIA_BYTES)
    {
        return Err("downloaded media exceeds the size limit".to_string());
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut stream = response.bytes_stream();
    let mut data = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        let next_size = data
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "downloaded media size overflow".to_string())?;
        if next_size as u64 > MAX_BOT_MEDIA_BYTES {
            return Err("downloaded media exceeds the size limit".to_string());
        }
        data.extend_from_slice(&chunk);
    }
    Ok((data, content_type))
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

fn json_response(value: Value, status: StatusCode) -> warp::reply::Response {
    warp::reply::with_status(warp::reply::json(&value), status).into_response()
}

fn unauthorized(message: &str) -> warp::reply::Response {
    json_response(json!({ "error": message }), StatusCode::UNAUTHORIZED)
}

fn bad_request(message: &str) -> warp::reply::Response {
    json_response(json!({ "error": message }), StatusCode::BAD_REQUEST)
}

fn server_error(message: &str) -> warp::reply::Response {
    json_response(
        json!({ "error": message }),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
}

async fn run_http_bot_server(
    addr: SocketAddr,
    config: Arc<RwLock<BotConfig>>,
    services: BotChatServices,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let route = warp::post()
        .and(warp::path::full())
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::header::optional::<String>("x-line-signature"))
        .and(warp::body::content_length_limit(
            MAX_GENERIC_WEBHOOK_BODY_BYTES as u64,
        ))
        .and(warp::body::bytes())
        .and(warp::any().map(move || config.clone()))
        .and(warp::any().map(move || services.clone()))
        .and_then(handle_http_bot_request)
        .recover(recover_http_bot_rejection);

    tracing::info!(target: "bot::http", "Bot HTTP server listening on {}", addr);
    warp::serve(route)
        .bind_with_graceful_shutdown(addr, async move {
            let _ = shutdown_rx.await;
        })
        .1
        .await;
    tracing::info!(target: "bot::http", "Bot HTTP server stopped");
}

async fn recover_http_bot_rejection(
    rejection: warp::Rejection,
) -> Result<warp::reply::Response, Infallible> {
    let (status, message) = if rejection.find::<warp::reject::PayloadTooLarge>().is_some() {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "Webhook body exceeds the configured limit",
        )
    } else if rejection.is_not_found() {
        (StatusCode::NOT_FOUND, "Not found")
    } else {
        (StatusCode::BAD_REQUEST, "Invalid webhook request")
    };
    Ok(json_response(json!({ "error": message }), status))
}

async fn handle_http_bot_request(
    full_path: warp::path::FullPath,
    authorization: Option<String>,
    line_signature: Option<String>,
    body: bytes::Bytes,
    config: Arc<RwLock<BotConfig>>,
    services: BotChatServices,
) -> Result<warp::reply::Response, Infallible> {
    let cfg = config.read().await.clone();
    let path = full_path.as_str();
    let line_path = normalize_path(&cfg.line.webhook_path);
    let webhook_path = normalize_path(&cfg.webhook.endpoint_path);

    let response = if cfg.line.enabled && path == line_path {
        handle_line_webhook(cfg.line, line_signature, body, services).await
    } else if cfg.webhook.enabled && path == webhook_path {
        handle_generic_webhook(cfg.webhook, authorization, body, &services).await
    } else {
        json_response(json!({ "error": "Not found" }), StatusCode::NOT_FOUND)
    };

    Ok(response)
}

#[derive(Debug, Deserialize)]
struct GenericWebhookMessage {
    request_id: Option<String>,
    text: Option<String>,
    message: Option<String>,
    image: Option<String>,
    images: Option<Vec<String>>,
    image_base64: Option<String>,
    image_mime_type: Option<String>,
    audio_base64: Option<String>,
    audio_format: Option<String>,
    character_id: Option<String>,
    conversation_id: Option<String>,
    conversation_type: Option<String>,
    user_id: Option<String>,
    source: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedGenericWebhookPayload {
    request_id: Option<String>,
    text: String,
    images: Vec<String>,
    audio: Option<Vec<u8>>,
    audio_format: Option<String>,
    character_id: Option<String>,
    conversation_key: Option<String>,
}

/// Resolve the character for one webhook request without allowing a blank
/// override to shadow the configured or active character.
fn resolve_webhook_character_id(
    request_character_id: Option<&str>,
    configured_character_id: Option<&str>,
    active_character_id: Option<&str>,
) -> String {
    [
        request_character_id,
        configured_character_id,
        active_character_id,
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|value| !value.is_empty())
    .unwrap_or("default")
    .to_string()
}

/// Validate the optional bearer token at the HTTP boundary. The auth scheme
/// is case-insensitive per RFC 7235, while the token itself remains exact.
fn verify_webhook_bearer(
    authorization: Option<&str>,
    expected_token: Option<&str>,
) -> Result<(), String> {
    let Some(expected_token) = expected_token.filter(|token| !token.trim().is_empty()) else {
        return Err("Webhook bearer token is not configured".to_string());
    };
    let Some(authorization) = authorization else {
        return Err("Missing Authorization header".to_string());
    };
    let Some((scheme, token)) = authorization.trim().split_once(char::is_whitespace) else {
        return Err("Invalid bearer token".to_string());
    };
    if !scheme.eq_ignore_ascii_case("Bearer") || token.trim() != expected_token {
        return Err("Invalid bearer token".to_string());
    }
    Ok(())
}

/// Build a stable external conversation key that is safe to scope to one
/// character. Private chats use the user identity; group chats use the group
/// identity so all members share one character-owned conversation.
fn webhook_conversation_key(
    conversation_type: Option<&str>,
    conversation_id: Option<&str>,
    user_id: Option<&str>,
    source: Option<&str>,
) -> Option<String> {
    let conversation_type = conversation_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("private")
        .to_ascii_lowercase();
    fn first_non_empty<'a>(values: &[Option<&'a str>]) -> Option<&'a str> {
        values
            .iter()
            .copied()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())
    }
    let prefix = if matches!(conversation_type.as_str(), "group" | "channel") {
        "group"
    } else {
        "private"
    };
    let identity = first_non_empty(&[conversation_id, user_id]);
    let source = first_non_empty(&[source]).unwrap_or("unknown");
    identity
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("{prefix}:{source}:{value}"))
}

/// Scope an external conversation identity to a character before it reaches
/// the persistence boundary. This prevents one bot identity from mixing
/// histories across characters.
fn map_webhook_conversation_to_character(
    character_id: &str,
    conversation_key: Option<&str>,
) -> String {
    let character_id = character_id.trim();
    let conversation_key = conversation_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("webhook");
    format!("{character_id}:{conversation_key}")
}

fn parse_generic_webhook_payload(body: &[u8]) -> Result<ParsedGenericWebhookPayload, String> {
    let request: GenericWebhookMessage =
        serde_json::from_slice(body).map_err(|error| format!("Invalid JSON: {error}"))?;
    let text = request
        .text
        .or(request.message)
        .unwrap_or_default()
        .trim()
        .to_string();
    let image_mime_type = request.image_mime_type.as_deref().unwrap_or("image/jpeg");
    let mut images = Vec::new();
    if let Some(image) = request.image.as_deref() {
        validate_image_reference(image)?;
        if let Some(normalized) = normalize_image_reference(image, image_mime_type) {
            images.push(normalized);
        }
    }
    if let Some(request_images) = request.images.as_ref() {
        for image in request_images {
            validate_image_reference(image)?;
            if let Some(normalized) = normalize_image_reference(image, image_mime_type) {
                images.push(normalized);
            }
        }
    }
    if let Some(image_base64) = request.image_base64.as_deref() {
        validate_image_reference(image_base64)?;
        if let Some(normalized) = normalize_image_reference(image_base64, image_mime_type) {
            images.push(normalized);
        }
    }

    let audio = request
        .audio_base64
        .as_deref()
        .map(decode_base64_payload)
        .transpose()?;
    let audio_format = audio.as_ref().map(|_| {
        request
            .audio_format
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| audio_format_from_mime_or_name(None, None))
    });
    let text = if text.is_empty() && !images.is_empty() {
        "The user sent an image:".to_string()
    } else {
        text
    };
    if text.is_empty() && audio.is_none() && images.is_empty() {
        return Err("Missing text or media".to_string());
    }

    let request_id = request.request_id.filter(|id| !id.trim().is_empty());
    if request_id.as_ref().is_some_and(|id| id.len() > 256) {
        return Err("request_id must be at most 256 bytes".into());
    }
    Ok(ParsedGenericWebhookPayload {
        request_id,
        text,
        images,
        audio,
        audio_format,
        character_id: request
            .character_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        conversation_key: webhook_conversation_key(
            request.conversation_type.as_deref(),
            request.conversation_id.as_deref(),
            request.user_id.as_deref(),
            request.source.as_deref(),
        ),
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Authenticate, normalize and deliver a webhook chat.
fn webhook_conversation_id(character_id: &str, conversation_key: Option<&str>) -> String {
    let scoped_key = map_webhook_conversation_to_character(character_id, conversation_key);
    format!(
        "webhook-{}",
        hex_digest(&Sha256::digest(scoped_key.as_bytes()))
    )
}

fn bot_error_response(error: &KokoroError) -> warp::reply::Response {
    match error {
        KokoroError::ChatPersistence(message) => json_response(
            json!({ "error": message, "code": KokoroError::CHAT_PERSISTENCE_CODE }),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        KokoroError::Chat(message) if message.starts_with("chat_request_conflict") => {
            json_response(json!({"error":message}), StatusCode::CONFLICT)
        }
        KokoroError::Validation(message) => bad_request(message),
        KokoroError::Chat(message)
            if message.starts_with("Character activation is in progress:")
                || message.contains("chat_turn_busy") =>
        {
            json_response(
                json!({ "error": "Character activation is in progress" }),
                StatusCode::SERVICE_UNAVAILABLE,
            )
        }
        _ => server_error(&error.to_string()),
    }
}

fn chat_unavailable(error: BotChatUnavailable) -> warp::reply::Response {
    json_response(
        json!({ "error": error.to_string() }),
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

pub(crate) async fn handle_generic_webhook(
    config: WebhookBotConfig,
    authorization: Option<String>,
    body: bytes::Bytes,
    services: &BotChatServices,
) -> warp::reply::Response {
    if let Err(error) = validate_generic_webhook_body(&body) {
        return bad_request(&error);
    }
    if let Err(error) = verify_webhook_bearer(
        authorization.as_deref(),
        resolve_secret(&config.bearer_token, &config.bearer_token_env).as_deref(),
    ) {
        return unauthorized(&error);
    }

    let parsed = match parse_generic_webhook_payload(&body) {
        Ok(value) => value,
        Err(error) => return bad_request(&error),
    };
    let chat = match services.chat() {
        Ok(chat) => chat,
        Err(error) => return chat_unavailable(error),
    };
    let active_character_id = chat.orchestrator.get_character_id().await;
    let character_id = resolve_webhook_character_id(
        parsed.character_id.as_deref(),
        config.character_id.as_deref(),
        Some(&active_character_id),
    );

    match generate_bot_reply(
        services,
        "webhook",
        BotInput {
            text: parsed.text,
            images: parsed.images,
            audio: parsed.audio.map(|data| ChatAudioInput {
                data,
                format: parsed
                    .audio_format
                    .unwrap_or_else(|| audio_format_from_mime_or_name(None, None)),
            }),
        },
        Some(&character_id),
        parsed.conversation_key.as_deref(),
        parsed.request_id.as_deref(),
    )
    .await
    {
        Ok(mut reply) => {
            attach_generated_images_to_reply(services, "webhook", &mut reply).await;
            if config.send_voice_reply {
                attach_voice_reply_to_webhook(&mut reply).await;
            }
            json_response(json!(reply), StatusCode::OK)
        }
        Err(KokoroError::Validation(error))
            if error.starts_with("chat_request_already_persisted") =>
        {
            json_response(json!({ "error": error }), StatusCode::CONFLICT)
        }
        Err(KokoroError::Validation(error)) => bad_request(&error),
        Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict") => {
            json_response(json!({ "error": error }), StatusCode::CONFLICT)
        }
        Err(error) => bot_error_response(&error),
    }
}

#[derive(Debug, Deserialize)]
struct LineWebhookPayload {
    events: Vec<LineEvent>,
}

#[derive(Debug, Deserialize)]
struct LineEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(rename = "replyToken")]
    reply_token: Option<String>,
    source: Option<LineSource>,
    message: Option<LineMessage>,
}

#[derive(Debug, Deserialize)]
struct LineSource {
    #[serde(rename = "userId")]
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LineMessage {
    id: Option<String>,
    #[serde(rename = "type")]
    message_type: String,
    text: Option<String>,
}

async fn handle_line_webhook(
    config: LineBotConfig,
    signature: Option<String>,
    body: bytes::Bytes,
    services: BotChatServices,
) -> warp::reply::Response {
    let Some(secret) = resolve_secret(&config.channel_secret, &config.channel_secret_env) else {
        return server_error("LINE channel secret is not configured");
    };
    let Some(signature) = signature else {
        return unauthorized("Missing LINE signature");
    };
    if !verify_line_signature(&secret, &body, &signature) {
        return unauthorized("Invalid LINE signature");
    }

    let Some(access_token) = resolve_secret(
        &config.channel_access_token,
        &config.channel_access_token_env,
    ) else {
        return server_error("LINE channel access token is not configured");
    };

    let payload: LineWebhookPayload = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => return bad_request(&format!("Invalid JSON: {}", error)),
    };

    for event in payload.events {
        if event.event_type != "message" {
            continue;
        }
        let Some(message) = event.message else {
            continue;
        };
        let user_id = event.source.and_then(|source| source.user_id);
        if !config.allowed_user_ids.is_empty() {
            let Some(ref user_id) = user_id else {
                continue;
            };
            if !config
                .allowed_user_ids
                .iter()
                .any(|allowed| allowed == user_id)
            {
                continue;
            }
        }
        let Some(reply_token) = event.reply_token else {
            continue;
        };

        let chat = match services.chat() {
            Ok(chat) => chat,
            Err(error) => return chat_unavailable(error),
        };
        let state = chat.orchestrator.as_ref();
        let binding = match BotRequestBinding::new(
            state,
            "line",
            config.character_id.as_deref(),
            user_id.as_deref(),
            message.id.as_deref(),
        )
        .await
        {
            Ok(binding) => binding,
            Err(error) => {
                tracing::error!(target:"bot::line", "{error}");
                continue;
            }
        };
        let input = if message.message_type == "text" {
            BotMediaInput {
                text: message.text.unwrap_or_default(),
                images: vec![],
                input_text: None,
            }
        } else if matches!(message.message_type.as_str(), "image" | "audio") {
            let prepared = prepare_bot_media_input(state, &binding, "", || async {
                let message_id = message.id.as_deref().ok_or_else(|| {
                    KokoroError::Validation("Missing LINE message identity".into())
                })?;
                let url = format!("https://api-data.line.me/v2/bot/message/{message_id}/content");
                let (data, content_type) =
                    download_url_bytes(&reqwest::Client::new(), &url, Some(&access_token))
                        .await
                        .map_err(KokoroError::Chat)?;
                if message.message_type == "image" {
                    Ok((
                        "The user sent an image:".into(),
                        vec![image_data_url(
                            &data,
                            content_type.as_deref().unwrap_or("image/jpeg"),
                        )],
                    ))
                } else {
                    let format = audio_format_from_mime_or_name(content_type.as_deref(), None);
                    let text = transcribe_bot_audio(&services, "line", data, format)
                        .await
                        .map_err(KokoroError::Stt)?
                        .ok_or_else(|| KokoroError::Stt("Could not recognize speech".into()))?;
                    Ok((text, vec![]))
                }
            })
            .await;
            match prepared {
                Ok(input) => input,
                Err(error) => {
                    tracing::error!(target:"bot::line", "Failed to prepare or recover LINE media: {error}");
                    let _ = send_line_reply(&access_token, &reply_token, &error.to_string()).await;
                    continue;
                }
            }
        } else {
            continue;
        };
        if input.text.trim().is_empty() && input.images.is_empty() {
            continue;
        }

        match submit_bound_bot_reply(
            &services,
            "line",
            BotInput {
                text: input.text,
                images: input.images,
                ..Default::default()
            },
            &binding,
            input.input_text.as_deref(),
        )
        .await
        {
            Ok(mut reply) => {
                let image_requested =
                    !reply.image_prompts.is_empty() || reply.generated_image_count > 0;
                if !reply.image_prompts.is_empty() {
                    attach_generated_images_to_reply(&services, "line", &mut reply).await;
                }
                let mut reply_text = reply_text_with_translation(&reply);
                if image_requested {
                    let image_notice = if reply.generated_image_count == 0 {
                        "Image generation was requested, but Kokoro Engine could not create the image. Please check the Kokoro Engine client."
                    } else {
                        "Image generated in Kokoro Engine. Please open the Kokoro Engine client to view it."
                    };
                    if reply_text.trim().is_empty() {
                        reply_text = image_notice.to_string();
                    } else {
                        reply_text = format!("{}\n\n{}", reply_text, image_notice);
                    }
                }
                if let Err(error) = send_line_reply(&access_token, &reply_token, &reply_text).await
                {
                    tracing::error!(target: "bot::line", "failed to send LINE reply: {}", error);
                }
            }
            Err(error) => {
                tracing::error!(target: "bot::line", "failed to generate LINE reply: {}", error);
                if matches!(error, KokoroError::ChatPersistence(_)) {
                    return bot_error_response(&error);
                }
            }
        }
    }

    json_response(json!({ "ok": true }), StatusCode::OK)
}

fn verify_line_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    let expected = base64::engine::general_purpose::STANDARD.encode(digest);
    expected == signature.trim()
}

async fn send_line_reply(access_token: &str, reply_token: &str, text: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.line.me/v2/bot/message/reply")
        .bearer_auth(access_token)
        .json(&json!({
            "replyToken": reply_token,
            "messages": [
                {
                    "type": "text",
                    "text": truncate_for_platform(text, 4900),
                }
            ]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("LINE API returned {}", response.status()));
    }
    Ok(())
}

async fn run_discord_gateway(
    token: String,
    config: Arc<RwLock<BotConfig>>,
    services: BotChatServices,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let client = reqwest::Client::new();
    loop {
        let enabled = config.read().await.discord.enabled;
        if !enabled {
            break;
        }

        match run_discord_session(
            &client,
            &token,
            config.clone(),
            services.clone(),
            &mut shutdown_rx,
        )
        .await
        {
            DiscordSessionExit::Shutdown => break,
            DiscordSessionExit::Reconnect => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    tracing::info!(target: "bot::discord", "Discord bot stopped");
}

enum DiscordSessionExit {
    Shutdown,
    Reconnect,
}

async fn run_discord_session(
    client: &reqwest::Client,
    token: &str,
    config: Arc<RwLock<BotConfig>>,
    services: BotChatServices,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> DiscordSessionExit {
    let gateway_url = match fetch_discord_gateway(client, token).await {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(target: "bot::discord", "failed to fetch gateway URL: {}", error);
            return DiscordSessionExit::Reconnect;
        }
    };

    let ws_url = format!("{}?v=10&encoding=json", gateway_url);
    let (socket, _) = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(target: "bot::discord", "failed to connect gateway: {}", error);
            return DiscordSessionExit::Reconnect;
        }
    };
    let (mut write, mut read) = socket.split();
    let mut sequence: Option<i64> = None;

    let heartbeat_interval_ms = loop {
        tokio::select! {
            _ = &mut *shutdown_rx => return DiscordSessionExit::Shutdown,
            message = read.next() => {
                let Some(Ok(WsMessage::Text(text))) = message else {
                    return DiscordSessionExit::Reconnect;
                };
                let Ok(payload) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if payload.get("op").and_then(Value::as_i64) == Some(10) {
                    break payload
                        .get("d")
                        .and_then(|d| d.get("heartbeat_interval"))
                        .and_then(Value::as_u64)
                        .unwrap_or(45_000);
                }
            }
        }
    };

    let identify = json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": 37377,
            "properties": {
                "os": "windows",
                "browser": "kokoro-engine",
                "device": "kokoro-engine"
            }
        }
    });
    if write
        .send(WsMessage::Text(identify.to_string().into()))
        .await
        .is_err()
    {
        return DiscordSessionExit::Reconnect;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return DiscordSessionExit::Shutdown;
            }
            _ = heartbeat.tick() => {
                let heartbeat_payload = json!({ "op": 1, "d": sequence });
                if write
                    .send(WsMessage::Text(heartbeat_payload.to_string().into()))
                    .await
                    .is_err()
                {
                    return DiscordSessionExit::Reconnect;
                }
            }
            message = read.next() => {
                let Some(message) = message else {
                    return DiscordSessionExit::Reconnect;
                };
                let Ok(message) = message else {
                    return DiscordSessionExit::Reconnect;
                };
                let WsMessage::Text(text) = message else {
                    continue;
                };
                let Ok(payload) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(seq) = payload.get("s").and_then(Value::as_i64) {
                    sequence = Some(seq);
                }
                if payload.get("op").and_then(Value::as_i64) == Some(7) {
                    return DiscordSessionExit::Reconnect;
                }
                if payload.get("op").and_then(Value::as_i64) == Some(0)
                    && payload.get("t").and_then(Value::as_str) == Some("MESSAGE_CREATE")
                {
                    handle_discord_message(client, token, &config, &services, payload.get("d").cloned().unwrap_or(Value::Null)).await;
                }
            }
        }
    }
}

async fn fetch_discord_gateway(client: &reqwest::Client, token: &str) -> Result<String, String> {
    let response = client
        .get("https://discord.com/api/v10/gateway/bot")
        .header("Authorization", format!("Bot {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Discord gateway endpoint returned {}",
            response.status()
        ));
    }
    let json: Value = response.json().await.map_err(|e| e.to_string())?;
    json.get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("Discord gateway response did not include url".to_string())
}

async fn handle_discord_message(
    client: &reqwest::Client,
    token: &str,
    config: &Arc<RwLock<BotConfig>>,
    services: &BotChatServices,
    message: Value,
) {
    if message
        .get("author")
        .and_then(|author| author.get("bot"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }

    let channel_id = message
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if channel_id.is_empty() {
        return;
    }

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let cfg = config.read().await.discord.clone();
    let is_direct_message = message.get("guild_id").and_then(Value::as_str).is_none();
    if !discord_message_allowed_by_scope(&cfg, &channel_id, is_direct_message) {
        return;
    }

    let author_id = message
        .get("author")
        .and_then(|author| author.get("id"))
        .and_then(Value::as_str);
    let conversation_key = author_id
        .map(|author| format!("{}:{}", channel_id, author))
        .unwrap_or_else(|| channel_id.clone());

    let chat = match services.chat() {
        Ok(chat) => chat,
        Err(error) => {
            tracing::warn!(target: "bot::discord", "{error}");
            return;
        }
    };
    let state = chat.orchestrator.as_ref();
    let binding = match BotRequestBinding::new(
        state,
        "discord",
        cfg.character_id.as_deref(),
        Some(&conversation_key),
        message.get("id").and_then(Value::as_str),
    )
    .await
    {
        Ok(binding) => binding,
        Err(error) => {
            tracing::error!(target:"bot::discord", "{error}");
            return;
        }
    };
    let attachments = message.get("attachments").and_then(Value::as_array);
    let has_media = attachments.is_some_and(|items| {
        items.iter().any(|attachment| {
            let mime = attachment.get("content_type").and_then(Value::as_str);
            let name = attachment.get("filename").and_then(Value::as_str);
            is_image_media(mime, name) || is_audio_media(mime, name)
        })
    });
    let input = if has_media {
        match prepare_bot_media_input(state, &binding, &content, || async {
            let mut images = Vec::new();
            let mut text = content.clone();
            for attachment in attachments.into_iter().flatten() {
                let filename = attachment.get("filename").and_then(Value::as_str);
                let content_type = attachment.get("content_type").and_then(Value::as_str);
                if !is_image_media(content_type, filename)
                    && !is_audio_media(content_type, filename)
                {
                    continue;
                }
                let url = attachment
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.is_empty())
                    .ok_or_else(|| {
                        KokoroError::Validation("Missing Discord attachment URL".into())
                    })?;
                let (data, downloaded_type) = download_url_bytes(client, url, None)
                    .await
                    .map_err(KokoroError::Chat)?;
                if is_image_media(content_type, filename) {
                    let mime = downloaded_type
                        .as_deref()
                        .or(content_type)
                        .unwrap_or_else(|| image_mime_from_name(filename.unwrap_or_default()));
                    images.push(image_data_url(&data, mime));
                } else {
                    let format = audio_format_from_mime_or_name(
                        downloaded_type.as_deref().or(content_type),
                        filename,
                    );
                    let transcript = transcribe_bot_audio(services, "discord", data, format)
                        .await
                        .map_err(KokoroError::Stt)?
                        .ok_or_else(|| KokoroError::Stt("Could not recognize speech".into()))?;
                    text = text_with_transcription(text, Some(transcript));
                }
            }
            if text.trim().is_empty() && !images.is_empty() {
                text = "The user sent an image:".into();
            }
            Ok((text, images))
        })
        .await
        {
            Ok(input) => input,
            Err(error) => {
                tracing::error!(target:"bot::discord", "Failed to prepare or recover Discord media: {error}");
                return;
            }
        }
    } else {
        BotMediaInput {
            text: content,
            images: vec![],
            input_text: None,
        }
    };
    if input.text.trim().is_empty() && input.images.is_empty() {
        return;
    }

    match submit_bound_bot_reply(
        services,
        "discord",
        BotInput {
            text: input.text,
            images: input.images,
            ..Default::default()
        },
        &binding,
        input.input_text.as_deref(),
    )
    .await
    {
        Ok(reply) => {
            let reply_text = reply_text_with_translation(&reply);
            if !reply_text.trim().is_empty() {
                if let Err(error) =
                    send_discord_message(client, token, &channel_id, &reply_text).await
                {
                    tracing::error!(target: "bot::discord", "failed to send Discord message: {}", error);
                }
            }
            for image in &reply.images {
                match bot_reply_image_bytes(image) {
                    Ok(data) => {
                        if let Err(error) = send_discord_file_message(
                            client,
                            token,
                            &channel_id,
                            "",
                            &image.file_name,
                            &image.mime_type,
                            data,
                        )
                        .await
                        {
                            tracing::error!(target: "bot::discord", "failed to send Discord tool image: {}", error);
                        }
                    }
                    Err(error) => {
                        tracing::error!(target: "bot::discord", "failed to decode Discord tool image: {}", error);
                    }
                }
            }
            let generated_images = generate_bot_images(services, "discord", &reply).await;
            for image in generated_images {
                if let Err(error) = send_discord_file_message(
                    client,
                    token,
                    &channel_id,
                    "",
                    &image.file_name,
                    &image.mime_type,
                    image.data,
                )
                .await
                {
                    tracing::error!(target: "bot::discord", "failed to send Discord image: {}", error);
                }
            }
            if cfg.send_voice_reply {
                match synthesize_bot_audio(&reply.reply, reply.speech_snapshot.as_deref()).await {
                    Ok(Some(audio)) => {
                        if let Err(error) = send_discord_file_message(
                            client,
                            token,
                            &channel_id,
                            "",
                            audio_metadata(&audio).1,
                            audio_metadata(&audio).0,
                            audio,
                        )
                        .await
                        {
                            tracing::error!(target: "bot::discord", "failed to send Discord voice reply: {}", error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::error!(target: "bot::discord", "failed to synthesize Discord voice reply: {}", error);
                    }
                }
            }
        }
        Err(error) => {
            tracing::error!(target: "bot::discord", "failed to generate Discord reply: {}", error);
        }
    }
}

fn discord_message_allowed_by_scope(
    cfg: &DiscordBotConfig,
    channel_id: &str,
    is_direct_message: bool,
) -> bool {
    if is_direct_message {
        return cfg.allow_direct_messages;
    }
    !cfg.allowed_channel_ids.is_empty() && cfg.allowed_channel_ids.iter().any(|id| id == channel_id)
}

async fn send_discord_message(
    client: &reqwest::Client,
    token: &str,
    channel_id: &str,
    text: &str,
) -> Result<(), String> {
    let response = client
        .post(format!(
            "https://discord.com/api/v10/channels/{}/messages",
            channel_id
        ))
        .header("Authorization", format!("Bot {}", token))
        .json(&json!({ "content": truncate_for_platform(text, 1900) }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Discord API returned {}", response.status()));
    }
    Ok(())
}

async fn send_discord_file_message(
    client: &reqwest::Client,
    token: &str,
    channel_id: &str,
    text: &str,
    file_name: &str,
    mime_type: &str,
    data: Vec<u8>,
) -> Result<(), String> {
    let part = reqwest::multipart::Part::bytes(data)
        .file_name(file_name.to_string())
        .mime_str(mime_type)
        .map_err(|error| error.to_string())?;
    let payload = json!({
        "content": truncate_for_platform(text, 1900),
    });
    let form = reqwest::multipart::Form::new()
        .text("payload_json", payload.to_string())
        .part("files[0]", part);
    let response = client
        .post(format!(
            "https://discord.com/api/v10/channels/{}/messages",
            channel_id
        ))
        .header("Authorization", format!("Bot {}", token))
        .multipart(form)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Discord API returned {}", response.status()));
    }
    Ok(())
}

fn truncate_for_platform(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn webhook_transport_keeps_authentication_when_chat_is_unavailable() {
        let services = BotChatServices::new(None, None, None, None, Arc::new(|_: BotEvent| Ok(())));
        let config = WebhookBotConfig {
            enabled: true,
            bind_host: "127.0.0.1".into(),
            port: 0,
            bearer_token: Some("fixture-token".into()),
            ..Default::default()
        };
        let runtime = BotRuntimeService::new(BotConfig {
            webhook: config.clone(),
            ..Default::default()
        });
        runtime
            .start_platform("webhook", services.clone())
            .await
            .unwrap();
        assert!(runtime.is_http_running().await);

        let body = bytes::Bytes::from_static(br#"{"text":"hello","request_id":"example"}"#);
        let unauthorized =
            handle_generic_webhook(config.clone(), None, body.clone(), &services).await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let unavailable =
            handle_generic_webhook(config, Some("Bearer fixture-token".into()), body, &services)
                .await;
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = warp::hyper::body::to_bytes(unavailable.into_body())
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"error": "Kokoro Engine chat is unavailable."}),
        );
        runtime.stop_platform("webhook").await.unwrap();
        assert!(!runtime.is_http_running().await);
    }

    #[derive(Default)]
    struct HeadlessProvider(std::sync::atomic::AtomicUsize);

    type TextStream = std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>;

    #[async_trait::async_trait]
    impl crate::llm::provider::LlmProvider for HeadlessProvider {
        async fn chat(
            &self,
            _: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
            _: Option<crate::llm::provider::LlmParams>,
        ) -> Result<String, String> {
            Ok("{}".into())
        }

        async fn chat_stream(
            &self,
            _: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
            _: Option<crate::llm::provider::LlmParams>,
        ) -> Result<TextStream, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(futures::stream::iter([
                Ok("fixture reply".into()),
            ])))
        }

        fn id(&self) -> &str {
            "headless-bot-fixture"
        }
    }

    #[tokio::test]
    async fn headless_webhook_and_telegram_share_persisted_turns() {
        use crate::chat::persistence::{ExternalReplyImage, ExternalReplyMedia};
        use crate::llm::service::LlmService;
        use crate::telegram::bot::{submit_message, SessionMode};
        use teloxide::types::ChatId;

        let temp = tempfile::tempdir().unwrap();
        let state = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
        state.set_memory_enabled(false).await;
        sqlx::query("INSERT INTO characters(id, name) VALUES ('bot-fixture', 'Bot Fixture')")
            .execute(&state.db)
            .await
            .unwrap();
        let provider = Arc::new(HeadlessProvider::default());
        let llm = LlmService::for_test(provider.clone());
        let chat = ChatService::for_test(state.clone(), llm.clone(), temp.path()).await;
        let services = BotChatServices::new(
            Some((chat, state.clone())),
            None,
            None,
            None,
            Arc::new(|_: BotEvent| Ok(())),
        );
        let request = json!({
            "request_id": "1",
            "character_id": "bot-fixture",
            "conversation_id": "shared",
            "text": "first from webhook",
        });
        let call = |services: BotChatServices, request: Value| async move {
            let response = handle_generic_webhook(
                WebhookBotConfig {
                    bearer_token: Some("fixture-token".into()),
                    ..Default::default()
                },
                Some("Bearer fixture-token".into()),
                request.to_string().into(),
                &services,
            )
            .await;
            let status = response.status();
            let body = warp::hyper::body::to_bytes(response.into_body())
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&body).unwrap())
        };
        let (status, first) = call(services.clone(), request.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["reply"], "fixture reply");
        assert_eq!(first["character_id"], "bot-fixture");
        let conversation = first["conversation_id"].as_str().unwrap();
        let mode = SessionMode::Desktop {
            character_id: "bot-fixture".into(),
            conversation_id: conversation.into(),
        };
        let telegram = submit_message(
            &services.chat().unwrap().service,
            ChatId(42),
            1,
            "second from telegram",
            vec![],
            &mode,
            "bot-fixture",
        )
        .await
        .unwrap();
        assert_eq!(telegram.status.as_deref(), Some("completed"));
        assert_eq!(telegram.conversation_id, conversation);
        assert_eq!(telegram.text, "fixture reply");
        assert_ne!(
            telegram.client_request_id.as_deref(),
            first["request_id"].as_str()
        );
        assert!(telegram.assistant_message_id.is_some());

        // Simulate output media already saved for this request. A fresh
        // journal must replay the persisted bytes without requesting the LLM.
        let media = ExternalReplyMedia {
            images: vec![ExternalReplyImage {
                prompt: "stored image".into(),
                mime_type: "image/png".into(),
                file_name: "stored.png".into(),
                data_base64: "AQID".into(),
            }],
            ..Default::default()
        };
        sqlx::query("UPDATE conversation_messages SET metadata=json_set(metadata,'$.external_reply_media',json(?)) WHERE conversation_id=? AND role='assistant' AND json_extract(metadata,'$.chat_save_id')=?")
            .bind(serde_json::to_string(&media).unwrap())
            .bind(conversation)
            .bind(format!("{}:assistant", first["request_id"].as_str().unwrap()))
            .execute(&state.db)
            .await
            .unwrap();
        let restarted = BotChatServices {
            chat: Some(BotChatAccess {
                service: ChatService::for_test(state.clone(), llm, temp.path()).await,
                orchestrator: state.clone(),
            }),
            ..services
        };
        let (status, replay) = call(restarted, request).await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        let mut expected = first.clone();
        expected["images"] = serde_json::to_value(media.images).unwrap();
        assert_eq!(replay, expected);
        assert_eq!(provider.0.load(Ordering::SeqCst), 2);
        let transcript: Vec<(String, String)> = sqlx::query_as(
            "SELECT role, content FROM conversation_messages WHERE conversation_id=? ORDER BY id",
        )
        .bind(conversation)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(
            transcript
                .iter()
                .map(|(role, content)| (role.as_str(), content.as_str()))
                .collect::<Vec<_>>(),
            [
                ("user", "first from webhook"),
                ("assistant", "fixture reply"),
                ("user", "second from telegram"),
                ("assistant", "fixture reply"),
            ]
        );
    }

    #[tokio::test]
    async fn media_replay_skips_preparation_and_keeps_text_identity_for_all_generic_platforms() {
        use super::*;
        for platform in ["webhook", "line", "discord"] {
            for fail_save in [false, true] {
                let state = AIOrchestrator::new("sqlite::memory:")
                    .await
                    .unwrap()
                    .clone();
                let binding = BotRequestBinding::new(
                    &state,
                    platform,
                    Some("default"),
                    Some("chat"),
                    Some("message"),
                )
                .await
                .unwrap();
                let prepared = prepare_bot_media_input(&state, &binding, "caption", || async {
                    Ok((
                        "caption\n\nVoice message transcript: Original, transcript!".into(),
                        vec!["image".into()],
                    ))
                })
                .await
                .unwrap();
                let turn = ExternalTurn::begin_with_input_text(
                    &state,
                    "default",
                    binding.conversation_id.clone(),
                    binding.request_id.clone(),
                    &prepared.text,
                    false,
                    prepared.input_text.as_deref(),
                )
                .await
                .unwrap();
                if fail_save {
                    sqlx::query("CREATE TRIGGER fail_media_reply BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'final failure'); END").execute(&state.db).await.unwrap();
                }
                assert_eq!(
                    turn.save_reply(&state, "reply", None).await.is_err(),
                    fail_save
                );
                if fail_save {
                    sqlx::query("DROP TRIGGER fail_media_reply")
                        .execute(&state.db)
                        .await
                        .unwrap();
                }
                for _ in 0..2 {
                    let replay = prepare_bot_media_input(&state, &binding, "caption", || async {
                        panic!("saved input must bypass STT and attachment download")
                    })
                    .await
                    .unwrap();
                    assert_eq!(replay.text, prepared.text);
                    assert!(replay.images.is_empty());
                    let turn = ExternalTurn::begin_with_input_text(
                        &state,
                        "default",
                        binding.conversation_id.clone(),
                        binding.request_id.clone(),
                        &replay.text,
                        false,
                        replay.input_text.as_deref(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(turn.previous_reply.unwrap().0, "reply");
                }
                assert!(matches!(
                    prepare_bot_media_input(&state, &binding, "edited caption", || async {
                        panic!("conflict before preparation")
                    })
                    .await,
                    Err(KokoroError::Validation(_))
                ));
                let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
                    .fetch_one(&state.db)
                    .await
                    .unwrap();
                assert_eq!(rows, 2);
                // Text-only requests retain their original full-content check.
                assert!(ExternalTurn::begin(
                    &state,
                    "default",
                    binding.conversation_id.clone(),
                    binding.request_id.clone(),
                    "different text",
                    false
                )
                .await
                .is_err());
            }
        }
    }

    #[tokio::test]
    async fn failed_media_preparation_does_not_admit_identity_and_lookup_is_scoped() {
        use super::*;
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let binding = BotRequestBinding::new(
            &state,
            "webhook",
            Some("default"),
            Some("chat"),
            Some("message"),
        )
        .await
        .unwrap();
        assert!(prepare_bot_media_input(&state, &binding, "", || async {
            Err(KokoroError::Stt("unavailable".into()))
        })
        .await
        .is_err());
        assert!(
            ExternalTurn::saved_user_message(&state, "default", &binding.request_id)
                .await
                .unwrap()
                .is_none()
        );
        let prepared = prepare_bot_media_input(&state, &binding, "", || async {
            Ok(("transcript".into(), vec![]))
        })
        .await
        .unwrap();
        ExternalTurn::begin_with_input_text(
            &state,
            "default",
            binding.conversation_id.clone(),
            binding.request_id.clone(),
            &prepared.text,
            false,
            prepared.input_text.as_deref(),
        )
        .await
        .unwrap();
        assert!(matches!(
            BotRequestBinding::new(&state, "webhook", Some("other"), Some("chat"), Some("message")).await,
            Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:")
        ));
        for (character, chat, request) in [
            ("default", "other", "message"),
            ("default", "chat", "other"),
        ] {
            let other = BotRequestBinding::new(
                &state,
                "webhook",
                Some(character),
                Some(chat),
                Some(request),
            )
            .await
            .unwrap();
            let input = prepare_bot_media_input(&state, &other, "", || async {
                Ok(("new input".into(), vec![]))
            })
            .await
            .unwrap();
            assert_eq!(input.text, "new input");
        }
        sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES (?,'user','legacy transcript',?,'now')")
            .bind(&binding.conversation_id)
            .bind(serde_json::json!({"source":"webhook","client_request_id":persistence::legacy_external_identity("webhook", "chat", "legacy")}).to_string())
            .execute(&state.db).await.unwrap();
        // Binding precedes the download/transcription callback for every adapter.
        for character in ["default", "other"] {
            assert!(matches!(
                BotRequestBinding::new(&state, "webhook", Some(character), Some("chat"), Some("legacy")).await,
                Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:")
            ));
        }
        sqlx::query("DROP TABLE conversation_messages")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(matches!(
            prepare_bot_media_input(&state, &binding, "", || async {
                panic!("lookup failure must not start media")
            })
            .await,
            Err(KokoroError::ChatPersistence(_))
        ));
    }

    use super::*;

    #[test]
    fn bot_config_defaults_cover_all_non_telegram_platforms() {
        let config = BotConfig::default();
        assert_eq!(config.selected_platform, "telegram");
        assert_eq!(
            config.telegram.bot_token_env.as_deref(),
            Some("TELEGRAM_BOT_TOKEN")
        );
        assert_eq!(
            config.discord.bot_token_env.as_deref(),
            Some("DISCORD_BOT_TOKEN")
        );
        assert!(!config.discord.send_voice_reply);
        assert_eq!(
            config.line.channel_access_token_env.as_deref(),
            Some("LINE_CHANNEL_ACCESS_TOKEN")
        );
        assert_eq!(config.line.webhook_path, "/line/webhook");
        assert_eq!(config.webhook.bind_host, "127.0.0.1");
        assert_eq!(config.webhook.port, 8787);
        assert!(!config.webhook.send_voice_reply);
    }

    #[test]
    fn bot_config_deserializes_partial_files_with_defaults() {
        let config: BotConfig = serde_json::from_str(r#"{"selected_platform":"discord"}"#).unwrap();
        assert_eq!(config.selected_platform, "discord");
        assert!(config.discord.allow_direct_messages);
        assert!(!config.discord.send_voice_reply);
        assert_eq!(config.webhook.endpoint_path, "/webhook/message");
        assert!(!config.webhook.send_voice_reply);
    }

    #[test]
    fn bot_config_normalization_enforces_fixed_env_names() {
        let mut config = BotConfig::default();
        config.telegram.bot_token_env = Some("CUSTOM_TELEGRAM".to_string());
        config.discord.bot_token_env = Some("CUSTOM_DISCORD".to_string());
        config.line.channel_access_token_env = Some("CUSTOM_LINE_TOKEN".to_string());
        config.line.channel_secret_env = Some("CUSTOM_LINE_SECRET".to_string());
        config.webhook.bearer_token_env = Some("CUSTOM_WEBHOOK".to_string());

        let normalized = normalize_bot_config_envs(config);

        assert_eq!(
            normalized.telegram.bot_token_env.as_deref(),
            Some(TELEGRAM_BOT_TOKEN_ENV)
        );
        assert_eq!(
            normalized.discord.bot_token_env.as_deref(),
            Some(DISCORD_BOT_TOKEN_ENV)
        );
        assert_eq!(
            normalized.line.channel_access_token_env.as_deref(),
            Some(LINE_CHANNEL_ACCESS_TOKEN_ENV)
        );
        assert_eq!(
            normalized.line.channel_secret_env.as_deref(),
            Some(LINE_CHANNEL_SECRET_ENV)
        );
        assert_eq!(
            normalized.webhook.bearer_token_env.as_deref(),
            Some(WEBHOOK_BEARER_TOKEN_ENV)
        );
    }

    #[test]
    fn load_bot_config_migrates_legacy_telegram_and_removes_old_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bot_path = temp_dir.path().join("bot_config.json");
        let legacy_path = temp_dir.path().join("telegram_config.json");
        let legacy = crate::telegram::TelegramConfig {
            enabled: true,
            bot_token: Some("legacy-token".to_string()),
            bot_token_env: Some("CUSTOM_TELEGRAM_ENV".to_string()),
            allowed_chat_ids: vec![12345],
            send_voice_reply: true,
            character_id: Some("hiyori".to_string()),
        };

        std::fs::write(
            &legacy_path,
            serde_json::to_string(&legacy).expect("legacy should serialize"),
        )
        .expect("write legacy config");

        let config = load_bot_config_from_paths(&bot_path, &legacy_path);

        assert!(bot_path.exists());
        assert!(!legacy_path.exists());
        assert_eq!(config.telegram.enabled, true);
        assert_eq!(config.telegram.bot_token.as_deref(), Some("legacy-token"));
        assert_eq!(
            config.telegram.bot_token_env.as_deref(),
            Some(TELEGRAM_BOT_TOKEN_ENV)
        );
        assert_eq!(config.telegram.allowed_chat_ids, vec![12345]);
        assert_eq!(config.telegram.send_voice_reply, true);
        assert_eq!(config.telegram.character_id.as_deref(), Some("hiyori"));

        let saved: BotConfig = serde_json::from_str(
            &std::fs::read_to_string(&bot_path).expect("read saved bot config"),
        )
        .expect("saved bot config should deserialize");
        assert_eq!(saved.telegram, config.telegram);
    }

    #[test]
    fn load_bot_config_keeps_existing_telegram_when_already_configured() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bot_path = temp_dir.path().join("bot_config.json");
        let legacy_path = temp_dir.path().join("telegram_config.json");
        let mut current = BotConfig::default();
        current.telegram.bot_token = Some("current-token".to_string());
        save_bot_config_file_to_path(&bot_path, &current).expect("save current bot config");
        std::fs::write(&legacy_path, "{not json").expect("write invalid legacy config");

        let config = load_bot_config_from_paths(&bot_path, &legacy_path);

        assert_eq!(config.telegram.bot_token.as_deref(), Some("current-token"));
        assert!(legacy_path.exists());
    }

    #[test]
    fn bot_telegram_missing_or_invalid_config_never_grants_a_default_chat() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bot_config.json");
        let legacy = temp.path().join("telegram_config.json");
        for contents in [
            None,
            Some("{}"),
            Some(r#"{"telegram":{"enabled":true}}"#),
            Some("not json"),
            Some(r#"{"telegram":{"allowed_chat_ids":["*"]}}"#),
        ] {
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            assert!(load_bot_config_from_paths(&path, &legacy)
                .telegram
                .allowed_chat_ids
                .is_empty());
        }
    }

    #[test]
    fn bot_telegram_running_with_empty_whitelist_does_not_restore_legacy_grants() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bot_config.json");
        let legacy = temp.path().join("telegram_config.json");
        let mut current = BotConfig::default();
        current.telegram.enabled = true;
        save_bot_config_file_to_path(&path, &current).unwrap();
        std::fs::write(&legacy, r#"{"allowed_chat_ids":[123,-1001234567890]}"#).unwrap();

        assert!(load_bot_config_from_paths(&path, &legacy)
            .telegram
            .allowed_chat_ids
            .is_empty());
        assert!(legacy.exists());
    }

    #[test]
    fn bot_normalize_image_reference_accepts_urls_and_wraps_base64() {
        assert_eq!(
            normalize_image_reference("https://example.com/a.png", "image/png").as_deref(),
            Some("https://example.com/a.png")
        );
        assert_eq!(
            normalize_image_reference("abcd", "image/png").as_deref(),
            Some("data:image/png;base64,abcd")
        );
    }

    #[test]
    fn bot_discord_empty_channel_allowlist_allows_dm_only() {
        let mut config = DiscordBotConfig::default();
        assert!(discord_message_allowed_by_scope(&config, "123", true));
        assert!(!discord_message_allowed_by_scope(&config, "123", false));

        config.allow_direct_messages = false;
        assert!(!discord_message_allowed_by_scope(&config, "123", true));

        config.allowed_channel_ids = vec!["123".to_string()];
        assert!(discord_message_allowed_by_scope(&config, "123", false));
        assert!(!discord_message_allowed_by_scope(&config, "999", false));
    }

    #[test]
    fn webhook_character_resolution_prefers_request_then_config_then_active() {
        assert_eq!(
            resolve_webhook_character_id(Some("request"), Some("configured"), Some("active")),
            "request"
        );
        assert_eq!(
            resolve_webhook_character_id(None, Some("configured"), Some("active")),
            "configured"
        );
        assert_eq!(
            resolve_webhook_character_id(None, None, Some("active")),
            "active"
        );
    }

    #[test]
    fn webhook_bearer_auth_requires_exact_bearer_token() {
        assert!(verify_webhook_bearer(Some("Bearer secret"), Some("secret")).is_ok());
        assert_eq!(
            verify_webhook_bearer(None, Some("secret")).unwrap_err(),
            "Missing Authorization header"
        );
        assert_eq!(
            verify_webhook_bearer(Some("Basic secret"), Some("secret")).unwrap_err(),
            "Invalid bearer token"
        );
        assert!(verify_webhook_bearer(None, None).is_err());
    }

    #[test]
    fn review_r18_webhook_without_configured_token_must_reject_requests() {
        assert!(verify_webhook_bearer(None, None).is_err());
        assert!(verify_webhook_bearer(Some("Bearer test-value"), Some("  ")).is_err());
    }

    #[test]
    fn review_r18_webhook_sessions_must_separate_platforms() {
        let first = webhook_conversation_key(
            Some("private"),
            Some("same-user"),
            Some("same-user"),
            Some("platform-a"),
        );
        let second = webhook_conversation_key(
            Some("private"),
            Some("same-user"),
            Some("same-user"),
            Some("platform-b"),
        );
        assert_ne!(
            first, second,
            "platform identity must participate in session identity"
        );
    }

    #[test]
    fn review_r18_explicit_private_session_must_not_be_shadowed_by_sender() {
        let first = webhook_conversation_key(
            Some("private"),
            Some("session-a"),
            Some("same-user"),
            Some("astrbot"),
        );
        let second = webhook_conversation_key(
            Some("private"),
            Some("session-b"),
            Some("same-user"),
            Some("astrbot"),
        );
        assert_ne!(
            first, second,
            "AstrBot platform_session must reach persistence identity"
        );
    }

    #[test]
    fn webhook_payload_parser_normalizes_text_images_and_audio() {
        let parsed = parse_generic_webhook_payload(
            br#"{
                "message": "  hello  ",
                "images": ["https://example.com/photo.png", "abcd"],
                "image_mime_type": "image/png",
                "audio_base64": "aGVsbG8=",
                "audio_format": "wav",
                "conversation_type": "private",
                "user_id": "user-7"
            }"#,
        )
        .expect("valid webhook payload");

        assert_eq!(parsed.text, "hello");
        assert_eq!(
            parsed.images,
            vec![
                "https://example.com/photo.png".to_string(),
                "data:image/png;base64,abcd".to_string()
            ]
        );
        assert_eq!(parsed.audio.as_deref(), Some(&b"hello"[..]));
        assert_eq!(parsed.audio_format.as_deref(), Some("wav"));
        assert_eq!(
            parsed.conversation_key.as_deref(),
            Some("private:unknown:user-7")
        );
    }

    #[test]
    fn webhook_payload_parser_maps_group_identity_before_persistence() {
        let parsed = parse_generic_webhook_payload(
            br#"{
                "text": "hello",
                "conversation_type": "group",
                "conversation_id": "group-42",
                "user_id": "user-7"
            }"#,
        )
        .expect("valid group webhook payload");

        assert_eq!(
            parsed.conversation_key.as_deref(),
            Some("group:unknown:group-42")
        );
        assert_eq!(
            map_webhook_conversation_to_character("char-2", parsed.conversation_key.as_deref()),
            "char-2:group:unknown:group-42"
        );
    }

    #[test]
    fn webhook_payload_parser_falls_back_to_conversation_when_user_is_blank() {
        let parsed = parse_generic_webhook_payload(
            br#"{
                "text": "hello",
                "conversation_type": "private",
                "conversation_id": "private-session-9",
                "user_id": "   "
            }"#,
        )
        .expect("valid private webhook payload");

        assert_eq!(
            parsed.conversation_key.as_deref(),
            Some("private:unknown:private-session-9")
        );
    }

    #[test]
    fn webhook_payload_parser_returns_json_error_for_malformed_input() {
        let error = parse_generic_webhook_payload(br#"{"text":"unterminated}"#)
            .expect_err("malformed JSON must be rejected");

        assert!(error.starts_with("Invalid JSON:"));
    }

    #[test]
    fn webhook_payload_parser_rejects_empty_text_without_media() {
        let error = parse_generic_webhook_payload(br#"{"text":"   "}"#)
            .expect_err("empty webhook messages must be rejected");

        assert_eq!(error, "Missing text or media");
    }

    #[test]
    fn webhook_payload_parser_rejects_invalid_image_base64() {
        let error = parse_generic_webhook_payload(
            br#"{"image_base64":"not*base64","image_mime_type":"image/png"}"#,
        )
        .expect_err("invalid image base64 must be rejected");

        assert!(error.starts_with("Invalid image base64:"));
    }

    #[test]
    fn webhook_body_limit_rejects_payloads_over_four_megabytes() {
        let body = vec![b'x'; MAX_GENERIC_WEBHOOK_BODY_BYTES + 1];

        assert_eq!(
            validate_generic_webhook_body(&body).unwrap_err(),
            "Webhook body exceeds the 4194304 byte limit"
        );
    }
}
