//! Telegram Bot core — message handling, command dispatch, and AI pipeline bridge.
// pattern: Mixed (unavoidable)
// Reason: Telegram 机器人处理网络 I/O、消息路由、持久化和异步任务调度，属于编排层。

use super::config::TelegramConfig;
use crate::ai::context::AIOrchestrator;
use crate::chat::application::{channel_id, submit_chat, ChatPolicy, ChatRequest};
use crate::stt::{AudioSource, SttService};
use crate::tts::TtsService;
use std::collections::HashMap;
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

type Sessions = Arc<RwLock<HashMap<ChatId, SessionMode>>>;
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
    let sessions: Sessions = Arc::new(RwLock::new(HashMap::new()));
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
            sessions.write().await.insert(
                chat_id,
                SessionMode::Fresh {
                    id: uuid::Uuid::new_v4().to_string(),
                },
            );
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
            sessions.write().await.insert(
                chat_id,
                SessionMode::Desktop {
                    character_id,
                    conversation_id,
                },
            );
            bot.send_message(chat_id, "🔗 Continuing the selected desktop conversation.")
                .await?;
        }
        "/status" => {
            let mode = sessions
                .read()
                .await
                .get(&chat_id)
                .cloned()
                .unwrap_or_default();
            let state = app.state::<AIOrchestrator>();
            let active_character = state.get_character_id().await;
            let character = config
                .character_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(&active_character);
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

async fn reply_to_message(
    bot: &Bot,
    msg: &Message,
    text: &str,
    images: Vec<String>,
    config: &TelegramConfig,
    sessions: &Sessions,
    app: &tauri::AppHandle,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let state = app.state::<AIOrchestrator>();
    let active_character = state.get_character_id().await;
    let preferred = config
        .character_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&active_character);
    let mode = sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .unwrap_or_default();
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
    let result = submit_message(app, chat_id, msg.id.0, text, images, &mode, preferred).await;

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
    if let Some(translation) = result.translation {
        text.push_str(&format!("\n\n📝 {translation}"));
    }
    for warning in result.output_warnings {
        text.push_str(&format!("\n\n⚠️ {warning}"));
    }
    for chunk in split_message(&text) {
        bot.send_message(chat_id, chunk).await?;
    }
    for image in result.images {
        send_image(bot, chat_id, &image.path).await?;
    }
    if config.send_voice_reply && !result.text.is_empty() {
        send_voice_reply(bot, chat_id, &result.text, app).await?;
    }
    Ok(())
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
    let (character_id, conversation_id, create_conversation) =
        mode.bind(chat_id, preferred_character);
    submit_chat(
        app.clone(),
        ChatRequest {
            message: text.trim().to_owned(),
            character_id: Some(character_id.to_owned()),
            conversation_id: Some(conversation_id),
            // Telegram message identifiers are unique within each chat.
            client_request_id: Some(channel_id(
                "telegram",
                &chat_id.0.to_string(),
                &message_id.to_string(),
            )),
            images: Some(images),
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

async fn send_image(bot: &Bot, chat_id: ChatId, path: &str) -> HandlerResult {
    match tokio::fs::read(path).await {
        Ok(data) => {
            bot.send_photo(chat_id, InputFile::memory(data).file_name("image.png"))
                .await?;
        }
        Err(error) => {
            tracing::warn!(target: "telegram", "Could not read the generated image: {error}");
            bot.send_message(
                chat_id,
                "⚠️ Kokoro Engine could not read the generated image.",
            )
            .await?;
        }
    }
    Ok(())
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;
    let voice = msg.voice().ok_or("No voice data")?;

    let stt_service = match app.try_state::<SttService>() {
        Some(s) => s,
        None => {
            bot.send_message(chat_id, "⚠️ STT service not available.")
                .await
                .ok();
            return Ok(());
        }
    };

    // Download voice file from Telegram
    if voice.file.size > MAX_MEDIA_BYTES {
        bot.send_message(chat_id, "⚠️ Voice message exceeds the 20 MB limit.")
            .await?;
        return Ok(());
    }
    let file = bot.get_file(&voice.file.id).await?;
    let mut buf = Vec::new();
    teloxide::net::Download::download_file(bot, &file.path, &mut buf).await?;

    // Transcribe (Telegram voice messages are OGG/Opus)
    let audio_source = AudioSource::Encoded {
        data: buf,
        format: "ogg".to_string(),
    };
    let transcription = match stt_service.transcribe(&audio_source, None).await {
        Ok(result) => result.text,
        Err(e) => {
            tracing::error!(target: "telegram", "[Telegram] STT error: {}", e);
            bot.send_message(chat_id, "⚠️ Failed to transcribe voice message.")
                .await
                .ok();
            return Ok(());
        }
    };

    if transcription.trim().is_empty() {
        bot.send_message(chat_id, "🔇 (Could not recognize speech)")
            .await
            .ok();
        return Ok(());
    }

    // Send recognized text back as context
    for chunk in split_message(&format!("🎤 {transcription}")) {
        bot.send_message(chat_id, chunk).await?;
    }

    // Process as regular text
    reply_to_message(bot, msg, &transcription, Vec::new(), config, sessions, app).await
}

/// Handle photo messages — download image, convert to base64, send to LLM with vision.
async fn handle_photo(
    bot: &Bot,
    msg: &Message,
    config: &TelegramConfig,
    sessions: &Sessions,
    app: &tauri::AppHandle,
) -> HandlerResult {
    let photo = msg
        .photo()
        .and_then(|photos| photos.last())
        .ok_or("No photo data")?;
    if photo.file.size > MAX_MEDIA_BYTES {
        bot.send_message(msg.chat.id, "⚠️ Photo exceeds the 20 MB limit.")
            .await?;
        return Ok(());
    }
    let file = bot.get_file(&photo.file.id).await?;
    let mut bytes = Vec::new();
    teloxide::net::Download::download_file(bot, &file.path, &mut bytes).await?;
    use base64::Engine as _;
    let mime = if file.path.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    let caption = msg
        .caption()
        .filter(|caption| !caption.trim().is_empty())
        .unwrap_or("The user sent you a photo:");
    reply_to_message(bot, msg, caption, vec![url], config, sessions, app).await
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
}
