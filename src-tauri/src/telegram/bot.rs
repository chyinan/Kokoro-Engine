//! Telegram Bot core — message handling, command dispatch, and AI pipeline bridge.

use super::config::TelegramConfig;
use crate::actions::tool_settings::ToolSettings;
use crate::actions::{execute_tool_calls, ToolInvocation};
use crate::ai::context::AIOrchestrator;
use crate::ai::memory_extractor;
use crate::imagegen::ImageGenService;
use crate::llm::messages::{
    assistant_text_message, is_user_message, replace_user_message_with_images, role_text_message,
    system_message, user_message_with_images, user_text_message,
};
use crate::llm::service::LlmService;
use crate::stt::{AudioSource, SttService};
use crate::tts::TtsService;
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
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

/// Per-chat session state.
#[derive(Clone, Debug)]
enum SessionMode {
    /// Continue the desktop conversation (default).
    Continue,
    /// Fresh conversation started via /new.
    New,
}

type Sessions = Arc<RwLock<HashMap<ChatId, SessionMode>>>;

async fn max_tool_rounds(app: &tauri::AppHandle) -> usize {
    if let Some(settings) = app.try_state::<Arc<RwLock<ToolSettings>>>() {
        settings.read().await.max_tool_rounds.max(1)
    } else {
        10
    }
}

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
    if !config.allowed_chat_ids.is_empty() && !config.allowed_chat_ids.contains(&chat_id.0) {
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
                tracing::error!(target: "telegram", "[Telegram] Failed to send /start reply: {}", e);
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
    // char_id 解析优先级：config 指定 > orchestrator 内存状态 > 磁盘文件 > "default"
    let char_id = match config.character_id.as_deref().filter(|s| !s.is_empty()) {
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
    tracing::info!(target: "telegram", "[Telegram] Resolved char_id='{}' for this request", char_id);
    orchestrator
        .add_message("user".to_string(), text.to_string(), &char_id)
        .await;

    // Sync user message to desktop UI
    let _ = app.emit(
        "telegram:chat-sync",
        TelegramChatSync {
            role: "user".to_string(),
            text: text.to_string(),
            translation: None,
        },
    );

    // 2. Compose prompt context (with tool prompt)
    let action_registry = app
        .try_state::<Arc<RwLock<crate::actions::ActionRegistry>>>()
        .ok_or("ActionRegistry not available")?;
    let tool_settings = app
        .try_state::<Arc<RwLock<ToolSettings>>>()
        .ok_or("ToolSettings not available")?;
    let tool_prompt = {
        let registry = action_registry.read().await;
        let settings = tool_settings.read().await;
        let p = registry.generate_tool_prompt_for_prompt_with_settings(
            orchestrator.is_memory_enabled(),
            &settings,
        );
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    };

    let prompt_messages = orchestrator
        .compose_prompt(text, false, tool_prompt, false, &char_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut client_messages = prompt_messages
        .into_iter()
        .map(|m| role_text_message(&m.role, m.content))
        .collect::<Result<Vec<_>, _>>()?;

    // Ensure the latest user turn is in the messages
    let already_has_user = client_messages.last().map(is_user_message).unwrap_or(false);
    if !already_has_user {
        client_messages.push(user_text_message(text.to_string()));
    }

    // 3. LLM call with tool execution loop
    let provider = llm_service.provider().await;
    let max_rounds = max_tool_rounds(app).await;
    let mut all_cleaned_text = String::new();
    let mut all_translations: Vec<String> = Vec::new();

    for _round in 0..max_rounds {
        let mut stream = provider
            .chat_stream(client_messages.clone(), None)
            .await
            .map_err(|e| format!("LLM stream error: {}", e))?;

        let mut response = String::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => response.push_str(&delta),
                Err(e) => {
                    tracing::error!(target: "telegram", "[Telegram] LLM stream error: {}", e);
                    break;
                }
            }
        }

        if response.is_empty() {
            break;
        }

        let (cleaned, tool_calls) = parse_tool_call_tags(&response);
        let (cleaned, round_translation) = extract_translate_tags(&cleaned);
        let cleaned = strip_leaked_tags(&cleaned);

        tracing::info!(
            target: "telegram",
            "[Telegram] Tool loop round: {} tool_calls found, char_id='{}'",
            tool_calls.len(),
            char_id
        );

        if let Some(t) = round_translation {
            all_translations.push(t);
        }
        if !cleaned.is_empty() {
            if !all_cleaned_text.is_empty() {
                all_cleaned_text.push(' ');
            }
            all_cleaned_text.push_str(&cleaned);
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
        let tool_results: Vec<String> = execution_outcomes
            .iter()
            .map(|outcome| {
                tracing::info!(
                    target: "telegram::tools",
                    "[Telegram/ToolCall] Executing: {} with args {:?}",
                    outcome.invocation.name, outcome.invocation.args
                );
                match &outcome.result {
                    Ok(result) => {
                        tracing::info!(target: "telegram::tools", "[Telegram/ToolCall] {} => {}", outcome.tool_name(), result.message);
                    }
                    Err(error) => {
                        tracing::error!(target: "telegram::tools", "[Telegram/ToolCall] {} failed: {}", outcome.tool_name(), error);
                    }
                }
                outcome.result_line()
            })
            .collect();
        let any_needs_feedback = execution_outcomes
            .iter()
            .any(|outcome| outcome.needs_feedback);

        if !any_needs_feedback {
            break;
        }

        client_messages.push(assistant_text_message(response));
        client_messages.push(system_message(format!(
            "[Tool results]\n{}\nContinue your response naturally.",
            tool_results.join("\n")
        )));
    }

    let response = strip_control_tags(&compact_newlines(&all_cleaned_text));
    let translation = if all_translations.is_empty() {
        None
    } else {
        Some(compact_newlines(&all_translations.join(" ")))
    };

    if response.is_empty() {
        bot.send_message(chat_id, "(No response from AI)")
            .await
            .ok();
        return Ok(());
    }

    // 5. Persist assistant message
    let metadata = translation
        .as_ref()
        .map(|t| serde_json::json!({ "translation": t }).to_string());
    orchestrator
        .add_message_with_metadata(
            "assistant".to_string(),
            response.clone(),
            metadata,
            &char_id,
            None,
        )
        .await;

    // Trigger periodic memory extraction (every 5 user messages)
    let msg_count = orchestrator.get_message_count().await;
    let memory_msg_count = orchestrator.get_memory_trigger_count().await;
    tracing::info!(
        target: "telegram::memory",
        "[Telegram/Memory] User message count: {}, memory trigger count: {}, char_id: {}",
        msg_count, memory_msg_count, char_id
    );
    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 5 == 0 {
        tracing::info!(
            target: "telegram::memory",
            "[Telegram/Memory] Triggering memory extraction (count={})",
            msg_count
        );
        let history = orchestrator.get_recent_memory_history(10).await;
        let memory_mgr = orchestrator.memory_manager.clone();
        let provider_for_mem = llm_service.provider().await;
        let char_id_for_mem = char_id.clone();
        let memory_enabled = orchestrator.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            memory_extractor::extract_and_store_memories(
                &history,
                &memory_mgr,
                provider_for_mem,
                char_id_for_mem,
            )
            .await;
        });
    }
    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 20 == 0 {
        let memory_mgr = orchestrator.memory_manager.clone();
        let char_id_for_consolidation = char_id.clone();
        let provider_for_consolidation = llm_service.provider().await;
        let memory_enabled = orchestrator.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            match memory_mgr
                .consolidate_memories(&char_id_for_consolidation, provider_for_consolidation)
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(target: "telegram::memory", "[Telegram/Memory] Consolidated {} memory clusters", count);
                }
                Err(e) => {
                    tracing::error!(target: "telegram::memory", "[Telegram/Memory] Consolidation failed: {}", e);
                }
                _ => {}
            }
        });
    }

    // Sync assistant message to desktop UI
    let _ = app.emit(
        "telegram:chat-sync",
        TelegramChatSync {
            role: "assistant".to_string(),
            text: response.clone(),
            translation: translation.clone(),
        },
    );

    // 6. Build reply text (include translation if present)
    let reply_text = if let Some(ref t) = translation {
        format!("{}\n\n📝 {}", response, t)
    } else {
        response.clone()
    };

    // 7. Send text reply
    bot.send_message(chat_id, &reply_text).await.ok();

    // 8. Optionally send voice reply
    if config.send_voice_reply {
        send_voice_reply(bot, chat_id, &response, app).await;
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

    tracing::info!(target: "telegram", "[Telegram] Photo received, caption: {}", caption);

    // 1. Record user message
    // char_id 解析优先级：config 指定 > orchestrator 内存状态 > 磁盘文件 > "default"
    let char_id = match config.character_id.as_deref().filter(|s| !s.is_empty()) {
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
    tracing::info!(
        target: "telegram",
        "[Telegram] Resolved char_id='{}' for photo request",
        char_id
    );
    orchestrator
        .add_message("user".to_string(), caption.clone(), &char_id)
        .await;

    // Sync user message to desktop UI
    let _ = app.emit(
        "telegram:chat-sync",
        TelegramChatSync {
            role: "user".to_string(),
            text: format!("[TG] 📷 {}", caption),
            translation: None,
        },
    );

    // 2. Compose prompt context (with tool prompt)
    let action_registry = app
        .try_state::<Arc<RwLock<crate::actions::ActionRegistry>>>()
        .ok_or("ActionRegistry not available")?;
    let tool_settings = app
        .try_state::<Arc<RwLock<ToolSettings>>>()
        .ok_or("ToolSettings not available")?;
    let tool_prompt = {
        let registry = action_registry.read().await;
        let settings = tool_settings.read().await;
        let p = registry.generate_tool_prompt_for_prompt_with_settings(
            orchestrator.is_memory_enabled(),
            &settings,
        );
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    };

    let prompt_messages = orchestrator
        .compose_prompt(&caption, false, tool_prompt, false, &char_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut client_messages = prompt_messages
        .into_iter()
        .map(|m| role_text_message(&m.role, m.content))
        .collect::<Result<Vec<_>, _>>()?;

    // Replace or append the last user message with multimodal content (text + image)
    let already_has_user = client_messages.last().map(is_user_message).unwrap_or(false);
    if already_has_user {
        let last = client_messages.last_mut().unwrap();
        replace_user_message_with_images(last, caption.clone(), vec![data_url])?;
    } else {
        client_messages.push(user_message_with_images(caption.clone(), vec![data_url]));
    }

    // 3. LLM call with tool execution loop
    let provider = llm_service.provider().await;
    let max_rounds = max_tool_rounds(app).await;
    let mut all_cleaned_text = String::new();
    let mut all_translations: Vec<String> = Vec::new();

    for _round in 0..max_rounds {
        let mut stream = provider
            .chat_stream(client_messages.clone(), None)
            .await
            .map_err(|e| format!("LLM stream error: {}", e))?;

        let mut round_response = String::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => round_response.push_str(&delta),
                Err(e) => {
                    tracing::error!(target: "telegram", "[Telegram] LLM stream error: {}", e);
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
            target: "telegram",
            "[Telegram/Photo] Tool loop round: {} tool_calls found, char_id='{}'",
            tool_calls.len(),
            char_id
        );

        if let Some(t) = round_translation {
            all_translations.push(t);
        }
        if !cleaned.is_empty() {
            if !all_cleaned_text.is_empty() {
                all_cleaned_text.push(' ');
            }
            all_cleaned_text.push_str(&cleaned);
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
        let tool_results: Vec<String> = execution_outcomes
            .iter()
            .map(|outcome| {
                tracing::info!(
                    target: "telegram::tools",
                    "[Telegram/ToolCall] Executing: {} with args {:?}",
                    outcome.invocation.name, outcome.invocation.args
                );
                match &outcome.result {
                    Ok(result) => {
                        tracing::info!(target: "telegram::tools", "[Telegram/ToolCall] {} => {}", outcome.tool_name(), result.message);
                    }
                    Err(error) => {
                        tracing::error!(target: "telegram::tools", "[Telegram/ToolCall] {} failed: {}", outcome.tool_name(), error);
                    }
                }
                outcome.result_line()
            })
            .collect();
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

    let response = strip_control_tags(&compact_newlines(&all_cleaned_text));
    let translation = if all_translations.is_empty() {
        None
    } else {
        Some(compact_newlines(&all_translations.join(" ")))
    };

    if response.is_empty() {
        bot.send_message(chat_id, "(No response from AI)")
            .await
            .ok();
        return Ok(());
    }

    // 5. Persist
    let metadata = translation
        .as_ref()
        .map(|t| serde_json::json!({ "translation": t }).to_string());
    orchestrator
        .add_message_with_metadata(
            "assistant".to_string(),
            response.clone(),
            metadata,
            &char_id,
            None,
        )
        .await;

    // Trigger periodic memory extraction (every 5 user messages)
    let msg_count = orchestrator.get_message_count().await;
    let memory_msg_count = orchestrator.get_memory_trigger_count().await;
    tracing::info!(
        target: "telegram::memory",
        "[Telegram/Memory] User message count: {}, memory trigger count: {}, char_id: {}",
        msg_count, memory_msg_count, char_id
    );
    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 5 == 0 {
        tracing::info!(
            target: "telegram::memory",
            "[Telegram/Memory] Triggering memory extraction (count={})",
            msg_count
        );
        let history = orchestrator.get_recent_memory_history(10).await;
        let memory_mgr = orchestrator.memory_manager.clone();
        let provider_for_mem = llm_service.provider().await;
        let char_id_for_mem = char_id.clone();
        let memory_enabled = orchestrator.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            memory_extractor::extract_and_store_memories(
                &history,
                &memory_mgr,
                provider_for_mem,
                char_id_for_mem,
            )
            .await;
        });
    }
    if orchestrator.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 20 == 0 {
        let memory_mgr = orchestrator.memory_manager.clone();
        let char_id_for_consolidation = char_id.clone();
        let provider_for_consolidation = llm_service.provider().await;
        let memory_enabled = orchestrator.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            match memory_mgr
                .consolidate_memories(&char_id_for_consolidation, provider_for_consolidation)
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(target: "telegram::memory", "[Telegram/Memory] Consolidated {} memory clusters", count);
                }
                Err(e) => {
                    tracing::error!(target: "telegram::memory", "[Telegram/Memory] Consolidation failed: {}", e);
                }
                _ => {}
            }
        });
    }

    // Sync to desktop
    let _ = app.emit(
        "telegram:chat-sync",
        TelegramChatSync {
            role: "assistant".to_string(),
            text: response.clone(),
            translation: translation.clone(),
        },
    );

    // 6. Reply
    let reply_text = if let Some(ref t) = translation {
        format!("{}\n\n📝 {}", response, t)
    } else {
        response.clone()
    };
    bot.send_message(chat_id, &reply_text).await.ok();

    // 7. Voice reply
    if config.send_voice_reply {
        send_voice_reply(bot, chat_id, &response, app).await;
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
                tracing::error!(target: "telegram", "[Telegram] Failed to send voice: {}", e);
            }
        }
        Ok(_) => {} // Empty audio, skip
        Err(e) => {
            tracing::error!(target: "telegram", "[Telegram] TTS synthesis error: {}", e);
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

        match imagegen
            .generate(prompt.to_string(), None, None, None)
            .await
        {
            Ok(result) => {
                // result.image_url is a local file path
                match tokio::fs::read(&result.image_url).await {
                    Ok(data) => {
                        let input = InputFile::memory(data).file_name("image.png");
                        if let Err(e) = bot.send_photo(chat_id, input).await {
                            tracing::error!(target: "telegram", "[Telegram] Failed to send photo: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!(target: "telegram", "[Telegram] Failed to read generated image: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!(target: "telegram", "[Telegram] Image generation failed: {}", e);
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
    let mut calls = Vec::new();

    // Parse [TOOL_CALL:name|key=val|...] format
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
                calls.push(ToolCall { name, args });
            }
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

    // Also parse simplified [action_name|key=val] format
    let mut extra_calls = Vec::new();
    let mut cleaned = result.clone();
    let mut offset = 0;
    while offset < cleaned.len() {
        let Some(rel_start) = cleaned[offset..].find('[') else {
            break;
        };
        let start = offset + rel_start;
        let rest = &cleaned[start..];
        let Some(end) = rest.find(']') else { break };
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
                extra_calls.push(ToolCall { name, args });
                let tag_end = start + end + 1;
                cleaned = format!(
                    "{}{}",
                    cleaned[..start].trim_end(),
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
    calls.extend(extra_calls);
    calls.reverse();
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

/// Collapse excessive newlines for cleaner Telegram output.
/// Removes lines that contain only ellipsis/whitespace, then collapses 2+ newlines to 1.
fn compact_newlines(text: &str) -> String {
    // Filter out lines that are only whitespace or lone ellipsis fragments
    let filtered: Vec<&str> = text
        .lines()
        .filter(|line| {
            let t = line.trim().trim_matches('…').trim_matches('.').trim();
            !t.is_empty()
        })
        .collect();
    filtered.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── compact_newlines ──────────────────────────────────

    #[test]
    fn test_compact_newlines_removes_ellipsis_only_lines() {
        let input = "Hello\n…\nWorld";
        assert_eq!(compact_newlines(input), "Hello\nWorld");
    }

    #[test]
    fn test_compact_newlines_removes_dot_only_lines() {
        let input = "Hello\n...\nWorld";
        assert_eq!(compact_newlines(input), "Hello\nWorld");
    }

    #[test]
    fn test_compact_newlines_no_change_needed() {
        let input = "Hello\nWorld";
        assert_eq!(compact_newlines(input), "Hello\nWorld");
    }

    #[test]
    fn test_compact_newlines_empty_string() {
        assert_eq!(compact_newlines(""), "");
    }

    // ── strip_control_tags ────────────────────────────────

    #[test]
    fn test_strip_control_tags_removes_action_tag() {
        // trim_end/trim_start collapses surrounding spaces
        let input = "Hello [ACTION:wave] world";
        assert_eq!(strip_control_tags(input), "Helloworld");
    }

    #[test]
    fn test_strip_control_tags_removes_emotion_tag() {
        let input = "[EMOTION:happy] Nice to meet you";
        assert_eq!(strip_control_tags(input), "Nice to meet you");
    }

    #[test]
    fn test_strip_control_tags_no_tags_unchanged() {
        let input = "Just a normal message";
        assert_eq!(strip_control_tags(input), "Just a normal message");
    }

    #[test]
    fn test_strip_control_tags_multiple_tags() {
        let input = "[ACTION:nod] Hello [EMOTION:curious] there";
        let result = strip_control_tags(input);
        assert!(!result.contains("[ACTION:"));
        assert!(!result.contains("[EMOTION:"));
        assert!(result.contains("Hello"));
        assert!(result.contains("there"));
    }

    // ── extract_translate_tags ────────────────────────────

    #[test]
    fn test_extract_translate_tags_basic() {
        let input = "こんにちは [TRANSLATE:Hello]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, Some("Hello".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_none() {
        let input = "Hello world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "Hello world");
        assert!(translation.is_none());
    }

    // ── parse_tool_call_tags ──────────────────────────────

    #[test]
    fn test_parse_tool_call_tags_no_tags() {
        let input = "Just a response";
        let (text, calls) = parse_tool_call_tags(input);
        assert_eq!(text, "Just a response");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_tags_with_params() {
        let input = "Sure! [TOOL_CALL:get_time|tz=UTC]";
        let (text, calls) = parse_tool_call_tags(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_time");
        assert_eq!(calls[0].args.get("tz"), Some(&"UTC".to_string()));
        assert!(!text.contains("[TOOL_CALL:"));
    }

    #[test]
    fn test_telegram_error_log_prefix_format() {
        let rendered = format!(
            "[ERROR][Telegram] Dispatcher task failed: {}. 常见原因是网络不可达或未开启代理。",
            "Network timeout"
        );
        assert!(rendered.starts_with("[ERROR][Telegram]"));
        assert!(rendered.contains("Network timeout"));
    }

}
