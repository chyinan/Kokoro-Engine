//! Telegram Bot core — message handling, command dispatch, and AI pipeline bridge.

use super::config::TelegramConfig;
use crate::ai::context::AIOrchestrator;
use crate::imagegen::ImageGenService;
use crate::llm::service::LlmService;
use crate::stt::{AudioSource, SttService};
use crate::tts::TtsService;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use serde::Serialize;
use tauri::{Emitter, Manager};
use teloxide::prelude::*;
use teloxide::types::InputFile;
use tokio::sync::{oneshot, RwLock};

/// Per-chat session state.
#[derive(Clone, Debug)]
enum SessionMode {
    /// Continue the desktop conversation (default).
    Continue,
    /// Fresh conversation started via /new.
    New,
}

type Sessions = Arc<RwLock<HashMap<ChatId, SessionMode>>>;

/// Event payload for syncing Telegram messages to the desktop chat UI.
#[derive(Clone, Serialize)]
struct TelegramChatSync {
    role: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
}

/// Run the long-polling loop. Blocks until `shutdown_rx` fires or an error occurs.
pub async fn run_polling(
    token: String,
    config: TelegramConfig,
    app: tauri::AppHandle,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let bot = Bot::new(&token);
    let config = Arc::new(config);
    let sessions: Sessions = Arc::new(RwLock::new(HashMap::new()));

    // Build the update handler
    let handler = Update::filter_message().endpoint(handle_message);

    let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
        .dependencies(dptree::deps![config.clone(), sessions.clone(), app.clone()])
        .default_handler(|_upd| async {})
        .build();

    // Run dispatcher in a spawned task so we can select on shutdown
    let shutdown_token = dispatcher.shutdown_token();
    tauri::async_runtime::spawn(async move {
        dispatcher.dispatch().await;
    });

    // Wait for shutdown signal
    let _ = shutdown_rx.await;
    shutdown_token
        .shutdown()
        .expect("Failed to shutdown dispatcher")
        .await;
}

/// Central message handler — dispatches commands and regular messages.
async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<TelegramConfig>,
    sessions: Sessions,
    app: tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;
    println!("[Telegram] Received message from chat_id={}, text={:?}", chat_id.0, msg.text());

    // Access control: check whitelist
    if !config.allowed_chat_ids.is_empty()
        && !config.allowed_chat_ids.contains(&chat_id.0)
    {
        println!("[Telegram] Chat {} not in whitelist, ignoring", chat_id.0);
        return Ok(());
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
        return handle_photo(&bot, &msg, &config, &app).await;
    }

    // Regular text message
    if let Some(text) = msg.text() {
        return handle_text(&bot, &msg, text, &config, &app).await;
    }

    Ok(())
}

/// Handle bot commands: /start, /new, /continue, /status
async fn handle_command(
    bot: &Bot,
    msg: &Message,
    text: &str,
    _config: &Arc<TelegramConfig>,
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
                eprintln!("[Telegram] Failed to send /start reply: {}", e);
            }
        }
        "/new" => {
            // Clear orchestrator history to start fresh
            if let Some(orchestrator) = app.try_state::<AIOrchestrator>() {
                let mut history = orchestrator.history.lock().await;
                history.clear();
                drop(history);
                let mut conv_id = orchestrator.current_conversation_id.lock().await;
                *conv_id = None;
            }
            {
                let mut s = sessions.write().await;
                s.insert(chat_id, SessionMode::New);
            }
            bot.send_message(chat_id, "✨ New conversation started.")
                .await
                .ok();
        }
        "/continue" => {
            {
                let mut s = sessions.write().await;
                s.insert(chat_id, SessionMode::Continue);
            }
            bot.send_message(chat_id, "🔗 Continuing desktop conversation.")
                .await
                .ok();
        }
        "/status" => {
            let mode = {
                let s = sessions.read().await;
                s.get(&chat_id).cloned().unwrap_or(SessionMode::Continue)
            };
            let mode_str = match mode {
                SessionMode::Continue => "Continue (desktop)",
                SessionMode::New => "New conversation",
            };
            let history_len = if let Some(orchestrator) = app.try_state::<AIOrchestrator>() {
                orchestrator.history.lock().await.len()
            } else {
                0
            };
            bot.send_message(
                chat_id,
                format!(
                    "📊 Session: {}\n💬 History: {} messages",
                    mode_str, history_len
                ),
            )
            .await
            .ok();
        }
        _ => {
            // Unknown command — treat as text
            let clean = text.trim_start_matches('/');
            if !clean.is_empty() {
                handle_text(bot, msg, text, _config, app).await?;
            }
        }
    }

    Ok(())
}

/// Handle a plain text message — run through the LLM pipeline and reply.
async fn handle_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &Arc<TelegramConfig>,
    app: &tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;

    let orchestrator = app
        .try_state::<AIOrchestrator>()
        .ok_or("AIOrchestrator not available")?;
    let llm_service = app
        .try_state::<LlmService>()
        .ok_or("LlmService not available")?;

    // 1. Record user message
    orchestrator
        .add_message("user".to_string(), text.to_string())
        .await;

    // Sync user message to desktop UI
    let _ = app.emit("telegram:chat-sync", TelegramChatSync {
        role: "user".to_string(),
        text: text.to_string(),
        translation: None,
    });

    // 2. Compose prompt context
    let prompt_messages = orchestrator
        .compose_prompt(text, false, None)
        .await
        .map_err(|e| e.to_string())?;

    let mut client_messages: Vec<crate::llm::openai::Message> = prompt_messages
        .into_iter()
        .map(|m| crate::llm::openai::Message {
            role: m.role,
            content: crate::llm::openai::MessageContent::Text(m.content),
        })
        .collect();

    // Ensure the latest user turn is in the messages
    let already_has_user = client_messages
        .last()
        .map(|m| m.role == "user")
        .unwrap_or(false);
    if !already_has_user {
        client_messages.push(crate::llm::openai::Message {
            role: "user".to_string(),
            content: crate::llm::openai::MessageContent::Text(text.to_string()),
        });
    }

    // 3. Stream LLM response (collect fully)
    let provider = llm_service.provider().await;
    let mut stream = provider
        .chat_stream(client_messages, None)
        .await
        .map_err(|e| format!("LLM stream error: {}", e))?;

    let mut response = String::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(delta) => response.push_str(&delta),
            Err(e) => {
                eprintln!("[Telegram] LLM stream error: {}", e);
                break;
            }
        }
    }

    if response.is_empty() {
        bot.send_message(chat_id, "(No response from AI)")
            .await
            .ok();
        return Ok(());
    }

    // 4. Parse tool calls and translations, clean response
    let (cleaned, _tool_calls) = parse_tool_call_tags(&response);
    let (cleaned, translation) = extract_translate_tags(&cleaned);
    let cleaned = strip_leaked_tags(&cleaned);
    // Strip remaining control tags that shouldn't appear in Telegram
    let cleaned = strip_control_tags(&cleaned);

    // 5. Persist assistant message
    let metadata = translation
        .as_ref()
        .map(|t| serde_json::json!({ "translation": t }).to_string());
    orchestrator
        .add_message_with_metadata("assistant".to_string(), cleaned.clone(), metadata)
        .await;

    // Sync assistant message to desktop UI
    let _ = app.emit("telegram:chat-sync", TelegramChatSync {
        role: "assistant".to_string(),
        text: cleaned.clone(),
        translation: translation.clone(),
    });

    // 6. Build reply text (include translation if present)
    let reply_text = if let Some(ref t) = translation {
        format!("{}\n\n📝 {}", cleaned, t)
    } else {
        cleaned.clone()
    };

    // 7. Send text reply
    bot.send_message(chat_id, &reply_text).await.ok();

    // 8. Optionally send voice reply
    if config.send_voice_reply {
        send_voice_reply(bot, chat_id, &cleaned, app).await;
    }

    // 9. Handle image generation tags
    handle_image_tags(bot, chat_id, &response, app).await;

    Ok(())
}

/// Handle voice messages — download, transcribe via STT, then process as text.
async fn handle_voice(
    bot: &Bot,
    msg: &Message,
    config: &Arc<TelegramConfig>,
    _sessions: &Sessions,
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
            eprintln!("[Telegram] STT error: {}", e);
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
    bot.send_message(chat_id, format!("🎤 {}", transcription))
        .await
        .ok();

    // Process as regular text
    handle_text(bot, msg, &transcription, config, app).await
}

/// Handle photo messages — download image, convert to base64, send to LLM with vision.
async fn handle_photo(
    bot: &Bot,
    msg: &Message,
    config: &Arc<TelegramConfig>,
    app: &tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id;
    let photos = msg.photo().ok_or("No photo data")?;

    // Telegram sends multiple sizes — pick the largest one
    let photo = photos.last().ok_or("Empty photo array")?;

    let orchestrator = app
        .try_state::<AIOrchestrator>()
        .ok_or("AIOrchestrator not available")?;
    let llm_service = app
        .try_state::<LlmService>()
        .ok_or("LlmService not available")?;

    // Download photo file
    let file = bot.get_file(&photo.file.id).await?;
    let mut buf = Vec::new();
    teloxide::net::Download::download_file(bot, &file.path, &mut buf).await?;

    // Convert to base64 data URL
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    // Detect mime from file extension or default to jpeg
    let mime = if file.path.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let data_url = format!("data:{};base64,{}", mime, b64);

    // Use caption as the user message, or a default prompt
    let caption = msg
        .caption()
        .unwrap_or("The user sent you a photo:")
        .to_string();

    println!("[Telegram] Photo received, caption: {}", caption);

    // 1. Record user message
    orchestrator
        .add_message("user".to_string(), caption.clone())
        .await;

    // Sync user message to desktop UI
    let _ = app.emit("telegram:chat-sync", TelegramChatSync {
        role: "user".to_string(),
        text: format!("[TG] 📷 {}", caption),
        translation: None,
    });

    // 2. Compose prompt context
    let prompt_messages = orchestrator
        .compose_prompt(&caption, false, None)
        .await
        .map_err(|e| e.to_string())?;

    let mut client_messages: Vec<crate::llm::openai::Message> = prompt_messages
        .into_iter()
        .map(|m| crate::llm::openai::Message {
            role: m.role,
            content: crate::llm::openai::MessageContent::Text(m.content),
        })
        .collect();

    // Replace or append the last user message with multimodal content (text + image)
    let already_has_user = client_messages
        .last()
        .map(|m| m.role == "user")
        .unwrap_or(false);
    if already_has_user {
        // Replace last user message with multimodal version
        let last = client_messages.last_mut().unwrap();
        last.content = crate::llm::openai::MessageContent::with_images(
            caption.clone(),
            vec![data_url],
        );
    } else {
        client_messages.push(crate::llm::openai::Message {
            role: "user".to_string(),
            content: crate::llm::openai::MessageContent::with_images(
                caption.clone(),
                vec![data_url],
            ),
        });
    }

    // 3. Stream LLM response
    let provider = llm_service.provider().await;
    let mut stream = provider
        .chat_stream(client_messages, None)
        .await
        .map_err(|e| format!("LLM stream error: {}", e))?;

    let mut response = String::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(delta) => response.push_str(&delta),
            Err(e) => {
                eprintln!("[Telegram] LLM stream error: {}", e);
                break;
            }
        }
    }

    if response.is_empty() {
        bot.send_message(chat_id, "(No response from AI)")
            .await
            .ok();
        return Ok(());
    }

    // 4. Clean response
    let (cleaned, _tool_calls) = parse_tool_call_tags(&response);
    let (cleaned, translation) = extract_translate_tags(&cleaned);
    let cleaned = strip_leaked_tags(&cleaned);
    let cleaned = strip_control_tags(&cleaned);

    // 5. Persist
    let metadata = translation
        .as_ref()
        .map(|t| serde_json::json!({ "translation": t }).to_string());
    orchestrator
        .add_message_with_metadata("assistant".to_string(), cleaned.clone(), metadata)
        .await;

    // Sync to desktop
    let _ = app.emit("telegram:chat-sync", TelegramChatSync {
        role: "assistant".to_string(),
        text: cleaned.clone(),
        translation: translation.clone(),
    });

    // 6. Reply
    let reply_text = if let Some(ref t) = translation {
        format!("{}\n\n📝 {}", cleaned, t)
    } else {
        cleaned.clone()
    };
    bot.send_message(chat_id, &reply_text).await.ok();

    // 7. Voice reply
    if config.send_voice_reply {
        send_voice_reply(bot, chat_id, &cleaned, app).await;
    }

    Ok(())
}

/// Synthesize text via TTS and send as a Telegram voice message.
async fn send_voice_reply(bot: &Bot, chat_id: ChatId, text: &str, app: &tauri::AppHandle) {
    let tts_service = match app.try_state::<TtsService>() {
        Some(s) => s,
        None => return,
    };

    match tts_service.synthesize_text(text, None).await {
        Ok(audio_bytes) if !audio_bytes.is_empty() => {
            let input = InputFile::memory(audio_bytes).file_name("reply.ogg");
            if let Err(e) = bot.send_voice(chat_id, input).await {
                eprintln!("[Telegram] Failed to send voice: {}", e);
            }
        }
        Ok(_) => {} // Empty audio, skip
        Err(e) => {
            eprintln!("[Telegram] TTS synthesis error: {}", e);
        }
    }
}

/// Check for `[IMAGE_PROMPT:...]` tags in the response and generate/send images.
async fn handle_image_tags(bot: &Bot, chat_id: ChatId, response: &str, app: &tauri::AppHandle) {
    let prefix = "[IMAGE_PROMPT:";
    let mut search = response.as_bytes();
    let response_bytes = response.as_bytes();

    loop {
        let offset = response_bytes.len() - search.len();
        let haystack = &response[offset..];
        let start = match haystack.find(prefix) {
            Some(s) => s,
            None => break,
        };
        let rest = &haystack[start + prefix.len()..];
        let end = match rest.find(']') {
            Some(e) => e,
            None => break,
        };
        let prompt = rest[..end].trim();
        // Advance search past this tag
        search = &response_bytes[offset + start + prefix.len() + end + 1..];

        if prompt.is_empty() {
            continue;
        }

        let imagegen = match app.try_state::<ImageGenService>() {
            Some(s) => s,
            None => break,
        };

        match imagegen.generate(prompt.to_string(), None, None).await {
            Ok(result) => {
                // result.image_url is a local file path
                match tokio::fs::read(&result.image_url).await {
                    Ok(data) => {
                        let input = InputFile::memory(data).file_name("image.png");
                        if let Err(e) = bot.send_photo(chat_id, input).await {
                            eprintln!("[Telegram] Failed to send photo: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("[Telegram] Failed to read generated image: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[Telegram] Image generation failed: {}", e);
                bot.send_message(chat_id, format!("⚠️ Image generation failed: {}", e))
                    .await
                    .ok();
            }
        }
    }
}

// ── Tag parsing helpers (mirrored from commands/chat.rs) ──────────

const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";
const TRANSLATE_TAG_PREFIX: &str = "[TRANSLATE:";

fn parse_tool_call_tags(text: &str) -> (String, Vec<String>) {
    let mut result = text.to_string();
    let mut calls = Vec::new();

    while let Some(start) = result.rfind(TOOL_CALL_TAG_PREFIX) {
        let rest = &result[start..];
        if let Some(end_bracket) = rest.find(']') {
            let inner = &rest[TOOL_CALL_TAG_PREFIX.len()..end_bracket];
            calls.push(inner.to_string());
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
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
    calls.reverse();
    (result.trim().to_string(), calls)
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

/// Strip control tags that shouldn't appear in Telegram messages:
/// [ACTION:xxx], [EMOTION:xxx], [IMAGE_PROMPT:xxx] (image handled separately)
fn strip_control_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Remove [ACTION:...] tags
    while let Some(start) = result.find("[ACTION:") {
        if let Some(end) = result[start..].find(']') {
            let tag_end = start + end + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            break;
        }
    }
    // Remove [EMOTION:...] tags
    while let Some(start) = result.find("[EMOTION:") {
        if let Some(end) = result[start..].find(']') {
            let tag_end = start + end + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            break;
        }
    }
    result.trim().to_string()
}
