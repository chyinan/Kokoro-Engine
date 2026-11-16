//! Telegram Bot core — message handling, command dispatch, and AI pipeline bridge.
// pattern: Mixed (unavoidable)
// Reason: Telegram 机器人处理网络 I/O、消息路由、持久化和异步任务调度，属于编排层。

use super::config::TelegramConfig;
use crate::ai::context::AIOrchestrator;
use crate::chat::application::{channel_id, submit_chat, ChatPolicy, ChatRequest};
use crate::chat::persistence::{self, ExternalTurn};
use crate::error::KokoroError;
use crate::stt::{AudioSource, SttService};
use crate::tts::TtsService;
use base64::Engine as _;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use teloxide::prelude::*;
use teloxide::types::InputFile;
use tokio::sync::{oneshot, RwLock};

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
    private: HashMap<(ChatId, String), SessionMode>,
    continued: HashMap<ChatId, SessionMode>,
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
    async fn mode(&self, chat: ChatId, character: &str) -> SessionMode {
        let modes = self.modes.read().await;
        modes
            .continued
            .get(&chat)
            .or_else(|| modes.private.get(&(chat, character.to_owned())))
            .cloned()
            .unwrap_or_default()
    }

    async fn start_new(&self, chat: ChatId, character: &str) {
        let mut modes = self.modes.write().await;
        modes.continued.remove(&chat);
        modes.private.insert(
            (chat, character.to_owned()),
            SessionMode::Fresh {
                id: uuid::Uuid::new_v4().to_string(),
            },
        );
    }

    async fn continue_desktop(&self, chat: ChatId, character_id: String, conversation_id: String) {
        self.modes.write().await.continued.insert(
            chat,
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

trait TelegramReplyDelivery: Sync {
    fn text(
        &self,
        text: String,
    ) -> impl std::future::Future<Output = Result<(), TelegramDeliveryError>> + Send;
    fn voice(&self, text: &str) -> impl std::future::Future<Output = ()> + Send;
    fn images(&self, text: &str) -> impl std::future::Future<Output = ()> + Send;
}

struct TelegramDelivery<'a> {
    bot: &'a Bot,
    chat_id: ChatId,
    app: &'a tauri::AppHandle,
    media: &'a persistence::ExternalReplyMedia,
}

impl TelegramReplyDelivery for TelegramDelivery<'_> {
    async fn text(&self, text: String) -> Result<(), TelegramDeliveryError> {
        for chunk in split_message(&text) {
            self.bot.send_message(self.chat_id, chunk).await?;
        }
        Ok(())
    }
    async fn voice(&self, text: &str) {
        let _ = send_voice_reply(self.bot, self.chat_id, text, self.app).await;
    }
    async fn images(&self, _text: &str) {
        for image in &self.media.images {
            if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(&image.data_base64) {
                if let Err(error) = self
                    .bot
                    .send_photo(
                        self.chat_id,
                        InputFile::memory(data).file_name(image.file_name.clone()),
                    )
                    .await
                {
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

/// Run the long-polling loop. Blocks until `shutdown_rx` fires or an error occurs.
pub async fn run_polling(
    token: String,
    config: Arc<RwLock<TelegramConfig>>,
    app: tauri::AppHandle,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let bot = Bot::new(&token);
    let sessions = Sessions::default();
    let rate_limiter: RateLimiter = Arc::new(RwLock::new(HashMap::new()));

    // Build the update handler
    let handler = Update::filter_message().endpoint(handle_message);

    let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
        .dependencies(dptree::deps![
            config.clone(),
            sessions.clone(),
            app.clone(),
            rate_limiter.clone()
        ])
        .default_handler(|_upd| async {})
        .build();

    // Run dispatcher in a spawned task and monitor its lifecycle.
    let shutdown_token = dispatcher.shutdown_token();
    let mut dispatch_task = tauri::async_runtime::spawn(async move {
        dispatcher.dispatch().await;
    });

    tokio::select! {
        _ = &mut shutdown_rx => {
            if let Ok(fut) = shutdown_token.shutdown() {
                fut.await;
            }
            let _ = (&mut dispatch_task).await;
        }
        dispatch_result = &mut dispatch_task => {
            match dispatch_result {
                Ok(_) => {
                    tracing::error!(target: "telegram", "dispatcher exited unexpectedly");
                }
                Err(dispatch_err) => {
                    tracing::error!(
                        target: "telegram",
                        "dispatcher task failed: {}",
                        dispatch_err
                    );
                }
            }
        }
    }
}

/// Central message handler — dispatches commands and regular messages.
async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<RwLock<TelegramConfig>>,
    sessions: Sessions,
    app: tauri::AppHandle,
    rate_limiter: RateLimiter,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;
    // 不记录消息内容，避免敏感信息泄露到日志
    tracing::info!(target: "telegram", "[Telegram] Received message from chat_id={}", chat_id.0);

    // Snapshot config for this request
    let config = Arc::new(config.read().await.clone());

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
            bot.send_message(chat_id, "⚠️ Too many messages. Please wait a moment.")
                .await
                .ok();
            return Ok(());
        }
    }

    // Check for commands
    if let Some(text) = msg.text() {
        if text.starts_with('/') {
            return handle_command(&bot, &msg, text, &config, &sessions, &app).await;
        }
    }

    // Voice message
    if msg.voice().is_some() {
        return handle_voice(&bot, &msg, &config, &sessions, &app).await;
    }

    // Photo message (with optional caption)
    if msg.photo().is_some() {
        return handle_photo(&bot, &msg, &config, &sessions, &app).await;
    }

    // Regular text message
    if let Some(text) = msg.text() {
        return reply_to_message(&bot, &msg, text, Vec::new(), &config, &sessions, &app).await;
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
    app: &tauri::AppHandle,
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
            if let Err(e) = bot.send_message(chat_id, text).await {
                tracing::error!(target: "telegram", "[Telegram] Failed to send /start reply: {}", e);
            }
        }
        "/new" => {
            let state = app.state::<AIOrchestrator>();
            let _guard = state.enter_chat_turn()?;
            let active = state.get_character_id().await;
            let character = config
                .character_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(&active);
            sessions.start_new(chat_id, character).await;
            bot.send_message(chat_id, "✨ New Telegram conversation started.")
                .await?;
        }
        "/continue" => {
            let state = app.state::<AIOrchestrator>();
            let (character_id, conversation_id) = {
                let _activation = state.enter_chat_turn()?;
                let _selection = state.conversation_switch_lock.lock().await;
                (
                    state.get_character_id().await,
                    state.current_conversation_id.lock().await.clone(),
                )
            };
            let Some(conversation_id) = conversation_id else {
                bot.send_message(
                    chat_id,
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
                bot.send_message(
                    chat_id,
                    "Telegram is configured to use a different character.",
                )
                .await?;
                return Ok(());
            }
            sessions
                .continue_desktop(chat_id, character_id, conversation_id)
                .await;
            bot.send_message(chat_id, "🔗 Continuing the selected desktop conversation.")
                .await?;
        }
        "/status" => {
            let state = app.state::<AIOrchestrator>();
            let active_character = state.get_character_id().await;
            let character = config
                .character_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(&active_character);
            let mode = sessions.mode(chat_id, character).await;
            let (character, conversation, _) = mode.bind(chat_id, character);
            let history_len = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM conversation_messages AS m JOIN conversations AS c ON m.conversation_id = c.id
                 WHERE c.id = ? AND c.character_id = ?"
            ).bind(conversation).bind(character).fetch_one(&state.db).await?;
            let mode_str = match mode {
                SessionMode::Desktop { .. } => "Desktop conversation",
                _ => "Telegram conversation",
            };
            bot.send_message(
                chat_id,
                format!("📊 Session: {mode_str}\n💬 History: {history_len} messages"),
            )
            .await?;
        }
        _ => {
            if !text.trim_start_matches('/').trim().is_empty() {
                reply_to_message(bot, msg, text, Vec::new(), config, sessions, app).await?;
            }
        }
    }
    Ok(())
}

fn telegram_request_id(chat_id: i64, message_id: i32) -> String {
    persistence::external_identity("telegram", &[&chat_id.to_string(), &message_id.to_string()])
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
    app: &tauri::AppHandle,
) -> HandlerResult {
    let state = app.state::<AIOrchestrator>();
    let _guard = state.enter_chat_turn()?;
    let active_character = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active_character);
    let mode = sessions.mode(msg.chat.id, preferred).await;
    reply_bound(
        bot, msg, text, images, config, sessions, app, &mode, preferred, None,
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
    app: &tauri::AppHandle,
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
        bot.send_message(
            chat_id,
            "The configured character changed. Use /new or /continue to select a conversation.",
        )
        .await?;
        return Ok(());
    }
    let result = if input_text.is_some() {
        submit_message_input(
            app, chat_id, msg.id.0, text, images, mode, preferred, input_text,
        )
        .await
    } else {
        submit_message(app, chat_id, msg.id.0, text, images, mode, preferred).await
    };
    let result = match result {
        Ok(result) if result.status.as_deref() == Some("cancelled") => {
            bot.send_message(chat_id, "Chat cancelled.").await?;
            return Ok(());
        }
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(target: "telegram", "Chat generation failed: {error}");
            let feedback = if error.to_string().contains("chat_turn_busy") {
                "Another chat is in progress. Send a new message after it finishes."
            } else {
                "Kokoro Engine could not complete this chat. Check the client for details."
            };
            bot.send_message(chat_id, feedback).await?;
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
            bot,
            chat_id,
            app,
            media: &result.external_media,
        },
    )
    .await
}

pub(crate) async fn submit_message(
    app: &tauri::AppHandle,
    chat_id: ChatId,
    message_id: i32,
    text: &str,
    images: Vec<String>,
    mode: &SessionMode,
    preferred_character: &str,
) -> Result<crate::chat::application::StreamChatResponse, crate::error::KokoroError> {
    submit_message_input(
        app,
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
    app: &tauri::AppHandle,
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
    submit_chat(
        app.clone(),
        ChatRequest {
            message: text.to_owned(),
            character_id: Some(character_id.to_owned()),
            conversation_id: Some(conversation_id),
            // Telegram message identifiers are unique within each chat.
            client_request_id: Some(channel_id(
                "telegram",
                &chat_id.0.to_string(),
                &message_id.to_string(),
            )),
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
    config: &Arc<TelegramConfig>,
    sessions: &Sessions,
    app: &tauri::AppHandle,
) -> HandlerResult {
    let state = app.state::<AIOrchestrator>();
    let _guard = state.enter_chat_turn()?;
    let active = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active);
    let mode = sessions.mode(msg.chat.id, preferred).await;
    let (character, _, _) = mode.bind(msg.chat.id, preferred);
    let input =
        prepare_telegram_media_input(&state, character, msg.chat.id.0, msg.id.0, || async {
            let voice = msg.voice().ok_or("No voice data")?;
            if voice.file.size > MAX_MEDIA_BYTES {
                return Err("Voice message exceeds the 20 MB limit".into());
            }
            let file = bot.get_file(&voice.file.id).await?;
            let mut data = Vec::new();
            teloxide::net::Download::download_file(bot, &file.path, &mut data).await?;
            let stt = app
                .try_state::<SttService>()
                .ok_or("STT service not available")?;
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
            app,
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
    app: &tauri::AppHandle,
) -> HandlerResult {
    let state = app.state::<AIOrchestrator>();
    let _guard = state.enter_chat_turn()?;
    let active = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active);
    let mode = sessions.mode(msg.chat.id, preferred).await;
    let (character, _, _) = mode.bind(msg.chat.id, preferred);
    let input =
        prepare_telegram_media_input(&state, character, msg.chat.id.0, msg.id.0, || async {
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
            app,
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
    text: &str,
    app: &tauri::AppHandle,
) -> HandlerResult {
    let tts_service = match app.try_state::<TtsService>() {
        Some(s) => s,
        None => {
            bot.send_message(chat_id, "⚠️ The TTS service is unavailable.")
                .await?;
            return Ok(());
        }
    };

    match tts_service.synthesize_text(text, None).await {
        Ok(audio_bytes) if !audio_bytes.is_empty() => {
            let (mime_type, extension) =
                crate::commands::bot::audio_metadata_for_external(&audio_bytes);
            let input = InputFile::memory(audio_bytes).file_name(format!("reply.{extension}"));
            let result = if mime_type == "audio/ogg" {
                bot.send_voice(chat_id, input).await.map(|_| ())
            } else {
                bot.send_audio(chat_id, input).await.map(|_| ())
            };
            result?;
        }
        outcome => {
            tracing::warn!(target: "telegram", "Voice reply unavailable: {outcome:?}");
            bot.send_message(
                chat_id,
                "⚠️ Kokoro Engine could not synthesize the voice reply.",
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

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
                    .fork_with_isolated_history()
                    .await;
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
                    let fresh = state.fork_with_isolated_history().await;
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
                    let fresh = state.fork_with_isolated_history().await;
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
            .fork_with_isolated_history()
            .await;
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
            .fork_with_isolated_history()
            .await;
        let sessions = Sessions::default();
        sqlx::query("INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES ('other', 'Other', '', '', 'test', 1, 1)")
            .execute(&state.db).await.unwrap();
        for (character, chat, message, transcript) in [
            ("default", 10, 1, "original"),
            ("other", 10, 1, "other character"),
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
                .fork_with_isolated_history()
                .await;
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
            .fork_with_isolated_history()
            .await;
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
            .fork_with_isolated_history()
            .await;
        let sessions = Sessions::default();
        sqlx::query("INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES ('memory-bot', 'Bot', '', '', 'test', 1, 1)")
            .execute(&state.db).await.unwrap();
        let mut extractions = 0;
        let mut consolidations = 0;
        for id in 1..=20 {
            let fork = state.fork_with_isolated_history().await;
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
        let fork = state.fork_with_isolated_history().await;
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
            .fork_with_isolated_history()
            .await;
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
        assert!(state.history.lock().await.is_empty());
        assert!(state.current_conversation_id.lock().await.is_none());
    }

    #[tokio::test]
    async fn telegram_redelivery_retries_only_the_saved_reply_in_its_original_conversation() {
        let sessions = Sessions::default();
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .fork_with_isolated_history()
            .await;
        let first =
            begin_telegram_turn(&state, "default", 10, 1, " typed prompt ", true, &sessions)
                .await
                .unwrap();
        assert!(first.previous_reply.is_none());
        state.clear_history().await;
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
        assert_eq!(state.history.lock().await.len(), 1);
    }
}
