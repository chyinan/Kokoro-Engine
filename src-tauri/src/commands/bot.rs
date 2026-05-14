use crate::actions::tool_settings::ToolSettings;
use crate::actions::{execute_tool_calls, ToolInvocation};
use crate::ai::context::AIOrchestrator;
use crate::ai::memory_event_ingress::{
    build_cooldown_key, select_memory_ingress_decision, should_use_structured_extraction,
    MemoryEventIngressOptions,
};
use crate::ai::memory_extractor;
use crate::error::KokoroError;
use crate::llm::messages::{
    assistant_text_message, is_user_message, role_text_message, system_message, user_text_message,
};
use crate::llm::service::LlmService;
use crate::telegram::TelegramService;
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::sync::{oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use warp::{http::StatusCode, Filter, Reply};

type HmacSha256 = Hmac<Sha256>;

pub(crate) const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
const LINE_CHANNEL_ACCESS_TOKEN_ENV: &str = "LINE_CHANNEL_ACCESS_TOKEN";
const LINE_CHANNEL_SECRET_ENV: &str = "LINE_CHANNEL_SECRET";
const WEBHOOK_BEARER_TOKEN_ENV: &str = "KOKORO_WEBHOOK_TOKEN";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BotConfig {
    pub selected_platform: String,
    pub telegram: crate::telegram::TelegramConfig,
    pub discord: DiscordBotConfig,
    pub line: LineBotConfig,
    pub webhook: WebhookBotConfig,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            selected_platform: "telegram".to_string(),
            telegram: crate::telegram::TelegramConfig::default(),
            discord: DiscordBotConfig::default(),
            line: LineBotConfig::default(),
            webhook: WebhookBotConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DiscordBotConfig {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub bot_token_env: Option<String>,
    pub allowed_channel_ids: Vec<String>,
    pub allow_direct_messages: bool,
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
            character_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WebhookBotConfig {
    pub enabled: bool,
    pub bind_host: String,
    pub port: u16,
    pub endpoint_path: String,
    pub bearer_token: Option<String>,
    pub bearer_token_env: Option<String>,
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
            character_id: None,
        }
    }
}

#[derive(Clone)]
pub struct BotRuntimeService {
    config: Arc<RwLock<BotConfig>>,
    discord_shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    http_shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
}

impl BotRuntimeService {
    pub fn new(config: BotConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            discord_shutdown_tx: Arc::new(RwLock::new(None)),
            http_shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn update_config(&self, config: BotConfig) {
        *self.config.write().await = config;
    }

    pub async fn current_config(&self) -> BotConfig {
        self.config.read().await.clone()
    }

    pub async fn is_discord_running(&self) -> bool {
        self.discord_shutdown_tx.read().await.is_some()
    }

    pub async fn is_http_running(&self) -> bool {
        self.http_shutdown_tx.read().await.is_some()
    }

    pub async fn start_enabled(&self, app: tauri::AppHandle) {
        let config = self.config.read().await.clone();
        if config.discord.enabled {
            if let Err(error) = self.start_discord(app.clone()).await {
                tracing::error!(target: "bot::discord", "failed to auto-start Discord bot: {}", error);
            }
        }
        if config.line.enabled || config.webhook.enabled {
            if let Err(error) = self.start_http(app).await {
                tracing::error!(target: "bot::http", "failed to auto-start Bot HTTP server: {}", error);
            }
        }
    }

    pub async fn start_platform(
        &self,
        platform: &str,
        app: tauri::AppHandle,
    ) -> Result<(), String> {
        match platform {
            "discord" => self.start_discord(app).await,
            "line" | "webhook" => self.start_http(app).await,
            other => Err(format!("Unsupported bot platform runtime: {}", other)),
        }
    }

    pub async fn stop_platform(&self, platform: &str) -> Result<(), String> {
        match platform {
            "discord" => self.stop_discord().await,
            "line" | "webhook" => self.stop_http().await,
            other => Err(format!("Unsupported bot platform runtime: {}", other)),
        }
    }

    async fn start_discord(&self, app: tauri::AppHandle) -> Result<(), String> {
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
        tauri::async_runtime::spawn(async move {
            run_discord_gateway(token, config_ref, app, rx).await;
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

    async fn start_http(&self, app: tauri::AppHandle) -> Result<(), String> {
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
        tauri::async_runtime::spawn(async move {
            run_http_bot_server(addr, config_ref, app, rx).await;
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

#[derive(Debug, Serialize)]
pub struct BotStatus {
    pub telegram: BotPlatformStatus,
    pub discord: BotPlatformStatus,
    pub line: BotPlatformStatus,
    pub webhook: BotPlatformStatus,
}

#[derive(Debug, Serialize)]
pub struct BotPlatformStatus {
    pub enabled: bool,
    pub configured: bool,
    pub running: bool,
}

fn app_data_dir() -> PathBuf {
    dirs_next::data_dir()
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

#[tauri::command]
pub async fn get_bot_config() -> Result<BotConfig, KokoroError> {
    Ok(load_bot_config())
}

#[tauri::command]
pub async fn save_bot_config(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
    config: BotConfig,
) -> Result<(), KokoroError> {
    let config = normalize_bot_config_envs(config);
    save_bot_config_file(&config)?;
    state.update_config(config.telegram.clone()).await;
    runtime.update_config(config).await;
    Ok(())
}

#[tauri::command]
pub async fn start_bot_platform(
    runtime: State<'_, BotRuntimeService>,
    app: tauri::AppHandle,
    platform: String,
) -> Result<(), KokoroError> {
    runtime
        .start_platform(&platform, app)
        .await
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn stop_bot_platform(
    runtime: State<'_, BotRuntimeService>,
    platform: String,
) -> Result<(), KokoroError> {
    runtime
        .stop_platform(&platform)
        .await
        .map_err(KokoroError::ExternalService)
}

#[tauri::command]
pub async fn get_bot_status(
    state: State<'_, TelegramService>,
    runtime: State<'_, BotRuntimeService>,
) -> Result<BotStatus, KokoroError> {
    let config = runtime.current_config().await;
    let telegram_config = state.get_config().await;
    let discord_running = runtime.is_discord_running().await;
    let http_running = runtime.is_http_running().await;
    Ok(BotStatus {
        telegram: BotPlatformStatus {
            enabled: telegram_config.enabled,
            configured: telegram_config.resolve_bot_token().is_some(),
            running: state.is_running().await,
        },
        discord: BotPlatformStatus {
            enabled: config.discord.enabled,
            configured: has_secret(&config.discord.bot_token, &config.discord.bot_token_env),
            running: discord_running,
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
        },
        webhook: BotPlatformStatus {
            enabled: config.webhook.enabled,
            configured: !config.webhook.endpoint_path.trim().is_empty(),
            running: http_running && config.webhook.enabled,
        },
    })
}

#[derive(Debug, Serialize)]
struct BotReply {
    reply: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
}

fn reply_text_with_translation(reply: &BotReply) -> String {
    if let Some(translation) = reply
        .translation
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        format!("{}\n\nTranslation: {}", reply.reply, translation)
    } else {
        reply.reply.clone()
    }
}

async fn generate_bot_reply(
    app: &tauri::AppHandle,
    platform: &str,
    text: &str,
    character_id: Option<&str>,
    conversation_key: Option<&str>,
) -> Result<BotReply, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Message text is empty".to_string());
    }

    let orchestrator = app
        .try_state::<AIOrchestrator>()
        .ok_or("AIOrchestrator not available")?;
    let llm_service = app
        .try_state::<LlmService>()
        .ok_or("LlmService not available")?;
    let action_registry = app
        .try_state::<Arc<RwLock<crate::actions::ActionRegistry>>>()
        .ok_or("ActionRegistry not available")?;
    let tool_settings = app
        .try_state::<Arc<RwLock<ToolSettings>>>()
        .ok_or("ToolSettings not available")?;

    let char_id = match character_id.filter(|id| !id.trim().is_empty()) {
        Some(id) => id.to_string(),
        None => {
            let mem_id = orchestrator.get_character_id().await;
            if !mem_id.is_empty() && mem_id != "default" {
                mem_id
            } else {
                crate::ai::context::AIOrchestrator::load_active_character_id()
                    .unwrap_or_else(|| "default".to_string())
            }
        }
    };
    let conversation_key = conversation_key
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(platform)
        .to_string();

    orchestrator
        .add_message("user".to_string(), trimmed.to_string(), &char_id)
        .await;

    let _ = app.emit(
        "telegram:chat-sync",
        json!({
            "role": "user",
            "text": format!("[{}] {}", platform, trimmed),
        }),
    );

    let tool_prompt = {
        let registry = action_registry.read().await;
        let settings = tool_settings.read().await;
        let prompt = registry.generate_tool_prompt_for_prompt_with_settings(
            orchestrator.is_memory_enabled(),
            &settings,
        );
        if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        }
    };

    let (prompt_messages, compose_warnings) = orchestrator
        .compose_prompt(trimmed, false, tool_prompt, false, &char_id)
        .await
        .map_err(|e| e.to_string())?;
    for warning in compose_warnings {
        tracing::warn!(target: "bot", "[{} compose_prompt] {}", platform, warning);
    }

    let mut client_messages = prompt_messages
        .into_iter()
        .map(|m| role_text_message(&m.role, m.content))
        .collect::<Result<Vec<_>, _>>()?;
    let already_has_user = client_messages.last().map(is_user_message).unwrap_or(false);
    if !already_has_user {
        client_messages.push(user_text_message(trimmed.to_string()));
    }

    let provider = llm_service.provider().await;
    let max_rounds = {
        let settings = tool_settings.read().await;
        settings.max_tool_rounds.max(1)
    };
    let mut all_cleaned_text = String::new();
    let mut all_translations = Vec::new();

    for round in 0..max_rounds {
        let mut stream = provider
            .chat_stream(client_messages.clone(), None)
            .await
            .map_err(|e| format!("LLM stream error: {}", e))?;

        let mut round_response = String::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => round_response.push_str(&delta),
                Err(error) => {
                    tracing::error!(target: "bot", "[{}] LLM stream error: {}", platform, error);
                    break;
                }
            }
        }

        if round_response.is_empty() {
            break;
        }

        let (cleaned, tool_calls) = parse_tool_call_tags(&round_response);
        let (cleaned, round_translation) = extract_translate_tags(&cleaned);
        let cleaned = strip_leaked_tags(&cleaned);

        tracing::info!(
            target: "bot::tools",
            "[{}] tool loop round {}: {} tool call(s), char_id='{}'",
            platform,
            round + 1,
            tool_calls.len(),
            char_id
        );

        if let Some(translation) = round_translation {
            all_translations.push(translation);
        }
        if !cleaned.is_empty() {
            merge_continuation_text(&mut all_cleaned_text, &cleaned);
        }

        if tool_calls.is_empty() {
            break;
        }

        let tool_invocations = {
            let registry = action_registry.read().await;
            tool_calls
                .iter()
                .map(|tool_call| {
                    crate::commands::actions::build_tool_invocation_from_input(
                        &registry,
                        &tool_call.name,
                        tool_call.args.clone(),
                        None,
                    )
                    .map_err(|error| format!("Tool resolution error: {}", error.0))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let execution_outcomes = execute_tool_calls(
            app,
            &action_registry,
            &tool_settings,
            &char_id,
            &tool_invocations,
        )
        .await;
        let tool_results = execution_outcomes
            .iter()
            .map(|outcome| {
                tracing::info!(
                    target: "bot::tools",
                    "[{}] executing {} with args {:?}",
                    platform,
                    outcome.invocation.name,
                    outcome.invocation.args
                );
                match &outcome.result {
                    Ok(result) => {
                        tracing::info!(
                            target: "bot::tools",
                            "[{}] {} => {}",
                            platform,
                            outcome.tool_name(),
                            result.message
                        );
                    }
                    Err(error) => {
                        tracing::error!(
                            target: "bot::tools",
                            "[{}] {} failed: {}",
                            platform,
                            outcome.tool_name(),
                            error
                        );
                    }
                }
                outcome.result_line()
            })
            .collect::<Vec<_>>();
        let any_needs_feedback = execution_outcomes
            .iter()
            .any(|outcome| outcome.needs_feedback);

        if !any_needs_feedback {
            break;
        }

        client_messages.push(assistant_text_message(round_response));
        client_messages.push(system_message(format!(
            "[Tool results]\n{}\nContinue your response naturally.",
            tool_results.join("\n")
        )));
    }

    let reply = compact_newlines(&strip_control_tags(&all_cleaned_text));
    if reply.is_empty() {
        return Err("No response from AI".to_string());
    }
    let translation = if all_translations.is_empty() {
        None
    } else {
        Some(compact_newlines(&all_translations.join(" ")))
    };

    let metadata = translation
        .as_ref()
        .map(|value| json!({ "translation": value }).to_string());
    orchestrator
        .add_message_with_metadata(
            "assistant".to_string(),
            reply.clone(),
            metadata,
            &char_id,
            None,
        )
        .await;

    trigger_bot_memory_tasks(
        platform,
        trimmed,
        &char_id,
        &conversation_key,
        &orchestrator,
        &llm_service,
    )
    .await;

    let _ = app.emit(
        "telegram:chat-sync",
        json!({
            "role": "assistant",
            "text": reply.clone(),
            "translation": translation.clone(),
        }),
    );

    Ok(BotReply { reply, translation })
}

async fn trigger_bot_memory_tasks(
    platform: &str,
    user_text: &str,
    char_id: &str,
    conversation_key: &str,
    orchestrator: &AIOrchestrator,
    llm_service: &LlmService,
) {
    let msg_count = orchestrator.get_message_count().await;
    let memory_msg_count = orchestrator.get_memory_trigger_count().await;
    let upgrade_config =
        crate::config::load_memory_upgrade_config(&crate::ai::memory::memory_upgrade_config_path());
    let ingress_options = MemoryEventIngressOptions {
        enabled: upgrade_config.event_trigger_enabled,
        event_cooldown_secs: upgrade_config.event_cooldown_secs,
        intent_routing_enabled: upgrade_config.intent_routing_enabled,
    };
    let memory_target_language = orchestrator.response_language.lock().await.clone();

    tracing::info!(
        target: "bot::memory",
        "[{}] user message count: {}, memory trigger count: {}, char_id: {}",
        platform,
        msg_count,
        memory_msg_count,
        char_id
    );

    if orchestrator.is_memory_enabled() {
        if let Some(decision) = select_memory_ingress_decision(user_text, &ingress_options) {
            let cooldown_key =
                build_cooldown_key(char_id, conversation_key, decision.event.event_type);
            if orchestrator
                .should_trigger_memory_event(&cooldown_key, decision.event.cooldown_secs)
                .await
            {
                let history = orchestrator.get_recent_memory_history(10).await;
                let memory_mgr = orchestrator.memory_manager.clone();
                let provider_for_mem = llm_service.provider().await;
                let char_id_for_mem = char_id.to_string();
                let source = platform.to_string();
                let memory_enabled = orchestrator.memory_enabled_flag();
                let observation_started_at = std::time::Instant::now();
                let trigger = decision.trigger_label.to_string();
                let extraction_options = memory_extractor::MemoryExtractionOptions {
                    structured_memory_enabled: should_use_structured_extraction(
                        upgrade_config.structured_memory_enabled,
                        &ingress_options,
                    ),
                    target_language: Some(memory_target_language.clone()),
                };
                tauri::async_runtime::spawn(async move {
                    if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    let _ = memory_mgr
                        .record_periodic_write_if_enabled(
                            &char_id_for_mem,
                            &source,
                            &trigger,
                            observation_started_at,
                        )
                        .await;
                    memory_extractor::extract_and_store_memories_with_options(
                        &history,
                        &memory_mgr,
                        provider_for_mem,
                        char_id_for_mem,
                        extraction_options,
                    )
                    .await;
                });
            }
        }
    }

    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 5 == 0 {
        let history = orchestrator.get_recent_memory_history(10).await;
        let memory_mgr = orchestrator.memory_manager.clone();
        let provider_for_mem = llm_service.provider().await;
        let char_id_for_mem = char_id.to_string();
        let source = platform.to_string();
        let memory_enabled = orchestrator.memory_enabled_flag();
        let observation_started_at = std::time::Instant::now();
        let extraction_options = memory_extractor::MemoryExtractionOptions {
            structured_memory_enabled: false,
            target_language: Some(memory_target_language.clone()),
        };
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let _ = memory_mgr
                .record_periodic_write_if_enabled(
                    &char_id_for_mem,
                    &source,
                    "periodic_extraction",
                    observation_started_at,
                )
                .await;
            memory_extractor::extract_and_store_memories_with_options(
                &history,
                &memory_mgr,
                provider_for_mem,
                char_id_for_mem,
                extraction_options,
            )
            .await;
        });
    }

    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 20 == 0 {
        let memory_mgr = orchestrator.memory_manager.clone();
        let char_id_for_consolidation = char_id.to_string();
        let provider_for_consolidation = llm_service.provider().await;
        let memory_enabled = orchestrator.memory_enabled_flag();
        let observation_started_at = std::time::Instant::now();
        let source = platform.to_string();
        let target_language_for_consolidation = memory_target_language.clone();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let _ = memory_mgr
                .periodic_consolidation_observation(
                    &char_id_for_consolidation,
                    &source,
                    observation_started_at,
                )
                .await;
            match memory_mgr
                .consolidate_memories_with_language(
                    &char_id_for_consolidation,
                    provider_for_consolidation,
                    Some(target_language_for_consolidation),
                )
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(
                        target: "bot::memory",
                        "[{}] consolidated {} memory clusters",
                        source,
                        count
                    );
                }
                Err(error) => {
                    tracing::error!(
                        target: "bot::memory",
                        "[{}] memory consolidation failed: {}",
                        source,
                        error
                    );
                }
                _ => {}
            }
        });
    }
}

const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";
const TRANSLATE_TAG_PREFIX: &str = "[TRANSLATE:";

#[derive(Debug, Clone)]
struct ToolCall {
    name: String,
    args: HashMap<String, String>,
}

impl From<ToolCall> for ToolInvocation {
    fn from(value: ToolCall) -> Self {
        Self {
            tool_call_id: None,
            name: value.name,
            args: value.args,
        }
    }
}

fn parse_tool_call_tags(text: &str) -> (String, Vec<ToolCall>) {
    let mut result = text.to_string();
    let mut calls_with_positions = Vec::new();

    while let Some(start) = result.rfind(TOOL_CALL_TAG_PREFIX) {
        let rest = &result[start..];
        if let Some(end_bracket) = rest.find(']') {
            let inner = &rest[TOOL_CALL_TAG_PREFIX.len()..end_bracket];
            let parts: Vec<&str> = inner.split('|').collect();
            if let Some(name) = parts.first() {
                let name = name.trim().to_string();
                let mut args = HashMap::new();
                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }
                calls_with_positions.push((start, ToolCall { name, args }));
            }
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                &result[..start],
                if tag_end < result.len() {
                    &result[tag_end..]
                } else {
                    ""
                }
            );
        } else {
            break;
        }
    }

    let mut cleaned = result.clone();
    let mut offset = 0;
    while offset < cleaned.len() {
        let Some(rel_start) = cleaned[offset..].find('[') else {
            break;
        };
        let start = offset + rel_start;
        let rest = &cleaned[start..];
        let Some(end) = rest.find(']') else {
            break;
        };
        let inner = &rest[1..end];
        let mut matched = false;
        if let Some(pipe_pos) = inner.find('|') {
            let name_part = &inner[..pipe_pos];
            let is_identifier =
                !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');
            let has_kv = inner[pipe_pos + 1..].contains('=');
            if is_identifier && has_kv {
                let parts: Vec<&str> = inner.split('|').collect();
                let name = parts[0].trim().to_string();
                let mut args = HashMap::new();
                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }
                calls_with_positions.push((start, ToolCall { name, args }));
                let tag_end = start + end + 1;
                cleaned = format!(
                    "{}{}",
                    &cleaned[..start],
                    if tag_end < cleaned.len() {
                        &cleaned[tag_end..]
                    } else {
                        ""
                    }
                );
                matched = true;
            }
        }
        if !matched {
            offset = start + 1;
        }
    }
    calls_with_positions.sort_by_key(|(start, _)| *start);
    let calls = calls_with_positions
        .into_iter()
        .map(|(_, call)| call)
        .collect();
    (cleaned.trim().to_string(), calls)
}

fn extract_translate_tags(text: &str) -> (String, Option<String>) {
    let mut translations = Vec::new();
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..start + end_bracket];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            result = result[..start].trim_end().to_string();
        }
    }
    let translation = if translations.is_empty() {
        None
    } else {
        Some(translations.join(" "))
    };
    (result.trim().to_string(), translation)
}

fn strip_leaked_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<tool_result>") {
        if let Some(end) = result[start..].find("</tool_result>") {
            let tag_end = start + end + "</tool_result>".len();
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            let line_end = result[start..]
                .find('\n')
                .map(|i| start + i)
                .unwrap_or(result.len());
            result = format!("{}{}", result[..start].trim_end(), &result[line_end..]);
        }
    }
    result.trim().to_string()
}

fn merge_continuation_text(accumulated: &mut String, next: &str) {
    let next = next.trim();
    if next.is_empty() {
        return;
    }
    if accumulated.is_empty() {
        accumulated.push_str(next);
        return;
    }
    if !accumulated.ends_with(char::is_whitespace) && !next.starts_with(char::is_whitespace) {
        accumulated.push(' ');
    }
    accumulated.push_str(next);
}

fn strip_control_tags(text: &str) -> String {
    let mut bracket_cleaned = text.to_string();
    for prefix in ["[ACTION:", "[EMOTION:", "[IMAGE_PROMPT:"] {
        while let Some(start) = bracket_cleaned.find(prefix) {
            if let Some(end) = bracket_cleaned[start..].find(']') {
                let tag_end = start + end + 1;
                bracket_cleaned = format!(
                    "{}{}",
                    &bracket_cleaned[..start],
                    &bracket_cleaned[tag_end..]
                );
            } else {
                break;
            }
        }
    }

    let mut result = String::with_capacity(bracket_cleaned.len());
    let mut chars = bracket_cleaned.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '<' {
            result.push(ch);
            continue;
        }

        let mut tag = String::new();
        while let Some(next) = chars.peek().copied() {
            tag.push(next);
            chars.next();
            if next == '>' || tag.len() > 64 {
                break;
            }
        }

        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("translate>")
            || lower.starts_with("/translate>")
            || lower.starts_with("emotion")
            || lower.starts_with("/emotion>")
            || lower.starts_with("cue")
            || lower.starts_with("/cue>")
        {
            continue;
        }

        result.push('<');
        result.push_str(&tag);
    }

    result.trim().to_string()
}

fn compact_newlines(text: &str) -> String {
    let mut out = String::new();
    let mut blank_count = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                out.push('\n');
            }
        } else {
            blank_count = 0;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(line.trim_end());
        }
    }
    out.trim().to_string()
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
    app: tauri::AppHandle,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let route = warp::post()
        .and(warp::path::full())
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::header::optional::<String>("x-line-signature"))
        .and(warp::body::bytes())
        .and(warp::any().map(move || config.clone()))
        .and(warp::any().map(move || app.clone()))
        .and_then(handle_http_bot_request);

    tracing::info!(target: "bot::http", "Bot HTTP server listening on {}", addr);
    warp::serve(route)
        .bind_with_graceful_shutdown(addr, async move {
            let _ = shutdown_rx.await;
        })
        .1
        .await;
    tracing::info!(target: "bot::http", "Bot HTTP server stopped");
}

async fn handle_http_bot_request(
    full_path: warp::path::FullPath,
    authorization: Option<String>,
    line_signature: Option<String>,
    body: bytes::Bytes,
    config: Arc<RwLock<BotConfig>>,
    app: tauri::AppHandle,
) -> Result<warp::reply::Response, Infallible> {
    let cfg = config.read().await.clone();
    let path = full_path.as_str();
    let line_path = normalize_path(&cfg.line.webhook_path);
    let webhook_path = normalize_path(&cfg.webhook.endpoint_path);

    let response = if cfg.line.enabled && path == line_path {
        handle_line_webhook(cfg.line, line_signature, body, app).await
    } else if cfg.webhook.enabled && path == webhook_path {
        handle_generic_webhook(cfg.webhook, authorization, body, app).await
    } else {
        json_response(json!({ "error": "Not found" }), StatusCode::NOT_FOUND)
    };

    Ok(response)
}

#[derive(Debug, Deserialize)]
struct GenericWebhookMessage {
    text: Option<String>,
    message: Option<String>,
    character_id: Option<String>,
    conversation_id: Option<String>,
    user_id: Option<String>,
    source: Option<String>,
}

async fn handle_generic_webhook(
    config: WebhookBotConfig,
    authorization: Option<String>,
    body: bytes::Bytes,
    app: tauri::AppHandle,
) -> warp::reply::Response {
    if let Some(expected) = resolve_secret(&config.bearer_token, &config.bearer_token_env) {
        let Some(actual) = authorization else {
            return unauthorized("Missing Authorization header");
        };
        if actual.trim() != format!("Bearer {}", expected) {
            return unauthorized("Invalid bearer token");
        }
    }

    let request: GenericWebhookMessage = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => return bad_request(&format!("Invalid JSON: {}", error)),
    };
    let text = request
        .text
        .or(request.message)
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return bad_request("Missing text");
    }

    let conversation_key = request
        .conversation_id
        .as_deref()
        .or(request.user_id.as_deref())
        .or(request.source.as_deref());
    match generate_bot_reply(
        &app,
        "webhook",
        &text,
        request.character_id.as_deref(),
        conversation_key,
    )
    .await
    {
        Ok(reply) => json_response(json!(reply), StatusCode::OK),
        Err(error) => server_error(&error),
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
    #[serde(rename = "type")]
    message_type: String,
    text: Option<String>,
}

async fn handle_line_webhook(
    config: LineBotConfig,
    signature: Option<String>,
    body: bytes::Bytes,
    app: tauri::AppHandle,
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
        if message.message_type != "text" {
            continue;
        }
        let Some(text) = message.text else {
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

        match generate_bot_reply(
            &app,
            "line",
            &text,
            config.character_id.as_deref(),
            user_id.as_deref(),
        )
        .await
        {
            Ok(reply) => {
                let reply_text = reply_text_with_translation(&reply);
                if let Err(error) = send_line_reply(&access_token, &reply_token, &reply_text).await
                {
                    tracing::error!(target: "bot::line", "failed to send LINE reply: {}", error);
                }
            }
            Err(error) => {
                tracing::error!(target: "bot::line", "failed to generate LINE reply: {}", error);
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
    app: tauri::AppHandle,
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
            app.clone(),
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
    app: tauri::AppHandle,
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
                    handle_discord_message(client, token, &config, &app, payload.get("d").cloned().unwrap_or(Value::Null)).await;
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
    app: &tauri::AppHandle,
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
        .trim()
        .to_string();
    if content.is_empty() {
        return;
    }

    let cfg = config.read().await.discord.clone();
    let is_direct_message = message.get("guild_id").and_then(Value::as_str).is_none();
    if is_direct_message && !cfg.allow_direct_messages {
        return;
    }
    if !is_direct_message
        && !cfg.allowed_channel_ids.is_empty()
        && !cfg.allowed_channel_ids.iter().any(|id| id == &channel_id)
    {
        return;
    }

    let author_id = message
        .get("author")
        .and_then(|author| author.get("id"))
        .and_then(Value::as_str);
    let conversation_key = author_id
        .map(|author| format!("{}:{}", channel_id, author))
        .unwrap_or_else(|| channel_id.clone());

    match generate_bot_reply(
        app,
        "discord",
        &content,
        cfg.character_id.as_deref(),
        Some(&conversation_key),
    )
    .await
    {
        Ok(reply) => {
            let reply_text = reply_text_with_translation(&reply);
            if let Err(error) = send_discord_message(client, token, &channel_id, &reply_text).await
            {
                tracing::error!(target: "bot::discord", "failed to send Discord message: {}", error);
            }
        }
        Err(error) => {
            tracing::error!(target: "bot::discord", "failed to generate Discord reply: {}", error);
        }
    }
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
        assert_eq!(
            config.line.channel_access_token_env.as_deref(),
            Some("LINE_CHANNEL_ACCESS_TOKEN")
        );
        assert_eq!(config.line.webhook_path, "/line/webhook");
        assert_eq!(config.webhook.bind_host, "127.0.0.1");
        assert_eq!(config.webhook.port, 8787);
    }

    #[test]
    fn bot_config_deserializes_partial_files_with_defaults() {
        let config: BotConfig = serde_json::from_str(r#"{"selected_platform":"discord"}"#).unwrap();
        assert_eq!(config.selected_platform, "discord");
        assert!(config.discord.allow_direct_messages);
        assert_eq!(config.webhook.endpoint_path, "/webhook/message");
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
    fn bot_parse_tool_call_tags_supports_explicit_and_short_forms() {
        let (cleaned, calls) = parse_tool_call_tags(
            "Hi [TOOL_CALL:store_memory|fact=likes tea] [play_cue|name=smile]",
        );

        assert_eq!(cleaned, "Hi");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "store_memory");
        assert_eq!(
            calls[0].args.get("fact").map(String::as_str),
            Some("likes tea")
        );
        assert_eq!(calls[1].name, "play_cue");
        assert_eq!(calls[1].args.get("name").map(String::as_str), Some("smile"));
    }

    #[test]
    fn bot_extract_translate_tags_strips_and_returns_translation() {
        let (cleaned, translation) = extract_translate_tags("Hello [TRANSLATE:こんにちは]");

        assert_eq!(cleaned, "Hello");
        assert_eq!(translation.as_deref(), Some("こんにちは"));
    }

    #[test]
    fn bot_strip_control_tags_removes_platform_unsafe_tags() {
        let cleaned = strip_control_tags(
            "Hello [ACTION:wave] <emotion>happy</emotion> [IMAGE_PROMPT:cat] world",
        );

        assert_eq!(cleaned, "Hello  happy  world");
    }
}
