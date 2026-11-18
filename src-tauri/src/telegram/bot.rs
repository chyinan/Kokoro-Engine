//! Telegram Bot core — message handling, command dispatch, and AI pipeline bridge.
// pattern: Mixed (unavoidable)
// Reason: Telegram 机器人处理网络 I/O、消息路由、持久化和异步任务调度，属于编排层。

use super::config::TelegramConfig;
use crate::ai::context::AIOrchestrator;
use crate::bot::{lifecycle::BotIngress, BotChatServices, BotChatUnavailable};
use crate::chat::application::{channel_id, ChatPolicy, ChatRequest, ChatService};
use crate::chat::persistence::{self, ExternalTurn};
use crate::error::KokoroError;
use crate::stt::AudioSource;
use base64::Engine as _;
use futures::FutureExt;
use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::InputFile;
use teloxide::update_listeners::UpdateListener;
use tokio::sync::RwLock;

/// 每用户速率限制：滑动窗口内最多允许的消息数
const RATE_LIMIT_MAX: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// 每个 chat 的速率限制状态
#[derive(Clone, Debug)]
struct RateState {
    count: usize,
    window_start: Instant,
}

type RateLimiter = Arc<RwLock<HashMap<ChatId, RateState>>>;

#[derive(Clone, Debug, Default)]
pub(crate) enum SessionMode {
    #[default]
    Private,
    Topic {
        thread: i32,
    },
    Fresh {
        id: String,
    },
    Desktop {
        character_id: String,
        conversation_id: String,
    },
}

impl SessionMode {
    fn bind<'a>(&'a self, chat_id: ChatId, character_id: &'a str) -> (&'a str, String, bool) {
        match self {
            Self::Private => (
                character_id,
                channel_id("telegram", character_id, &chat_id.0.to_string()),
                true,
            ),
            Self::Topic { thread } => (
                character_id,
                channel_id(
                    "telegram",
                    character_id,
                    &format!("{}:thread:{thread}", chat_id.0),
                ),
                true,
            ),
            Self::Fresh { id } => (character_id, channel_id("telegram", character_id, id), true),
            Self::Desktop {
                character_id,
                conversation_id,
            } => (character_id, conversation_id.clone(), false),
        }
    }
}

#[derive(Clone, Default)]
struct Sessions {
    modes: Arc<RwLock<SessionBindings>>,
    deliveries: Arc<std::sync::Mutex<VecDeque<ReplyDeliveryEntry>>>,
}

#[derive(Default)]
struct SessionBindings {
    private: HashMap<(ChatId, Option<i32>, String), SessionMode>,
    continued: HashMap<(ChatId, Option<i32>), SessionMode>,
}

#[derive(Default)]
struct ReplyDeliveryProgress {
    voice_attempted: bool,
    images_attempted: bool,
}

struct ReplyDeliveryEntry {
    conversation_id: String,
    request_id: String,
    progress: Arc<tokio::sync::Mutex<ReplyDeliveryProgress>>,
}

impl Sessions {
    #[cfg(test)]
    async fn mode(&self, chat: ChatId, character: &str) -> SessionMode {
        self.mode_for(chat, None, character).await
    }
    async fn mode_for(&self, chat: ChatId, thread: Option<i32>, character: &str) -> SessionMode {
        let modes = self.modes.read().await;
        modes
            .continued
            .get(&(chat, thread))
            .or_else(|| modes.private.get(&(chat, thread, character.to_owned())))
            .cloned()
            .unwrap_or_else(|| {
                thread.map_or(SessionMode::Private, |thread| SessionMode::Topic { thread })
            })
    }

    #[cfg(test)]
    async fn start_new(&self, chat: ChatId, character: &str) {
        self.start_new_for(chat, None, character).await;
    }

    async fn start_new_for(&self, chat: ChatId, thread: Option<i32>, character: &str) {
        let mut modes = self.modes.write().await;
        modes.continued.remove(&(chat, thread));
        modes.private.insert(
            (chat, thread, character.to_owned()),
            SessionMode::Fresh {
                id: uuid::Uuid::new_v4().to_string(),
            },
        );
    }

    #[cfg(test)]
    async fn continue_desktop(&self, chat: ChatId, character_id: String, conversation_id: String) {
        self.continue_desktop_for(chat, None, character_id, conversation_id)
            .await;
    }
    async fn continue_desktop_for(
        &self,
        chat: ChatId,
        thread: Option<i32>,
        character_id: String,
        conversation_id: String,
    ) {
        self.modes.write().await.continued.insert(
            (chat, thread),
            SessionMode::Desktop {
                character_id,
                conversation_id,
            },
        );
    }

    fn reply_delivery(
        &self,
        identity: (&str, &str),
        newly_committed: bool,
    ) -> Option<Arc<tokio::sync::Mutex<ReplyDeliveryProgress>>> {
        let mut deliveries = self.deliveries.lock().unwrap();
        if let Some(entry) = deliveries
            .iter()
            .find(|entry| entry.conversation_id == identity.0 && entry.request_id == identity.1)
        {
            return Some(entry.progress.clone());
        }
        // A reply loaded from SQLite may already have delivered its media, even
        // after a restart or cache eviction. Only a new commit admits media work.
        if !newly_committed {
            return None;
        }
        let progress = Arc::new(tokio::sync::Mutex::new(ReplyDeliveryProgress::default()));
        deliveries.push_back(ReplyDeliveryEntry {
            conversation_id: identity.0.to_owned(),
            request_id: identity.1.to_owned(),
            progress: progress.clone(),
        });
        while deliveries.len() > 32 {
            deliveries.pop_front();
        }
        Some(progress)
    }
}

type TelegramDeliveryError = Box<dyn std::error::Error + Send + Sync>;

async fn send_text(
    bot: &Bot,
    chat: ChatId,
    thread: Option<teloxide::types::ThreadId>,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    let request = bot.send_message(chat, text);
    match thread {
        Some(thread) => request.message_thread_id(thread).await,
        None => request.await,
    }
}

trait TelegramReplyDelivery: Sync {
    fn text(
        &self,
        text: String,
    ) -> impl std::future::Future<Output = Result<(), TelegramDeliveryError>> + Send;
    fn voice(&self, text: &str) -> impl std::future::Future<Output = ()> + Send;
    fn images(&self, text: &str) -> impl std::future::Future<Output = ()> + Send;
}

struct TelegramDelivery<'a> {
    thread: Option<teloxide::types::ThreadId>,
    speech: Option<&'a crate::tts::manager::TtsExecutionSnapshot>,
    bot: &'a Bot,
    chat_id: ChatId,
    tts_available: bool,
    media: &'a persistence::ExternalReplyMedia,
}

impl TelegramReplyDelivery for TelegramDelivery<'_> {
    async fn text(&self, text: String) -> Result<(), TelegramDeliveryError> {
        for chunk in split_message(&text) {
            send_text(self.bot, self.chat_id, self.thread, chunk)
                .await
                .map_err(|error| match error {
                    teloxide::RequestError::Network(error) => format!(
                        "Telegram reply delivery is uncertain: {}",
                        error.without_url()
                    )
                    .into(),
                    error => TelegramDeliveryError::from(error),
                })?;
        }
        Ok(())
    }
    async fn voice(&self, text: &str) {
        let _ = send_voice_reply(
            self.bot,
            self.chat_id,
            self.thread,
            text,
            self.tts_available,
            self.speech,
        )
        .await;
    }
    async fn images(&self, _text: &str) {
        for image in &self.media.images {
            if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(&image.data_base64) {
                let request = self.bot.send_photo(
                    self.chat_id,
                    InputFile::memory(data).file_name(image.file_name.clone()),
                );
                let result = match self.thread {
                    Some(thread) => request.message_thread_id(thread).await,
                    None => request.await,
                };
                if let Err(error) = result {
                    tracing::warn!(target: "telegram", "Image delivery failed: {error}");
                }
            }
        }
    }
}

struct TelegramReply<'a> {
    text: &'a str,
    translation: Option<&'a str>,
    newly_committed: bool,
    voice: bool,
    images: bool,
}

/// Normal replies and recovered saves share delivery. Text can be resent, while
/// optional media is attempted once; a failed text send leaves media available
/// for another delivery of this request while its bounded progress is retained.
async fn deliver_telegram_reply(
    sessions: &Sessions,
    identity: (&str, &str),
    reply: TelegramReply<'_>,
    delivery: &impl TelegramReplyDelivery,
) -> Result<(), TelegramDeliveryError> {
    let progress = sessions.reply_delivery(identity, reply.newly_committed);
    let mut progress = match &progress {
        Some(progress) => Some(progress.lock().await),
        None => None,
    };
    let text = reply.text.to_owned();
    delivery
        .text(match reply.translation {
            Some(translation) => format!("{text}\n\n🌐 {translation}"),
            None => text.clone(),
        })
        .await?;
    if let Some(progress) = progress.as_mut() {
        // Claim before invoking best-effort media so replay never repeats a
        // generation or delivery whose external outcome may be ambiguous.
        if !progress.voice_attempted {
            progress.voice_attempted = true;
            if reply.voice {
                delivery.voice(&text).await;
            }
        }
        if !progress.images_attempted {
            progress.images_attempted = true;
            if reply.images {
                delivery.images(&text).await;
            }
        }
    }
    Ok(())
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
const MAX_MESSAGE_CHARS: usize = 4096;
const MAX_MEDIA_BYTES: u32 = 20 * 1024 * 1024;

/// Run the long-polling loop and let the SDK drain its message workers.
pub(crate) async fn run_polling(
    token: String,
    config: Arc<RwLock<TelegramConfig>>,
    services: BotChatServices,
    ingress: BotIngress,
) -> Result<(), String> {
    let bot = Bot::new(&token);
    let stop = ingress.token().clone();
    let mut listener = tokio::select! {
        biased;
        _ = stop.cancelled() => return Ok(()),
        listener = teloxide::update_listeners::polling_default(bot.clone()) => listener,
    };
    let listener_stop = listener.stop_token();
    let sessions = Sessions::default();
    let rate_limiter: RateLimiter = Arc::new(RwLock::new(HashMap::new()));
    let handler = Update::filter_message().endpoint(handle_message);
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            config,
            sessions,
            services,
            rate_limiter,
            ingress
        ])
        .default_handler(|_upd| async {})
        .build();

    let shutdown_token = dispatcher.shutdown_token();
    let dispatch = dispatcher.try_dispatch_with_listener(
        listener,
        teloxide::error_handlers::LoggingErrorHandler::with_custom_text("Telegram polling failed"),
    );
    tokio::pin!(dispatch);
    let result = tokio::select! {
        biased;
        result = &mut dispatch => result,
        _ = stop.cancelled() => {
            // During get_me, the dispatcher has no update workers yet.
            if shutdown_token.shutdown().is_err() {
                return Ok(());
            }
            listener_stop.stop();
            dispatch.await
        }
    };
    // Telegram request errors can include the token in a URL.
    result.map_err(|_| "Telegram dispatcher could not initialize".to_owned())
}

/// Central message handler.
async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<RwLock<TelegramConfig>>,
    sessions: Sessions,
    services: BotChatServices,
    rate_limiter: RateLimiter,
    ingress: BotIngress,
) -> HandlerResult {
    let Some(_work) = ingress.admit("Telegram message") else {
        return Ok(());
    };
    let reply = AssertUnwindSafe(async {
        let result = dispatch_message(
            &bot,
            &msg,
            &config,
            &rate_limiter,
            &TelegramMessageHandler {
                bot: &bot,
                sessions: &sessions,
                services: &services,
            },
        )
        .await;
        match result {
            Err(error) if error.is::<BotChatUnavailable>() => {
                send_text(&bot, msg.chat.id, msg.thread_id, error.to_string()).await?;
                Ok(())
            }
            result => result,
        }
    })
    .catch_unwind()
    .await;
    match reply {
        Ok(result) => result,
        Err(_) => {
            Err("Telegram message handler panicked. External reply delivery is uncertain.".into())
        }
    }
}

/// Keep authorization and routing together, before any business or media work.
/// The handler boundary lets local tests observe whether business work started.
trait MessageHandler: Sync {
    fn command(
        &self,
        msg: &Message,
        text: &str,
        config: &TelegramConfig,
    ) -> impl std::future::Future<Output = HandlerResult> + Send;
    fn voice(
        &self,
        msg: &Message,
        config: &TelegramConfig,
    ) -> impl std::future::Future<Output = HandlerResult> + Send;
    fn photo(
        &self,
        msg: &Message,
        config: &TelegramConfig,
    ) -> impl std::future::Future<Output = HandlerResult> + Send;
    fn text(
        &self,
        msg: &Message,
        text: &str,
        config: &TelegramConfig,
    ) -> impl std::future::Future<Output = HandlerResult> + Send;
}

struct TelegramMessageHandler<'a> {
    bot: &'a Bot,
    sessions: &'a Sessions,
    services: &'a BotChatServices,
}

impl MessageHandler for TelegramMessageHandler<'_> {
    async fn command(&self, msg: &Message, text: &str, config: &TelegramConfig) -> HandlerResult {
        handle_command(self.bot, msg, text, config, self.sessions, self.services).await
    }

    async fn voice(&self, msg: &Message, config: &TelegramConfig) -> HandlerResult {
        handle_voice(self.bot, msg, config, self.sessions, self.services).await
    }

    async fn photo(&self, msg: &Message, config: &TelegramConfig) -> HandlerResult {
        handle_photo(self.bot, msg, config, self.sessions, self.services).await
    }

    async fn text(&self, msg: &Message, text: &str, config: &TelegramConfig) -> HandlerResult {
        reply_to_message(
            self.bot,
            msg,
            text,
            Vec::new(),
            config,
            self.sessions,
            self.services,
        )
        .await
    }
}

async fn dispatch_message(
    bot: &Bot,
    msg: &Message,
    config: &RwLock<TelegramConfig>,
    rate_limiter: &RateLimiter,
    handler: &impl MessageHandler,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    // 不记录消息内容，避免敏感信息泄露到日志
    tracing::info!(target: "telegram", "[Telegram] Received message from chat_id={}", chat_id.0);

    // Snapshot config for this request
    let config = config.read().await.clone();

    // Access control: check whitelist
    if !config.allowed_chat_ids.contains(&chat_id.0) {
        tracing::info!(target: "telegram", "[Telegram] Chat {} not in whitelist, ignoring", chat_id.0);
        return Ok(());
    }

    // 速率限制：滑动窗口，每分钟最多 RATE_LIMIT_MAX 条消息
    {
        let mut limiter = rate_limiter.write().await;
        let state = limiter.entry(chat_id).or_insert(RateState {
            count: 0,
            window_start: Instant::now(),
        });
        if state.window_start.elapsed() >= RATE_LIMIT_WINDOW {
            state.count = 0;
            state.window_start = Instant::now();
        }
        state.count += 1;
        if state.count > RATE_LIMIT_MAX {
            tracing::info!(target: "telegram", "[Telegram] Rate limit exceeded for chat_id={}", chat_id.0);
            send_text(
                bot,
                chat_id,
                msg.thread_id,
                "⚠️ Too many messages. Please wait a moment.",
            )
            .await
            .ok();
            return Ok(());
        }
    }

    // Check for commands
    if let Some(text) = msg.text() {
        if text.starts_with('/') {
            return handler.command(msg, text, &config).await;
        }
    }

    // Voice message
    if msg.voice().is_some() {
        return handler.voice(msg, &config).await;
    }

    // Photo message (with optional caption)
    if msg.photo().is_some() {
        return handler.photo(msg, &config).await;
    }

    // Regular text message
    if let Some(text) = msg.text() {
        return handler.text(msg, text, &config).await;
    }

    Ok(())
}

/// Handle bot commands: /start, /new, /continue, /status
async fn handle_command(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &TelegramConfig,
    sessions: &Sessions,
    services: &BotChatServices,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;
    let cmd = text.split_whitespace().next().unwrap_or("");

    match cmd {
        "/start" => {
            let text = "Kokoro Engine — Telegram Bridge\n\n\
                Commands:\n\
                /continue — Resume the desktop conversation\n\
                /new — Start a fresh conversation\n\
                /status — Show current session info\n\n\
                Just send a text or voice message to chat!";
            if let Err(e) = send_text(bot, chat_id, msg.thread_id, text).await {
                tracing::error!(target: "telegram", "[Telegram] Failed to send /start reply: {}", e);
            }
        }
        "/new" => {
            let state = services.chat()?.orchestrator.as_ref();
            let _guard = state.enter_chat_turn()?;
            let active = state.get_character_id().await;
            let character = config
                .character_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(&active);
            sessions
                .start_new_for(chat_id, msg.thread_id.map(|id| id.0 .0), character)
                .await;
            send_text(
                bot,
                chat_id,
                msg.thread_id,
                "✨ New Telegram conversation started.",
            )
            .await?;
        }
        "/continue" => {
            let state = services.chat()?.orchestrator.as_ref();
            let (character_id, conversation_id) = {
                let _activation = state.enter_chat_turn()?;
                let _selection = state.selection.conversation_switch_lock.lock().await;
                (
                    state.get_character_id().await,
                    state.selection.current_conversation_id.lock().await.clone(),
                )
            };
            let Some(conversation_id) = conversation_id else {
                send_text(
                    bot,
                    chat_id,
                    msg.thread_id,
                    "Open an existing desktop conversation before using /continue.",
                )
                .await?;
                return Ok(());
            };
            if config
                .character_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty() && id != character_id)
            {
                send_text(
                    bot,
                    chat_id,
                    msg.thread_id,
                    "Telegram is configured to use a different character.",
                )
                .await?;
                return Ok(());
            }
            sessions
                .continue_desktop_for(
                    chat_id,
                    msg.thread_id.map(|id| id.0 .0),
                    character_id,
                    conversation_id,
                )
                .await;
            send_text(
                bot,
                chat_id,
                msg.thread_id,
                "🔗 Continuing the selected desktop conversation.",
            )
            .await?;
        }
        "/status" => {
            let state = services.chat()?.orchestrator.as_ref();
            let active_character = state.get_character_id().await;
            let character = config
                .character_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(&active_character);
            let mode = sessions
                .mode_for(chat_id, msg.thread_id.map(|id| id.0 .0), character)
                .await;
            let (character, conversation, _) = mode.bind(chat_id, character);
            let history_len = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM conversation_messages AS m JOIN conversations AS c ON m.conversation_id = c.id
                 WHERE c.id = ? AND c.character_id = ?"
            ).bind(conversation).bind(character).fetch_one(&state.db).await?;
            let mode_str = match mode {
                SessionMode::Desktop { .. } => "Desktop conversation",
                _ => "Telegram conversation",
            };
            send_text(
                bot,
                chat_id,
                msg.thread_id,
                format!("📊 Session: {mode_str}\n💬 History: {history_len} messages"),
            )
            .await?;
        }
        _ => {
            if !text.trim_start_matches('/').trim().is_empty() {
                reply_to_message(bot, msg, text, Vec::new(), config, sessions, services).await?;
            }
        }
    }
    Ok(())
}

fn telegram_request_id(chat_id: i64, message_id: i32) -> String {
    persistence::external_identity("telegram", &[&chat_id.to_string(), &message_id.to_string()])
}

fn telegram_previous_request_ids(chat_id: i64, message_id: i32) -> Vec<String> {
    vec![persistence::legacy_external_identity(
        "telegram",
        &chat_id.to_string(),
        &message_id.to_string(),
    )]
}

struct TelegramPreparedInput {
    text: String,
    image: Option<String>,
}

/// The committed user row is authoritative on redelivery. Only a new platform
/// message needs downloading/transcription, and preparation must succeed before
/// its user row is written so a transient download failure remains retryable.
async fn prepare_telegram_media_input<F, Fut>(
    orchestrator: &AIOrchestrator,
    character_id: &str,
    chat_id: i64,
    message_id: i32,
    prepare: F,
) -> Result<Option<TelegramPreparedInput>, TelegramDeliveryError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<TelegramPreparedInput>, TelegramDeliveryError>>,
{
    persistence::validate_external_request_owner(
        &orchestrator.db,
        character_id,
        "telegram",
        &telegram_request_id(chat_id, message_id),
        &telegram_previous_request_ids(chat_id, message_id),
    )
    .await?;
    if let Some((_, text)) = ExternalTurn::saved_user_message(
        orchestrator,
        character_id,
        &telegram_request_id(chat_id, message_id),
    )
    .await?
    {
        return Ok(Some(TelegramPreparedInput { text, image: None }));
    }
    prepare().await
}

async fn reply_to_message(
    bot: &Bot,
    msg: &Message,
    text: &str,
    images: Vec<String>,
    config: &TelegramConfig,
    sessions: &Sessions,
    services: &BotChatServices,
) -> HandlerResult {
    let state = services.chat()?.orchestrator.as_ref();
    let _guard = state.enter_chat_turn()?;
    let active_character = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active_character);
    let mode = sessions
        .mode_for(msg.chat.id, msg.thread_id.map(|id| id.0 .0), preferred)
        .await;
    drop(_guard);
    reply_bound(
        bot, msg, text, images, config, sessions, services, &mode, preferred, None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn reply_bound(
    bot: &Bot,
    msg: &Message,
    text: &str,
    images: Vec<String>,
    config: &TelegramConfig,
    sessions: &Sessions,
    services: &BotChatServices,
    mode: &SessionMode,
    preferred: &str,
    input_text: Option<String>,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let (character_id, _, create_conversation) = mode.bind(chat_id, preferred);
    if !create_conversation
        && config
            .character_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty() && id != character_id)
    {
        send_text(
            bot,
            chat_id,
            msg.thread_id,
            "The configured character changed. Use /new or /continue to select a conversation.",
        )
        .await?;
        return Ok(());
    }
    let chat = &services.chat()?.service;
    let result = if input_text.is_some() {
        submit_message_input(
            chat, chat_id, msg.id.0, text, images, mode, preferred, input_text,
        )
        .await
    } else {
        submit_message(chat, chat_id, msg.id.0, text, images, mode, preferred).await
    };
    let result = match result {
        Ok(result) if result.status.as_deref() == Some("cancelled") => {
            send_text(bot, chat_id, msg.thread_id, "Chat cancelled.").await?;
            return Ok(());
        }
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(target: "telegram", "Chat generation failed: {error}");
            let feedback = if error.to_string().contains("chat_turn_busy") {
                "Another chat is in progress. Send a new message after it finishes."
            } else if error.to_string().contains("chat_shutting_down") {
                "Kokoro Engine is shutting down."
            } else {
                "Kokoro Engine could not complete this chat. Check the client for details."
            };
            send_text(bot, chat_id, msg.thread_id, feedback).await?;
            return Ok(());
        }
    };
    let mut text = result.text.clone();
    for warning in &result.output_warnings {
        text.push_str(&format!("\n\n⚠️ {warning}"));
    }
    deliver_telegram_reply(
        sessions,
        (
            &result.conversation_id,
            &telegram_request_id(chat_id.0, msg.id.0),
        ),
        TelegramReply {
            text: &text,
            translation: result.translation.as_deref(),
            newly_committed: result.newly_committed,
            voice: config.send_voice_reply,
            images: !result.external_media.images.is_empty(),
        },
        &TelegramDelivery {
            thread: msg.thread_id,
            speech: result.speech_snapshot.as_deref(),
            bot,
            chat_id,
            tts_available: services.tts.is_some(),
            media: &result.external_media,
        },
    )
    .await
}

pub(crate) async fn submit_message(
    chat: &ChatService,
    chat_id: ChatId,
    message_id: i32,
    text: &str,
    images: Vec<String>,
    mode: &SessionMode,
    preferred_character: &str,
) -> Result<crate::chat::application::StreamChatResponse, crate::error::KokoroError> {
    submit_message_input(
        chat,
        chat_id,
        message_id,
        text,
        images,
        mode,
        preferred_character,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn submit_message_input(
    chat: &ChatService,
    chat_id: ChatId,
    message_id: i32,
    text: &str,
    images: Vec<String>,
    mode: &SessionMode,
    preferred_character: &str,
    external_input_text: Option<String>,
) -> Result<crate::chat::application::StreamChatResponse, KokoroError> {
    let (character_id, conversation_id, create_conversation) =
        mode.bind(chat_id, preferred_character);
    chat.submit(
        ChatRequest {
            message: text.to_owned(),
            character_id: Some(character_id.to_owned()),
            conversation_id: Some(conversation_id),
            // Telegram message identifiers are unique within each chat.
            client_request_id: Some(telegram_request_id(chat_id.0, message_id)),
            previous_request_ids: telegram_previous_request_ids(chat_id.0, message_id),
            images: Some(images),
            external_input_text,
            ..Default::default()
        },
        ChatPolicy::External {
            create_conversation,
            image_output: true,
            source: "telegram",
        },
    )
    .await
}

fn split_message(text: &str) -> impl Iterator<Item = &str> {
    let mut remaining = text.trim();
    std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        let boundary = remaining
            .char_indices()
            .nth(MAX_MESSAGE_CHARS)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let (chunk, rest) = remaining.split_at(boundary);
        remaining = rest;
        Some(chunk)
    })
}

async fn handle_voice(
    bot: &Bot,
    msg: &Message,
    config: &TelegramConfig,
    sessions: &Sessions,
    services: &BotChatServices,
) -> HandlerResult {
    let state = services.chat()?.orchestrator.as_ref();
    let _guard = state.enter_chat_turn()?;
    let active = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active);
    let mode = sessions
        .mode_for(msg.chat.id, msg.thread_id.map(|id| id.0 .0), preferred)
        .await;
    drop(_guard);
    let (character, _, _) = mode.bind(msg.chat.id, preferred);
    let input = prepare_telegram_media_input(state, character, msg.chat.id.0, msg.id.0, || async {
        let voice = msg.voice().ok_or("No voice data")?;
        if voice.file.size > MAX_MEDIA_BYTES {
            return Err("Voice message exceeds the 20 MB limit".into());
        }
        let file = bot.get_file(&voice.file.id).await?;
        let mut data = Vec::new();
        teloxide::net::Download::download_file(bot, &file.path, &mut data).await?;
        let stt = services.stt.as_ref().ok_or("STT service not available")?;
        let text = stt
            .transcribe(
                &AudioSource::Encoded {
                    data,
                    format: "ogg".into(),
                },
                None,
            )
            .await?
            .text;
        if text.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(TelegramPreparedInput { text, image: None }))
    })
    .await?;
    if let Some(input) = input {
        reply_bound(
            bot,
            msg,
            &input.text,
            vec![],
            config,
            sessions,
            services,
            &mode,
            preferred,
            Some(msg.caption().unwrap_or("").to_owned()),
        )
        .await?;
    }
    Ok(())
}

async fn handle_photo(
    bot: &Bot,
    msg: &Message,
    config: &TelegramConfig,
    sessions: &Sessions,
    services: &BotChatServices,
) -> HandlerResult {
    let state = services.chat()?.orchestrator.as_ref();
    let _guard = state.enter_chat_turn()?;
    let active = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active);
    let mode = sessions
        .mode_for(msg.chat.id, msg.thread_id.map(|id| id.0 .0), preferred)
        .await;
    drop(_guard);
    let (character, _, _) = mode.bind(msg.chat.id, preferred);
    let input = prepare_telegram_media_input(state, character, msg.chat.id.0, msg.id.0, || async {
        let photo = msg
            .photo()
            .and_then(|photos| photos.last())
            .ok_or("No photo data")?;
        if photo.file.size > MAX_MEDIA_BYTES {
            return Err("Photo exceeds the 20 MB limit".into());
        }
        let file = bot.get_file(&photo.file.id).await?;
        let mut data = Vec::new();
        teloxide::net::Download::download_file(bot, &file.path, &mut data).await?;
        let mime = if file.path.ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        Ok(Some(TelegramPreparedInput {
            text: msg
                .caption()
                .filter(|caption| !caption.trim().is_empty())
                .unwrap_or("The user sent you a photo:")
                .to_owned(),
            image: Some(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            )),
        }))
    })
    .await?;
    if let Some(input) = input {
        reply_bound(
            bot,
            msg,
            &input.text,
            input.image.into_iter().collect(),
            config,
            sessions,
            services,
            &mode,
            preferred,
            Some(msg.caption().unwrap_or("").to_owned()),
        )
        .await?;
    }
    Ok(())
}

/// Synthesize text via TTS and send as a Telegram voice message.
async fn send_voice_reply(
    bot: &Bot,
    chat_id: ChatId,
    thread: Option<teloxide::types::ThreadId>,
    text: &str,
    tts_available: bool,
    speech: Option<&crate::tts::manager::TtsExecutionSnapshot>,
) -> HandlerResult {
    if !tts_available {
        send_text(bot, chat_id, thread, "⚠️ The TTS service is unavailable.").await?;
        return Ok(());
    }

    let Some(speech) = speech else {
        return Ok(());
    };
    match speech.synthesize(text).await {
        Ok(audio_bytes) if !audio_bytes.is_empty() => {
            let (mime_type, extension) = crate::bot::audio_metadata_for_external(&audio_bytes);
            let input = InputFile::memory(audio_bytes).file_name(format!("reply.{extension}"));
            let result = if mime_type == "audio/ogg" {
                {
                    let request = bot.send_voice(chat_id, input);
                    match thread {
                        Some(thread) => request.message_thread_id(thread).await,
                        None => request.await,
                    }
                    .map(|_| ())
                }
            } else {
                {
                    let request = bot.send_audio(chat_id, input);
                    match thread {
                        Some(thread) => request.message_thread_id(thread).await,
                        None => request.await,
                    }
                    .map(|_| ())
                }
            };
            result?;
        }
        outcome => {
            tracing::warn!(target: "telegram", "Voice reply unavailable: {outcome:?}");
            send_text(
                bot,
                chat_id,
                thread,
                "⚠️ Kokoro Engine could not synthesize the voice reply.",
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
pub(crate) async fn verify_bound_session_submission(services: &BotChatServices) {
    let sessions = Sessions::default();
    let chat = services.chat().unwrap();
    let state = chat.orchestrator.as_ref();
    let selected = state.selection.current_conversation_id.lock().await.clone();
    let first = sessions
        .mode_for(ChatId(901), Some(7), "test-character")
        .await;
    let old = submit_message(
        &chat.service,
        ChatId(901),
        1,
        "topic original",
        vec![],
        &first,
        "test-character",
    )
    .await
    .unwrap();
    sessions
        .start_new_for(ChatId(901), Some(7), "test-character")
        .await;
    let fresh = sessions
        .mode_for(ChatId(901), Some(7), "test-character")
        .await;
    let new = submit_message(
        &chat.service,
        ChatId(901),
        2,
        "topic new",
        vec![],
        &fresh,
        "test-character",
    )
    .await
    .unwrap();
    assert_ne!(old.conversation_id, new.conversation_id);
    sessions
        .continue_desktop_for(
            ChatId(901),
            Some(7),
            "test-character".into(),
            "ownership-a".into(),
        )
        .await;
    let continued = sessions
        .mode_for(ChatId(901), Some(7), "other-character")
        .await;
    let shared = submit_message(
        &chat.service,
        ChatId(901),
        3,
        "topic continued",
        vec![],
        &continued,
        "other-character",
    )
    .await
    .unwrap();
    assert_eq!(shared.conversation_id, "ownership-a");
    for (id, text) in [
        (&old.conversation_id, "topic original"),
        (&new.conversation_id, "topic new"),
        (&shared.conversation_id, "topic continued"),
    ] {
        let actual: String = sqlx::query_scalar(
            "SELECT conversation_id FROM conversation_messages WHERE role='user' AND content=?",
        )
        .bind(text)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(&actual, id);
    }
    let other_topic = sessions
        .mode_for(ChatId(901), Some(8), "test-character")
        .await
        .bind(ChatId(901), "test-character")
        .1;
    let other_chat = sessions
        .mode_for(ChatId(902), Some(7), "test-character")
        .await
        .bind(ChatId(902), "test-character")
        .1;
    assert_ne!(other_topic, old.conversation_id);
    assert_ne!(other_chat, old.conversation_id);
    let restart = Sessions::default()
        .mode_for(ChatId(901), Some(7), "test-character")
        .await
        .bind(ChatId(901), "test-character")
        .1;
    assert_eq!(
        restart, old.conversation_id,
        "restart reconstructs platform private mapping, never desktop selection"
    );
    assert_eq!(
        *state.selection.current_conversation_id.lock().await,
        selected
    );
}

#[cfg(test)]
mod tests {
    #[derive(Default)]
    struct EntryEffects {
        routes: Vec<&'static str>,
        model: usize,
        tools: usize,
        storage: usize,
        memory: usize,
        download: usize,
        transcription: usize,
        vision: usize,
        tts: usize,
    }

    struct RecordingMessageHandler {
        state: AIOrchestrator,
        sessions: Sessions,
        effects: std::sync::Mutex<EntryEffects>,
    }

    impl RecordingMessageHandler {
        async fn new() -> Self {
            Self {
                state: AIOrchestrator::new("sqlite::memory:").await.unwrap(),
                sessions: Sessions::default(),
                effects: std::sync::Mutex::new(EntryEffects::default()),
            }
        }

        async fn reply(&self, msg: &Message, text: &str) -> HandlerResult {
            self.effects.lock().unwrap().storage += 1;
            let turn = begin_telegram_turn(
                &self.state,
                "default",
                msg.chat.id.0,
                msg.id.0,
                text,
                false,
                &self.sessions,
            )
            .await?;
            {
                // Local substitutes for external services; persistence and
                // session mutations below use the existing application code.
                let mut effects = self.effects.lock().unwrap();
                effects.model += 1;
                effects.tools += 1;
                effects.memory += 1;
                effects.tts += 1;
            }
            turn.save_reply(&self.state, "local reply", None).await?;
            Ok(())
        }

        async fn assert_no_business_effects(&self) {
            {
                let effects = self.effects.lock().unwrap();
                assert!(effects.routes.is_empty());
                assert_eq!(effects.model, 0);
                assert_eq!(effects.tools, 0);
                assert_eq!(effects.storage, 0);
                assert_eq!(effects.memory, 0);
                assert_eq!(effects.download, 0);
                assert_eq!(effects.transcription, 0);
                assert_eq!(effects.vision, 0);
                assert_eq!(effects.tts, 0);
            }
            let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
                .fetch_one(&self.state.db)
                .await
                .unwrap();
            assert_eq!(rows, 0);
            let conversations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
                .fetch_one(&self.state.db)
                .await
                .unwrap();
            assert_eq!(conversations, 0);
            let modes = self.sessions.modes.read().await;
            assert!(modes.private.is_empty());
            assert!(modes.continued.is_empty());
            assert!(self.state.selection.history.lock().await.is_empty());
        }
    }

    impl MessageHandler for RecordingMessageHandler {
        async fn command(
            &self,
            msg: &Message,
            _text: &str,
            _config: &TelegramConfig,
        ) -> HandlerResult {
            self.effects.lock().unwrap().routes.push("command");
            // Any command reaching this substitute mutates real session state.
            self.sessions.start_new(msg.chat.id, "default").await;
            Ok(())
        }

        async fn voice(&self, msg: &Message, _config: &TelegramConfig) -> HandlerResult {
            {
                let mut effects = self.effects.lock().unwrap();
                effects.routes.push("voice");
                effects.download += 1;
                effects.transcription += 1;
            }
            self.reply(msg, "local transcript").await
        }

        async fn photo(&self, msg: &Message, _config: &TelegramConfig) -> HandlerResult {
            {
                let mut effects = self.effects.lock().unwrap();
                effects.routes.push("photo");
                effects.download += 1;
                effects.vision += 1;
            }
            self.reply(msg, "local photo description").await
        }

        async fn text(&self, msg: &Message, text: &str, _config: &TelegramConfig) -> HandlerResult {
            self.effects.lock().unwrap().routes.push("text");
            self.reply(msg, text).await
        }
    }

    fn incoming_message(chat_id: i64, message_id: i32, input: &str) -> Message {
        let mut message = serde_json::json!({
            "message_id": message_id,
            "date": 1_700_000_000,
            "chat": if chat_id < 0 {
                serde_json::json!({"id": chat_id, "type": "supergroup", "title": "Local group"})
            } else {
                serde_json::json!({"id": chat_id, "type": "private", "first_name": "Local chat"})
            },
            "from": {"id": 42, "is_bot": false, "first_name": "Local sender", "username": "allowed_name"}
        });
        match input {
            "voice" => {
                message["voice"] = serde_json::json!({
                "file_id": "voice", "file_unique_id": "voice-unique", "duration": 1,
                "file_size": 10, "mime_type": "audio/ogg"
                })
            }
            "photo" => {
                message["photo"] = serde_json::json!([{
                    "file_id": "photo", "file_unique_id": "photo-unique", "width": 1, "height": 1, "file_size": 10
                }])
            }
            "document" => {
                message["document"] = serde_json::json!({
                    "file_id": "document", "file_unique_id": "document-unique", "file_size": 10
                })
            }
            text => message["text"] = serde_json::json!(text),
        }
        let message: Message = serde_json::from_value(message).unwrap();
        match input {
            "voice" => assert!(message.voice().is_some()),
            "photo" => assert!(message.photo().is_some()),
            "document" => assert!(message.document().is_some()),
            text => assert_eq!(message.text(), Some(text)),
        }
        message
    }

    async fn local_bot() -> (Bot, wiremock::MockServer) {
        let server = wiremock::MockServer::start().await;
        let bot = Bot::new("local-test-token").set_api_url(server.uri().parse().unwrap());
        (bot, server)
    }

    #[tokio::test]
    async fn telegram_reports_chat_unavailable_only_to_authorized_chats() {
        use crate::bot::BotEvent;
        use serde_json::json;
        use wiremock::{matchers::method, Mock, ResponseTemplate};

        let (bot, telegram) = local_bot().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {
                    "message_id": 42,
                    "date": 1_700_000_000,
                    "chat": {"id": 123, "type": "private", "first_name": "Local"}
                }
            })))
            .expect(2)
            .mount(&telegram)
            .await;
        let config = Arc::new(RwLock::new(TelegramConfig {
            allowed_chat_ids: vec![123],
            ..Default::default()
        }));
        let services = BotChatServices::new(None, None, None, None, Arc::new(|_: BotEvent| Ok(())));
        let sessions = Sessions::default();
        let limiter = RateLimiter::default();
        let ingress = BotIngress::for_test();
        for (chat, id, message) in [(456, 1, "hello"), (123, 2, "/start"), (123, 3, "voice")] {
            handle_message(
                bot.clone(),
                incoming_message(chat, id, message),
                config.clone(),
                sessions.clone(),
                services.clone(),
                limiter.clone(),
                ingress.clone(),
            )
            .await
            .unwrap();
        }
        let requests = telegram.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| request
            .url
            .path()
            .to_ascii_lowercase()
            .ends_with("/sendmessage")));
        let body = |index: usize| {
            serde_json::from_slice::<serde_json::Value>(&requests[index].body).unwrap()
        };
        assert!(body(0)["text"]
            .as_str()
            .unwrap()
            .contains("Telegram Bridge"));
        assert_eq!(body(1)["text"], "Kokoro Engine chat is unavailable.");
    }

    #[tokio::test]
    async fn telegram_entry_rejects_all_routes_before_business_or_media_work() {
        let handler = RecordingMessageHandler::new().await;
        let (bot, telegram) = local_bot().await;
        let limiter = RateLimiter::default();
        // Empty, mismatched sender ID, and special values are never wildcards.
        for ids in [vec![], vec![42], vec![0, -1], vec![123]] {
            let config = RwLock::new(TelegramConfig {
                allowed_chat_ids: ids,
                ..Default::default()
            });
            for chat in [456, -1001234567890] {
                for (index, input) in [
                    "hello",
                    "/new",
                    "/continue",
                    "/status",
                    "/start",
                    "/unknown",
                    "voice",
                    "photo",
                    "document",
                ]
                .into_iter()
                .enumerate()
                {
                    dispatch_message(
                        &bot,
                        &incoming_message(chat, index as i32 + 1, input),
                        &config,
                        &limiter,
                        &handler,
                    )
                    .await
                    .unwrap();
                }
            }
        }
        handler.assert_no_business_effects().await;
        assert!(limiter.read().await.is_empty());
        assert!(telegram.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_entry_accepts_exact_signed_chat_ids_and_preserves_routes() {
        for chat in [
            123,
            -1001234567890,
            4_503_599_627_370_495,
            -4_503_599_627_370_495,
        ] {
            let handler = RecordingMessageHandler::new().await;
            let (bot, telegram) = local_bot().await;
            let limiter = RateLimiter::default();
            let config = RwLock::new(TelegramConfig {
                allowed_chat_ids: vec![chat],
                ..Default::default()
            });
            // A sign change must not authorize a different chat.
            dispatch_message(
                &bot,
                &incoming_message(-chat, 1, "hello"),
                &config,
                &limiter,
                &handler,
            )
            .await
            .unwrap();
            handler.assert_no_business_effects().await;
            for (id, input) in [
                (1, "hello"),
                (2, "/new"),
                (3, "voice"),
                (4, "photo"),
                (5, "document"),
            ] {
                dispatch_message(
                    &bot,
                    &incoming_message(chat, id, input),
                    &config,
                    &limiter,
                    &handler,
                )
                .await
                .unwrap();
            }
            {
                let effects = handler.effects.lock().unwrap();
                assert_eq!(effects.routes, ["text", "command", "voice", "photo"]);
                assert_eq!(
                    (
                        effects.model,
                        effects.tools,
                        effects.storage,
                        effects.memory,
                        effects.tts
                    ),
                    (3, 3, 3, 3, 3)
                );
                assert_eq!(
                    (effects.download, effects.transcription, effects.vision),
                    (2, 1, 1)
                );
            }
            assert!(matches!(
                handler.sessions.mode(ChatId(chat), "default").await,
                SessionMode::Fresh { .. }
            ));
            let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
                .fetch_one(&handler.state.db)
                .await
                .unwrap();
            assert_eq!(rows, 6);
            assert!(telegram.received_requests().await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn telegram_entry_uses_updated_service_whitelist_for_each_new_message() {
        let handler = RecordingMessageHandler::new().await;
        let (bot, telegram) = local_bot().await;
        let limiter = RateLimiter::default();
        let lifecycle = crate::lifecycle::AppLifecycle::new();
        let service = crate::telegram::TelegramService::new(
            TelegramConfig::default(),
            lifecycle.scope(crate::lifecycle::TaskStage::Work, "Telegram"),
            lifecycle.scope(crate::lifecycle::TaskStage::Service, "Telegram"),
        );
        for (ids, chat, id, input, expected_calls) in [
            (vec![123], 123, 1, "hello", 1),
            (vec![-1001234567890], 123, 2, "/new", 1),
            (vec![-1001234567890], -1001234567890, 3, "photo", 2),
            (vec![], -1001234567890, 4, "voice", 2),
            (vec![], 123, 5, "/continue", 2),
            (vec![123], 123, 6, "/new", 3),
        ] {
            service
                .update_config(TelegramConfig {
                    allowed_chat_ids: ids,
                    ..Default::default()
                })
                .await;
            dispatch_message(
                &bot,
                &incoming_message(chat, id, input),
                &service.config,
                &limiter,
                &handler,
            )
            .await
            .unwrap();
            assert_eq!(handler.effects.lock().unwrap().routes.len(), expected_calls);
        }
        {
            let effects = handler.effects.lock().unwrap();
            assert_eq!(effects.routes, ["text", "photo", "command"]);
            assert_eq!(effects.transcription, 0);
        }
        assert!(telegram.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_entry_rejects_missing_or_invalid_authorization_config() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telegram.json");
        let handler = RecordingMessageHandler::new().await;
        let (bot, telegram) = local_bot().await;
        let limiter = RateLimiter::default();
        for contents in [
            None,
            Some("{}"),
            Some(r#"{"enabled":true,"bot_token":"local-test-token"}"#),
            Some("not json"),
            Some(r#"{"allowed_chat_ids":null}"#),
            Some(r#"{"allowed_chat_ids":["*"]}"#),
            Some(r#"{"allowed_chat_ids":[1.5]}"#),
        ] {
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            let config = RwLock::new(super::super::config::load_config(&path));
            for input in ["hello", "/new", "voice", "photo"] {
                dispatch_message(
                    &bot,
                    &incoming_message(123, 1, input),
                    &config,
                    &limiter,
                    &handler,
                )
                .await
                .unwrap();
            }
        }
        handler.assert_no_business_effects().await;
        assert!(limiter.read().await.is_empty());
        assert!(telegram.received_requests().await.unwrap().is_empty());
    }

    async fn start_new_telegram_session(
        _state: &AIOrchestrator,
        sessions: &Sessions,
        character: &str,
        chat: ChatId,
    ) {
        sessions.start_new(chat, character).await;
    }
    async fn begin_telegram_turn(
        state: &AIOrchestrator,
        character: &str,
        chat: i64,
        message: i32,
        text: &str,
        continue_selected: bool,
        sessions: &Sessions,
    ) -> Result<ExternalTurn, KokoroError> {
        let mode = sessions.mode(ChatId(chat), character).await;
        let (_, conversation, _) = mode.bind(ChatId(chat), character);
        ExternalTurn::begin(
            state,
            character,
            conversation,
            telegram_request_id(chat, message),
            text,
            continue_selected,
        )
        .await
    }
    #[test]
    fn topic_delivery_payloads_keep_the_origin_thread() {
        use teloxide::requests::HasPayload;
        let bot = Bot::new("fixture-token");
        let chat = ChatId(901);
        let thread = teloxide::types::ThreadId(teloxide::types::MessageId(7));
        let text = bot.send_message(chat, "reply").message_thread_id(thread);
        let voice = bot
            .send_voice(chat, InputFile::memory(vec![1u8]))
            .message_thread_id(thread);
        let audio = bot
            .send_audio(chat, InputFile::memory(vec![2u8]))
            .message_thread_id(thread);
        let photo = bot
            .send_photo(chat, InputFile::memory(vec![3u8]))
            .message_thread_id(thread);
        for payload in [
            serde_json::to_value(text.payload_ref()).unwrap(),
            serde_json::to_value(voice.payload_ref()).unwrap(),
            serde_json::to_value(audio.payload_ref()).unwrap(),
            serde_json::to_value(photo.payload_ref()).unwrap(),
        ] {
            assert_eq!(payload["chat_id"], 901);
            assert_eq!(payload["message_thread_id"], 7);
        }
    }

    #[tokio::test]
    async fn new_rotates_only_its_character_and_chat_while_continue_stays_explicitly_bound() {
        let sessions = Sessions::default();
        sessions.start_new(ChatId(10), "a").await;
        sessions.start_new(ChatId(10), "b").await;
        sessions.start_new(ChatId(11), "a").await;
        let before_a = sessions.mode(ChatId(10), "a").await.bind(ChatId(10), "a").1;
        let before_b = sessions.mode(ChatId(10), "b").await.bind(ChatId(10), "b").1;
        let before_other = sessions.mode(ChatId(11), "a").await.bind(ChatId(11), "a").1;
        sessions
            .continue_desktop(ChatId(10), "a".into(), "desktop-a".into())
            .await;
        let pinned = sessions.mode(ChatId(10), "b").await;
        assert_eq!(
            pinned.bind(ChatId(10), "b"),
            ("a", "desktop-a".into(), false)
        );
        sessions.start_new(ChatId(10), "b").await;
        assert_eq!(
            sessions.mode(ChatId(10), "a").await.bind(ChatId(10), "a").1,
            before_a
        );
        assert_ne!(
            sessions.mode(ChatId(10), "b").await.bind(ChatId(10), "b").1,
            before_b
        );
        assert_eq!(
            sessions.mode(ChatId(11), "a").await.bind(ChatId(11), "a").1,
            before_other
        );
    }

    #[derive(Default)]
    struct RecordingDelivery {
        texts: std::sync::Mutex<Vec<String>>,
        voices: std::sync::Mutex<Vec<String>>,
        images: std::sync::Mutex<Vec<String>>,
        fail_text_once: std::sync::atomic::AtomicBool,
    }

    impl TelegramReplyDelivery for RecordingDelivery {
        async fn text(&self, text: String) -> Result<(), TelegramDeliveryError> {
            self.texts.lock().unwrap().push(text);
            if self
                .fail_text_once
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(std::io::Error::other("injected Telegram send failure").into());
            }
            Ok(())
        }
        async fn voice(&self, text: &str) {
            self.voices.lock().unwrap().push(text.to_owned());
        }
        async fn images(&self, text: &str) {
            assert!(text.contains("[IMAGE_PROMPT:a forest]"));
            self.images.lock().unwrap().push(text.to_owned());
        }
    }

    /// A generation substitute exercises the same replay decision and shared
    /// delivery path without a model, tool service, or Telegram credentials.
    async fn delivery_attempt(
        state: &AIOrchestrator,
        sessions: &Sessions,
        input: &str,
        photos: bool,
        generation_calls: &std::sync::atomic::AtomicUsize,
        delivery: &RecordingDelivery,
    ) -> Result<(), TelegramDeliveryError> {
        let turn = begin_telegram_turn(state, "default", 10, 1, input, false, sessions).await?;
        let (text, translation, newly_committed) =
            if let Some((text, translation)) = &turn.previous_reply {
                (
                    text.clone(),
                    translation.clone(),
                    turn.previous_reply_newly_committed,
                )
            } else {
                generation_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let text = "**answer** [IMAGE_PROMPT:a forest]".to_owned();
                turn.save_reply(state, &text, Some("translation")).await?;
                (text, Some("translation".into()), true)
            };
        deliver_telegram_reply(
            sessions,
            (&turn.target.conversation_id, turn.request_id()),
            TelegramReply {
                text: &text,
                translation: translation.as_deref(),
                newly_committed,
                voice: true,
                images: !photos,
            },
            delivery,
        )
        .await
    }

    #[tokio::test]
    async fn telegram_media_replay_reuses_committed_input_before_preparation() {
        for photos in [false, true] {
            for fail_save in [false, true] {
                let state = AIOrchestrator::new("sqlite::memory:")
                    .await
                    .unwrap()
                    .clone();
                let sessions = Sessions::default();
                let delivery = RecordingDelivery::default();
                let generations = std::sync::atomic::AtomicUsize::new(0);
                let preparations = std::sync::atomic::AtomicUsize::new(0);
                let original = "  Original transcript, with punctuation!\n";
                let input = prepare_telegram_media_input(&state, "default", 10, 1, || async {
                    preparations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(Some(TelegramPreparedInput {
                        text: original.into(),
                        image: photos.then(|| "data:image/jpeg;base64,aW1hZ2U=".into()),
                    }))
                })
                .await
                .unwrap()
                .unwrap();
                assert_eq!(input.image.is_some(), photos);
                if fail_save {
                    sqlx::query("CREATE TRIGGER reject_media_reply BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'reply save fault'); END")
                        .execute(&state.db).await.unwrap();
                }
                let first = delivery_attempt(
                    &state,
                    &sessions,
                    &input.text,
                    photos,
                    &generations,
                    &delivery,
                )
                .await;
                assert_eq!(first.is_err(), fail_save);
                let original_conversation: String = sqlx::query_scalar(
                    "SELECT conversation_id FROM conversation_messages WHERE role = 'user'",
                )
                .fetch_one(&state.db)
                .await
                .unwrap();
                // Reproduce the old path: a second STT result with only spacing
                // and punctuation changes is rejected before reply recovery.
                let error = begin_telegram_turn(
                    &state,
                    "default",
                    10,
                    1,
                    "Original transcript with punctuation",
                    false,
                    &sessions,
                )
                .await
                .err()
                .unwrap();
                // The public error intentionally redacts SQLite details.
                assert!(matches!(error, KokoroError::ChatPersistence(_)));
                // Both a new /new session and a fresh request orchestrator must
                // still recover the original request's transcript and target.
                start_new_telegram_session(&state, &sessions, "default", ChatId(10)).await;
                if fail_save {
                    // A drifted second STT result must not be attempted, even
                    // while the final SQLite write is still failing.
                    let fresh = state.clone();
                    let replay = prepare_telegram_media_input(&fresh, "default", 10, 1, || async {
                        preparations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(Some(TelegramPreparedInput {
                            text: "Original transcript with punctuation".into(),
                            image: None,
                        }))
                    })
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(replay.text, original);
                    assert!(delivery_attempt(
                        &fresh,
                        &sessions,
                        &replay.text,
                        photos,
                        &generations,
                        &delivery
                    )
                    .await
                    .is_err());
                    sqlx::query("DROP TRIGGER reject_media_reply")
                        .execute(&state.db)
                        .await
                        .unwrap();
                }
                for _ in 0..2 {
                    let fresh = state.clone();
                    let replay = prepare_telegram_media_input(&fresh, "default", 10, 1, || async {
                        preparations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(std::io::Error::other("STT unavailable / Telegram file expired").into())
                    })
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(replay.text, original);
                    assert!(replay.image.is_none());
                    delivery_attempt(
                        &fresh,
                        &sessions,
                        &replay.text,
                        photos,
                        &generations,
                        &delivery,
                    )
                    .await
                    .unwrap();
                }
                assert_eq!(preparations.load(std::sync::atomic::Ordering::SeqCst), 1);
                assert_eq!(generations.load(std::sync::atomic::Ordering::SeqCst), 1);
                assert_eq!(
                    delivery.texts.lock().unwrap().len(),
                    if fail_save { 2 } else { 3 }
                );
                assert_eq!(delivery.voices.lock().unwrap().len(), 1);
                assert_eq!(delivery.images.lock().unwrap().len(), usize::from(!photos));
                let rows: Vec<(String, String, String)> = sqlx::query_as(
                    "SELECT conversation_id, role, content FROM conversation_messages ORDER BY id",
                )
                .fetch_all(&state.db)
                .await
                .unwrap();
                assert_eq!(
                    rows,
                    [
                        (
                            original_conversation.clone(),
                            "user".into(),
                            original.into()
                        ),
                        (
                            original_conversation,
                            "assistant".into(),
                            "**answer** [IMAGE_PROMPT:a forest]".into()
                        ),
                    ]
                );
            }
        }
    }

    #[tokio::test]
    async fn telegram_media_preparation_failure_keeps_new_input_retryable() {
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        // Download/network failure and unrecognized/unavailable STT produce no
        // user row, so the same platform message can be prepared successfully.
        assert!(
            prepare_telegram_media_input(&state, "default", 10, 1, || async {
                Err(std::io::Error::other("download failed").into())
            })
            .await
            .is_err()
        );
        assert!(
            prepare_telegram_media_input(&state, "default", 10, 1, || async { Ok(None) })
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let input = prepare_telegram_media_input(&state, "default", 10, 1, || async {
            Ok(Some(TelegramPreparedInput {
                text: "retry succeeded".into(),
                image: None,
            }))
        })
        .await
        .unwrap()
        .unwrap();
        let sessions = Sessions::default();
        let turn = begin_telegram_turn(&state, "default", 10, 1, &input.text, false, &sessions)
            .await
            .unwrap();
        turn.save_reply(&state, "answer", None).await.unwrap();
        let replay = prepare_telegram_media_input(&state, "default", 10, 1, || async {
            panic!("completed request must not download or transcribe again")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(replay.text, "retry succeeded");
    }

    #[tokio::test]
    async fn telegram_saved_media_input_is_scoped_to_character_chat_and_message() {
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let sessions = Sessions::default();
        sqlx::query("INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES ('other', 'Other', '', '', 'test', 1, 1)")
            .execute(&state.db).await.unwrap();
        for (character, chat, message, transcript) in [
            ("default", 10, 1, "original"),
            ("other", 10, 3, "other character"),
            ("default", 11, 1, "other chat"),
            ("default", 10, 2, "other message"),
        ] {
            let input = prepare_telegram_media_input(&state, character, chat, message, || async {
                Ok(Some(TelegramPreparedInput {
                    text: transcript.into(),
                    image: None,
                }))
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(input.text, transcript);
            let turn = begin_telegram_turn(
                &state,
                character,
                chat,
                message,
                &input.text,
                false,
                &sessions,
            )
            .await
            .unwrap();
            turn.save_reply(&state, "answer", None).await.unwrap();
            let replay = prepare_telegram_media_input(&state, character, chat, message, || async {
                panic!("scoped committed input must be reused")
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(replay.text, transcript);
        }
        let conflict = prepare_telegram_media_input(&state, "other", 10, 1, || async {
            panic!("a request owned by another character must not trigger media work")
        })
        .await
        .err()
        .unwrap();
        assert!(
            matches!(conflict.downcast_ref::<KokoroError>(), Some(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
        );
        let conversation: String =
            sqlx::query_scalar("SELECT id FROM conversations WHERE character_id='default' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES (?,'user','legacy transcript',?,'now')")
            .bind(conversation)
            .bind(serde_json::json!({"source":"telegram","client_request_id":persistence::legacy_external_identity("telegram", "10", "4")}).to_string())
            .execute(&state.db).await.unwrap();
        for character in ["default", "other"] {
            let conflict = prepare_telegram_media_input(&state, character, 10, 4, || async {
                panic!("legacy accepted input must not trigger media work")
            })
            .await
            .err()
            .unwrap();
            assert!(
                matches!(conflict.downcast_ref::<KokoroError>(), Some(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
            );
        }
        // Read errors cannot be misinterpreted as a fresh request and trigger STT.
        state.db.close().await;
        let error = prepare_telegram_media_input(&state, "default", 10, 1, || async {
            panic!("failed persisted-input read must not trigger media work")
        })
        .await
        .err()
        .unwrap();
        assert!(matches!(
            error.downcast_ref::<KokoroError>(),
            Some(KokoroError::ChatPersistence(_))
        ));
    }

    #[tokio::test]
    async fn telegram_save_recovery_delivers_text_voice_and_images_once_per_input_kind() {
        for (input, photos) in [
            ("typed prompt", false),
            ("voice transcript", false),
            ("photo caption", true),
        ] {
            let state = AIOrchestrator::new("sqlite::memory:")
                .await
                .unwrap()
                .clone();
            // Replay delivery must not depend on whether memory extraction ran.
            state.set_memory_enabled(false).await;
            let sessions = Sessions::default();
            let delivery = RecordingDelivery::default();
            let generations = std::sync::atomic::AtomicUsize::new(0);
            sqlx::query("CREATE TRIGGER reject_delivery_reply BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'reply save fault'); END")
                .execute(&state.db).await.unwrap();
            for _ in 0..2 {
                let error =
                    delivery_attempt(&state, &sessions, input, photos, &generations, &delivery)
                        .await
                        .unwrap_err();
                assert!(matches!(
                    error.downcast_ref::<KokoroError>(),
                    Some(KokoroError::ChatPersistence(_))
                ));
            }
            assert!(delivery.texts.lock().unwrap().is_empty());
            assert!(delivery.voices.lock().unwrap().is_empty());
            assert!(delivery.images.lock().unwrap().is_empty());
            sqlx::query("DROP TRIGGER reject_delivery_reply")
                .execute(&state.db)
                .await
                .unwrap();
            for _ in 0..2 {
                delivery_attempt(&state, &sessions, input, photos, &generations, &delivery)
                    .await
                    .unwrap();
            }
            assert_eq!(generations.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(delivery.texts.lock().unwrap().len(), 2);
            assert!(delivery.texts.lock().unwrap()[0].ends_with("🌐 translation"));
            assert_eq!(delivery.voices.lock().unwrap().len(), 1);
            assert_eq!(delivery.images.lock().unwrap().len(), usize::from(!photos));
            let roles: Vec<String> =
                sqlx::query_scalar("SELECT role FROM conversation_messages ORDER BY id")
                    .fetch_all(&state.db)
                    .await
                    .unwrap();
            assert_eq!(roles, ["user", "assistant"]);
            // Lost in-process progress must not regenerate already delivered media.
            delivery_attempt(
                &state,
                &Sessions::default(),
                input,
                photos,
                &generations,
                &delivery,
            )
            .await
            .unwrap();
            assert_eq!(delivery.voices.lock().unwrap().len(), 1);
            assert_eq!(delivery.images.lock().unwrap().len(), usize::from(!photos));
        }
    }

    #[tokio::test]
    async fn telegram_replay_continues_media_after_text_delivery_failure_without_regeneration() {
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let sessions = Sessions::default();
        let delivery = RecordingDelivery::default();
        delivery
            .fail_text_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let generations = std::sync::atomic::AtomicUsize::new(0);
        assert!(
            delivery_attempt(&state, &sessions, "prompt", false, &generations, &delivery)
                .await
                .is_err()
        );
        assert!(delivery.voices.lock().unwrap().is_empty());
        assert!(delivery.images.lock().unwrap().is_empty());
        let (first, duplicate) = tokio::join!(
            delivery_attempt(&state, &sessions, "prompt", false, &generations, &delivery),
            delivery_attempt(&state, &sessions, "prompt", false, &generations, &delivery),
        );
        first.unwrap();
        duplicate.unwrap();
        assert_eq!(generations.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(delivery.texts.lock().unwrap().len(), 3);
        assert_eq!(delivery.voices.lock().unwrap().len(), 1);
        assert_eq!(delivery.images.lock().unwrap().len(), 1);
        let assistants: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE role = 'assistant'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(assistants, 1);
    }

    use super::*;

    #[test]
    fn new_and_default_sessions_are_isolated_while_continue_pins_desktop_scope() {
        let default = SessionMode::Private;
        let other = default.bind(ChatId(2), "character").1;
        let first = default.bind(ChatId(1), "character").1;
        assert_ne!(first, other);
        assert_ne!(first, default.bind(ChatId(1), "another-character").1);
        let fresh = SessionMode::Fresh { id: "fresh".into() };
        assert_ne!(fresh.bind(ChatId(1), "character").1, first);
        assert_eq!(default.bind(ChatId(2), "character").1, other);
        let continue_desktop = SessionMode::Desktop {
            character_id: "character".into(),
            conversation_id: "selected".into(),
        };
        assert_eq!(
            continue_desktop.bind(ChatId(1), "switched"),
            ("character", "selected".into(), false)
        );
    }

    #[test]
    fn splitting_preserves_the_message_and_handles_multibyte_characters() {
        let original = "界".repeat(MAX_MESSAGE_CHARS + 3);
        let chunks = split_message(&original).collect::<Vec<_>>();
        assert_eq!(chunks[0].chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(chunks.concat(), original);
    }
    #[tokio::test]
    async fn telegram_memory_cadence_follows_mixed_messages_and_new_sessions() {
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let sessions = Sessions::default();
        sqlx::query("INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES ('memory-bot', 'Bot', '', '', 'test', 1, 1)")
            .execute(&state.db).await.unwrap();
        let mut extractions = 0;
        let mut consolidations = 0;
        for id in 1..=20 {
            let fork = state.clone();
            fork.apply_character_profile("memory-bot").await.unwrap();
            let text = ["typed prompt", "photo caption", "voice transcript"][id as usize % 3];
            let turn = begin_telegram_turn(&fork, "memory-bot", 10, id, text, false, &sessions)
                .await
                .unwrap();
            let work = turn
                .save_reply(&fork, "answer", None)
                .await
                .unwrap()
                .unwrap();
            extractions += usize::from(work.extract());
            consolidations += usize::from(work.consolidate());
            assert_eq!(work.count, id as u64);
            assert!(!turn
                .recent_committed_history(&fork, work, 10)
                .await
                .is_empty());
        }
        assert_eq!((extractions, consolidations), (4, 1));
        start_new_telegram_session(&state, &sessions, "memory-bot", ChatId(10)).await;
        let fork = state.clone();
        let turn = begin_telegram_turn(&fork, "memory-bot", 10, 21, "new prompt", false, &sessions)
            .await
            .unwrap();
        let work = turn
            .save_reply(&fork, "new answer", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(work.count, 1);
        let history = turn.recent_committed_history(&fork, work, 10).await;
        assert_eq!(
            history
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["new prompt", "new answer"]
        );
        assert_eq!(state.get_memory_trigger_count().await, 0);
    }

    #[tokio::test]
    async fn telegram_initial_write_failure_propagates_before_text_or_photo_can_generate() {
        let sessions = Sessions::default();
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        sqlx::query(
            "CREATE TRIGGER reject_telegram_user BEFORE INSERT ON conversation_messages
            WHEN NEW.role = 'user' BEGIN SELECT RAISE(ABORT, 'Telegram user fault'); END",
        )
        .execute(&state.db)
        .await
        .unwrap();
        for (id, text) in [
            (1, "typed prompt"),
            (2, "photo caption"),
            (3, "voice transcript"),
        ] {
            assert!(matches!(
                begin_telegram_turn(&state, "default", 10, id, text, true, &sessions).await,
                Err(KokoroError::ChatPersistence(_))
            ));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(state.selection.history.lock().await.is_empty());
        assert!(state
            .selection
            .current_conversation_id
            .lock()
            .await
            .is_none());
    }

    #[tokio::test]
    async fn telegram_redelivery_retries_only_the_saved_reply_in_its_original_conversation() {
        let sessions = Sessions::default();
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let first =
            begin_telegram_turn(&state, "default", 10, 1, " typed prompt ", true, &sessions)
                .await
                .unwrap();
        assert!(first.previous_reply.is_none());
        state.clear_history().await;
        sessions.start_new(ChatId(10), "default").await;
        let second =
            begin_telegram_turn(&state, "default", 10, 2, "second question", true, &sessions)
                .await
                .unwrap();
        assert_ne!(second.target.conversation_id, first.target.conversation_id);
        sqlx::query(
            "CREATE TRIGGER reject_telegram_assistant BEFORE INSERT ON conversation_messages
            WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'Telegram assistant fault'); END",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let raw_reply = "**full reply**\n\n\nnext paragraph";
        assert!(matches!(
            first
                .save_reply(&state, raw_reply, Some("translation"))
                .await,
            Err(KokoroError::ChatPersistence(_))
        ));
        assert!(matches!(
            begin_telegram_turn(&state, "default", 10, 1, " typed prompt ", true, &sessions).await,
            Err(KokoroError::ChatPersistence(_))
        ));
        sqlx::query("DROP TRIGGER reject_telegram_assistant")
            .execute(&state.db)
            .await
            .unwrap();
        for _ in 0..2 {
            let retry =
                begin_telegram_turn(&state, "default", 10, 1, " typed prompt ", true, &sessions)
                    .await
                    .unwrap();
            assert_eq!(retry.target.conversation_id, first.target.conversation_id);
            assert_eq!(
                retry.previous_reply,
                Some((raw_reply.into(), Some("translation".into())))
            );
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT role, content FROM conversation_messages WHERE conversation_id = ? ORDER BY id",
        )
        .bind(&first.target.conversation_id)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(
            rows,
            [
                ("user".into(), " typed prompt ".into()),
                ("assistant".into(), raw_reply.into())
            ]
        );
        let wrong: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages WHERE role = 'assistant' AND conversation_id = ?")
            .bind(&second.target.conversation_id).fetch_one(&state.db).await.unwrap();
        assert_eq!(wrong, 0);
        assert!(
            state.selection.history.lock().await.is_empty(),
            "platform session rotation does not edit the desktop projection"
        );
    }
}
