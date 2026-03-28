use crate::ai::context::AIOrchestrator;
use crate::ai::context::Message;
use crate::ai::memory_extractor;
use crate::commands::system::WindowSizeState;
use crate::error::KokoroError;
use crate::imagegen::ImageGenService;
use crate::actions::tool_settings::ToolSettings;
use crate::llm::messages::{
    assistant_tool_calls_message, extract_message_text, replace_user_message_with_images,
    history_message_to_chat_message, system_message, tool_result_message,
    user_text_message,
};
use crate::llm::provider::LlmStreamEvent;
use crate::llm::service::LlmService;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{Emitter, Manager, State, Window};
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize)]
pub struct ContextSettings {
    pub strategy: String,
    pub max_message_chars: usize,
}

#[tauri::command]
pub async fn get_context_settings(
    state: State<'_, AIOrchestrator>,
) -> Result<ContextSettings, KokoroError> {
    let (strategy, max_message_chars) = state.get_context_settings().await;
    Ok(ContextSettings { strategy, max_message_chars })
}

#[tauri::command]
pub async fn set_context_settings(
    state: State<'_, AIOrchestrator>,
    settings: ContextSettings,
) -> Result<(), KokoroError> {
    // Validate strategy
    let strategy = if settings.strategy == "summary" {
        "summary".to_string()
    } else {
        "window".to_string()
    };
    // Clamp max_message_chars to safe range
    let max_chars = settings.max_message_chars.clamp(100, 50_000);

    state.set_context_settings(strategy.clone(), max_chars).await;

    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let _ = std::fs::create_dir_all(&app_data);
    let path = app_data.join("context_settings.json");
    let json = serde_json::json!({
        "strategy": strategy,
        "max_message_chars": max_chars,
    });
    if let Err(e) = std::fs::write(&path, json.to_string()) {
        eprintln!("[Context] Failed to persist context_settings: {}", e);
    }

    Ok(())
}

#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub allow_image_gen: Option<bool>,
    pub images: Option<Vec<String>>,
    pub character_id: Option<String>,
    /// If true, neither the user message nor the assistant response is saved to history.
    /// Used for touch interactions and proactive triggers where the instruction shouldn't appear in chat.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Serialize, Clone)]
#[allow(dead_code)]
struct ChatImageGenEvent {
    prompt: String,
}

#[cfg(debug_assertions)]
fn debug_log_llm_messages(label: &str, messages: &[async_openai::types::chat::ChatCompletionRequestMessage]) {
    println!("[LLM/Debug] {} ({} messages)", label, messages.len());
    for (index, message) in messages.iter().enumerate() {
        let role = match message {
            async_openai::types::chat::ChatCompletionRequestMessage::Developer(_) => "developer",
            async_openai::types::chat::ChatCompletionRequestMessage::System(_) => "system",
            async_openai::types::chat::ChatCompletionRequestMessage::User(_) => "user",
            async_openai::types::chat::ChatCompletionRequestMessage::Assistant(_) => "assistant",
            async_openai::types::chat::ChatCompletionRequestMessage::Tool(_) => "tool",
            async_openai::types::chat::ChatCompletionRequestMessage::Function(_) => "function",
        };
        let text = extract_message_text(message);
        let compact = text.replace('\n', "\\n");
        let preview = if compact.chars().count() > 300 {
            format!("{}...", compact.chars().take(300).collect::<String>())
        } else {
            compact
        };
        println!("[LLM/Debug]   #{} role={} text={}", index, role, preview);
    }
}

const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";
const TRANSLATE_TAG_PREFIX: &str = "[TRANSLATE:";

/// Tag prefixes that should be buffered (not emitted to frontend mid-stream).
const BUFFERED_TAG_PREFIXES: &[&str] = &[TOOL_CALL_TAG_PREFIX, TRANSLATE_TAG_PREFIX];

/// Returns the byte position up to which it's safe to emit text to the frontend.
/// Holds back any suffix that could be the start of a known tag prefix.
fn find_safe_emit_boundary(text: &str) -> usize {
    if let Some(last_bracket) = text.rfind('[') {
        let suffix = &text[last_bracket..];
        for prefix in BUFFERED_TAG_PREFIXES {
            if suffix.len() < prefix.len() {
                // Partial match — could still become a full tag
                if prefix.starts_with(suffix) {
                    return last_bracket;
                }
            } else if suffix.starts_with(prefix) {
                // Full prefix match — definitely a tag, hold it
                return last_bracket;
            }
        }
    }
    text.len()
}

/// Strip any `<tool_result>...</tool_result>` blocks or stray tags that the LLM may echo back.
fn strip_leaked_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Remove <tool_result>...</tool_result> blocks (greedy within single block)
    while let Some(start) = result.find("<tool_result>") {
        if let Some(end) = result[start..].find("</tool_result>") {
            let tag_end = start + end + "</tool_result>".len();
            result = format!("{}{}", result[..start].trim_end(), result[tag_end..].trim_start());
        } else {
            // Unclosed tag — remove from <tool_result> to end of line
            let line_end = result[start..].find('\n').map(|i| start + i).unwrap_or(result.len());
            result = format!("{}{}", result[..start].trim_end(), &result[line_end..]);
        }
    }
    result.trim().to_string()
}

/// Strip `[TRANSLATE:...]` tags from text.
fn strip_translate_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let tag_end = start + end_bracket + 1;
            result = format!("{}{}", result[..start].trim_end(), result[tag_end..].trim_start());
        } else {
            // Unclosed tag — remove from [TRANSLATE: to end
            result = result[..start].trim_end().to_string();
        }
    }
    result.trim().to_string()
}

fn merge_continuation_text(accumulated: &mut String, next: &str) {
    if next.is_empty() {
        return;
    }
    if accumulated.is_empty() {
        accumulated.push_str(next);
        return;
    }
    if next.starts_with(accumulated.as_str()) {
        *accumulated = next.to_string();
        return;
    }
    if accumulated.ends_with(next) {
        return;
    }

    let mut overlap = 0usize;
    let max_overlap = accumulated.len().min(next.len());
    for candidate in (1..=max_overlap).rev() {
        if accumulated.is_char_boundary(accumulated.len() - candidate)
            && next.is_char_boundary(candidate)
            && accumulated[accumulated.len() - candidate..] == next[..candidate]
        {
            overlap = candidate;
            break;
        }
    }

    if overlap > 0 {
        accumulated.push_str(&next[overlap..]);
    } else {
        if !accumulated.ends_with(char::is_whitespace) && !next.starts_with(char::is_whitespace) {
            accumulated.push(' ');
        }
        accumulated.push_str(next);
    }
}

/// Extract the content inside `[TRANSLATE:...]` tags, then strip them from text.
/// Returns (cleaned_text, Option<translation>).
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
            result = format!("{}{}", result[..start].trim_end(), result[tag_end..].trim_start());
        } else {
            // Unclosed tag — extract what we can
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

/// Parsed tool call from `[TOOL_CALL:name|key=val|key=val]`
#[derive(Debug, Clone, Serialize)]
struct ToolCall {
    tool_call_id: Option<String>,
    name: String,
    args: HashMap<String, String>,
}

/// Parse all `[TOOL_CALL:name|key=val|...]` tags from the text.
/// Returns (cleaned_text, Vec<ToolCall>).
fn parse_tool_call_tags(text: &str) -> (String, Vec<ToolCall>) {
    let mut result = text.to_string();
    let mut calls = Vec::new();

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

                calls.push(ToolCall { tool_call_id: None, name, args });
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

    // 额外支持简化格式: [action_name|key=val|key=val]
    // 例: [play_cue|cue=shy]
    let mut extra_calls = Vec::new();
    let mut cleaned = result.clone();
    let mut offset = 0;
    while offset < cleaned.len() {
        let Some(rel_start) = cleaned[offset..].find('[') else { break };
        let start = offset + rel_start;
        let rest = &cleaned[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(pipe_pos) = inner.find('|') {
            let name_part = &inner[..pipe_pos];
            let is_identifier = !name_part.is_empty()
                && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');
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
                extra_calls.push(ToolCall { tool_call_id: None, name, args });
                let tag_end = start + end + 1;
                cleaned = format!(
                    "{}{}",
                    cleaned[..start].trim_end(),
                    if tag_end < cleaned.len() { &cleaned[tag_end..] } else { "" }
                );
                // offset 不变，继续从同一位置扫描（内容已缩短）
                matched = true;
            }
        }
        if !matched {
            // 跳过这个 [ 继续往后找
            offset = start + 1;
        }
    }
    calls.extend(extra_calls);

    // 支持冒号格式: [action_name:value]
    // 例: [play_cue:happy]、[set_background:beach]
    // 将 value 映射到该 action 的主参数名
    let primary_arg_map: &[(&str, &str)] = &[
        ("play_cue", "cue"),
        ("set_background", "prompt"),
    ];
    let mut colon_calls = Vec::new();
    let mut cleaned2 = cleaned.clone();
    let mut offset2 = 0;
    while offset2 < cleaned2.len() {
        let Some(rel_start) = cleaned2[offset2..].find('[') else { break };
        let start = offset2 + rel_start;
        let rest = &cleaned2[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(colon_pos) = inner.find(':') {
            let name_part = inner[..colon_pos].trim();
            let val_part = inner[colon_pos + 1..].trim();
            let is_identifier = !name_part.is_empty()
                && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');

            if is_identifier && !val_part.is_empty() {
                if let Some(&(_, arg_key)) = primary_arg_map.iter().find(|&&(n, _)| n == name_part) {
                    let mut args = HashMap::new();
                    args.insert(arg_key.to_string(), val_part.to_string());
                    colon_calls.push(ToolCall { tool_call_id: None, name: name_part.to_string(), args });
                    let tag_end = start + end + 1;
                    cleaned2 = format!(
                        "{}{}",
                        cleaned2[..start].trim_end(),
                        if tag_end < cleaned2.len() { &cleaned2[tag_end..] } else { "" }
                    );
                    matched = true;
                }
            }
        }
        if !matched {
            offset2 = start + 1;
        }
    }
    calls.extend(colon_calls);

    calls.reverse();
    (cleaned2.trim().to_string(), calls)
}

// ── Stream Chat Command ────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn stream_chat(
    window: Window,
    app: tauri::AppHandle,
    request: ChatRequest,
    state: State<'_, AIOrchestrator>,
    imagegen_state: State<'_, ImageGenService>,
    llm_state: State<'_, LlmService>,
    _action_registry: State<'_, std::sync::Arc<RwLock<crate::actions::ActionRegistry>>>,
    tool_settings_state: State<'_, std::sync::Arc<RwLock<ToolSettings>>>,
    _vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
    window_size_state: State<'_, WindowSizeState>,
    vision_server: State<
        '_,
        std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>,
    >,
) -> Result<(), KokoroError> {
    // 0. Resolve character ID for this request (not stored in shared state)
    let char_id = request
        .character_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    // Keep shared character_id in sync for modules that still read it (emotion snapshot, heartbeat)
    state.set_character_id(char_id.clone()).await;

    // Record user activity
    state.touch_activity().await;

    // Emotion update
    let emotion_classification = crate::ai::emotion_classifier::classify_text(&request.message).await;
    state
        .update_emotion(
            &emotion_classification.label,
            emotion_classification.raw_mood,
        )
        .await;

    // Typing simulation
    {
        let emotion = state.emotion_state.lock().await;
        let is_question = request.message.contains('?') || request.message.contains('？');
        let typing_params = crate::ai::typing_sim::calculate_typing_delay(
            emotion.current_emotion(),
            emotion.mood(),
            emotion.personality().expressiveness,
            request.message.chars().count(),
            is_question,
        );
        let _ = app.emit("chat-typing", &typing_params);
    }

    // 1. Update History with User Message (skip for hidden/touch interactions)
    if !request.hidden {
        state
            .add_message("user".to_string(), request.message.clone(), &char_id)
            .await;
    }

    // ── LAYER 1 & 2: SYSTEM SETUP ───────────────────────────────

    let system_provider = llm_state.system_provider().await;

    // ── EXECUTION & STATE UPDATE ────────────────────────────────

    // ── LAYER 3: PERSONA GENERATION ─────────────────────────────

    let llm_config = llm_state.config().await;
    let chat_provider = llm_state.provider().await;
    let native_tools_enabled = llm_config
        .providers
        .iter()
        .find(|provider| provider.id == llm_config.active_provider)
        .map(|provider| provider.supports_native_tools)
        .unwrap_or(true);
    println!(
        "[Chat] active_provider={}, native_tools_enabled={}",
        llm_config.active_provider, native_tools_enabled
    );

    // Native tool-calling requests already carry structured tool definitions,
    // so avoid duplicating a long textual tool prompt there.
    let tool_prompt = {
        let registry = _action_registry.read().await;
        let tool_settings = tool_settings_state.read().await;
        let prompt = if native_tools_enabled {
            String::new()
        } else {
            registry.generate_tool_prompt_for_prompt_with_settings(
                state.is_memory_enabled(),
                &tool_settings,
            )
        };
        if prompt.is_empty() { None } else { Some(prompt) }
    };

    let native_tools = {
        let registry = _action_registry.read().await;
        let tool_settings = tool_settings_state.read().await;
        registry.list_tools_for_llm_with_settings(state.is_memory_enabled(), &tool_settings)
    };

    // Compose Persona Prompt
    let prompt_messages = state
        .compose_prompt(
            &request.message,
            request.allow_image_gen.unwrap_or(false),
            tool_prompt,
            native_tools_enabled,
            &char_id,
        )
        .await
        .map_err(|e| KokoroError::Chat(e.to_string()))?;

    let mut client_messages = prompt_messages
        .into_iter()
        .map(|m| history_message_to_chat_message(&m.role, m.content, m.metadata.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    let assistant_turn_id = uuid::Uuid::new_v4().to_string();

    // 注入视觉上下文（如果有最近的屏幕观察）
    if let Some(vision_desc) = _vision_watcher.context.get_context_string().await {
        client_messages.push(system_message(format!(
            "[Vision] The user's screen currently shows: {}",
            vision_desc
        )));
    }

    // Attach images to the last user message if present
    if let Some(images) = &request.images {
        if !images.is_empty() {
            // Find the last message with role "user"
            if let Some(last_user_msg) = client_messages
                .iter_mut()
                .rfind(|m| crate::llm::messages::is_user_message(m))
            {
                let text_content = extract_message_text(last_user_msg);

                // Process images: convert local URLs to base64
                let mut processed_images = Vec::with_capacity(images.len());
                let vision_server_guard = vision_server.lock().await;
                let port = vision_server_guard.port;
                let upload_dir = vision_server_guard.upload_dir.clone();
                drop(vision_server_guard);

                for img_url in images {
                    let mut final_url = img_url.clone();
                    // Check if local
                    if img_url.contains(&format!("http://127.0.0.1:{}", port)) {
                        // Extract filename
                        if let Some(filename) = img_url.split("/vision/").nth(1) {
                            let file_path = upload_dir.join(filename);
                            if let Ok(file_content) = tokio::fs::read(&file_path).await {
                                // Convert to base64
                                use base64::Engine as _;
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(&file_content);
                                // Detect mime type
                                let mime = crate::vision::server::detect_image_mime(&file_content)
                                    .unwrap_or("image/png".to_string());
                                final_url = format!("data:{};base64,{}", mime, b64);
                            }
                        }
                    }
                    processed_images.push(final_url);
                }

                // Create multimodal content
                replace_user_message_with_images(last_user_msg, text_content, processed_images)?;
                println!("[Chat] Attached {} images to user message", images.len());
            }
        }
    }

    // For hidden messages (touch interactions), the user message wasn't added to
    // history, so we must explicitly include it in the context for the LLM to see.
    if request.hidden {
        client_messages.push(user_text_message(request.message.clone()));
    }

    #[cfg(debug_assertions)]
    {
        println!(
            "[LLM/Debug] active_provider={} native_tools_enabled={} tool_count={}",
            llm_config.active_provider,
            native_tools_enabled,
            native_tools.len()
        );
        debug_log_llm_messages("initial chat request", &client_messages);
    }

    // Stream Response with Tool Call Feedback Loop
    let max_tool_rounds = {
        let tool_settings = tool_settings_state.read().await;
        tool_settings.max_tool_rounds.max(1)
    };
    let mut all_cleaned_text = String::new();
    let mut all_translations = Vec::new();
    let mut bg_generated_by_tool = false;
    let mut cue_set_by_tool = false;
    let mut draft_row_id: Option<i64> = None;
    let mut forced_text_after_side_effect = false;

    for round in 0..max_tool_rounds {
        println!("[Chat] Tool loop round {}", round + 1);

        let mut stream = chat_provider
            .chat_stream_with_tools(client_messages.clone(), None, native_tools.clone())
            .await?;

        let mut round_response = String::new();
        let mut emit_buffer = String::new();
        let mut native_tool_calls = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(event) => {
                    match event {
                        LlmStreamEvent::Text(content) => {
                            round_response.push_str(&content);
                            emit_buffer.push_str(&content);

                            // Only emit text up to the safe boundary (before any potential tag)
                            let safe = find_safe_emit_boundary(&emit_buffer);
                            if safe > 0 {
                                let to_emit = emit_buffer[..safe].to_string();
                                emit_buffer = emit_buffer[safe..].to_string();
                                app
                                    .emit("chat-delta", &to_emit)
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                        LlmStreamEvent::ToolCall(tool_call) => {
                            native_tool_calls.push(ToolCall {
                                tool_call_id: Some(tool_call.id),
                                name: tool_call.name,
                                args: tool_call.args,
                            });
                        }
                    }
                }
                Err(e) => {
                    if round_response.is_empty() && emit_buffer.is_empty() {
                        app.emit("chat-error", e).map_err(|e| KokoroError::Chat(e.to_string()))?;
                    } else {
                        eprintln!(
                            "[Chat] Ignoring trailing stream error after partial response: {}",
                            e
                        );
                    }
                    break;
                }
            }
        }

        // Flush remaining buffer — strip any complete tags before emitting
        if !emit_buffer.is_empty() {
            let (cleaned_remainder, _) = parse_tool_call_tags(&emit_buffer);
            let cleaned_remainder = strip_translate_tags(&cleaned_remainder);
            if !cleaned_remainder.is_empty() {
                app
                    .emit("chat-delta", &cleaned_remainder)
                    .map_err(|e| KokoroError::Chat(e.to_string()))?;
            }
        }

        let (cleaned_text, mut tool_calls) = parse_tool_call_tags(&round_response);
        let (cleaned_text, round_translation) = extract_translate_tags(&cleaned_text);
        tool_calls.extend(native_tool_calls);

        println!("[Chat] Round {} raw response ({} chars): ...{}",
            round + 1,
            round_response.len(),
            round_response.chars().rev().take(100).collect::<String>().chars().rev().collect::<String>());
        println!("[Chat] Round {} translation: {:?}", round + 1, round_translation);
        println!("[Chat] Round {} tool_calls: {}", round + 1, tool_calls.len());

        // Collect translation from this round
        if let Some(t) = round_translation {
            all_translations.push(t);
        }

        // Accumulate cleaned text for history
        merge_continuation_text(&mut all_cleaned_text, &cleaned_text);

        // Persist assistant draft incrementally (hidden interactions still save the response, just not the user message)
        if !all_cleaned_text.is_empty() {
            let draft_content = strip_leaked_tags(&all_cleaned_text);
            if !draft_content.is_empty() {
                match draft_row_id {
                    None => {
                        // First round: insert draft row
                        match state.persist_streaming_draft(&draft_content, &char_id).await {
                            Ok(id) => { draft_row_id = Some(id); }
                            Err(e) => { eprintln!("[Chat] Failed to persist streaming draft: {}", e); }
                        }
                    }
                    Some(id) => {
                        // Subsequent rounds: update draft row
                        if let Err(e) = state.update_streaming_draft(id, &draft_content, None).await {
                            eprintln!("[Chat] Failed to update streaming draft: {}", e);
                        }
                    }
                }
            }
        }

        // No tool calls → final round
        if tool_calls.is_empty() {
            break;
        }

        // Execute tool calls and collect results
        let registry = _action_registry.read().await;
        let mut tool_results = Vec::new();
        let mut tool_result_messages = Vec::new();
        let mut continuation_tool_calls = Vec::new();
        let mut any_needs_feedback = false;
        let has_native_tool_calls = tool_calls.iter().any(|tc| tc.tool_call_id.is_some());

        for tc in &tool_calls {
            println!("[ToolCall] Executing: {} with args {:?}", tc.name, tc.args);
            if tc.name == "set_background" {
                bg_generated_by_tool = true;
            }
            if tc.name == "play_cue" {
                cue_set_by_tool = true;
            }
            if registry.needs_feedback(&tc.name) {
                any_needs_feedback = true;
            }
            let tool_enabled = {
                let tool_settings = tool_settings_state.read().await;
                tool_settings.is_enabled(&tc.name)
            };
            if !tool_enabled {
                let message = format!("Tool '{}' is disabled", tc.name);
                eprintln!("[ToolCall] {}", message);
                let _ = app.emit(
                    "chat-tool-result",
                    serde_json::json!({
                        "tool": tc.name,
                        "error": message,
                    }),
                );
                tool_results.push(format!("- {}: Error: {}", tc.name, message));
                if let Some(tool_call_id) = &tc.tool_call_id {
                    continuation_tool_calls.push((
                        tool_call_id.clone(),
                        tc.name.clone(),
                        serde_json::to_string(&tc.args).unwrap_or_else(|_| "{}".to_string()),
                    ));
                    tool_result_messages.push(tool_result_message(
                        tool_call_id.clone(),
                        format!("Error: {}", message),
                    ));
                }
                continue;
            }
            let ctx = crate::actions::registry::ActionContext {
                app: window.app_handle().clone(),
                character_id: char_id.clone(),
            };
            match registry.execute(&tc.name, tc.args.clone(), ctx).await {
                Ok(result) => {
                    println!("[ToolCall] {} => {}", tc.name, result.message);
                    let _ = app.emit(
                        "chat-tool-result",
                        serde_json::json!({
                            "tool": tc.name,
                            "result": result,
                        }),
                    );
                    tool_results.push(format!("- {}: {}", tc.name, result.message));
                    if let Some(tool_call_id) = &tc.tool_call_id {
                        continuation_tool_calls.push((
                            tool_call_id.clone(),
                            tc.name.clone(),
                            serde_json::to_string(&tc.args).unwrap_or_else(|_| "{}".to_string()),
                        ));
                        tool_result_messages.push(tool_result_message(
                            tool_call_id.clone(),
                            result.message.clone(),
                        ));
                    }
                }
                Err(e) => {
                    eprintln!("[ToolCall] {} failed: {}", tc.name, e.0);
                    let _ = app.emit(
                        "chat-tool-result",
                        serde_json::json!({
                            "tool": tc.name,
                            "error": e.0,
                        }),
                    );
                    tool_results.push(format!("- {}: Error: {}", tc.name, e.0));
                    if let Some(tool_call_id) = &tc.tool_call_id {
                        continuation_tool_calls.push((
                            tool_call_id.clone(),
                            tc.name.clone(),
                            serde_json::to_string(&tc.args).unwrap_or_else(|_| "{}".to_string()),
                        ));
                        tool_result_messages.push(tool_result_message(
                            tool_call_id.clone(),
                            format!("Error: {}", e.0),
                        ));
                    }
                }
            }
        }
        drop(registry);

        if has_native_tool_calls {
            let assistant_tool_call_metadata = serde_json::json!({
                "type": "assistant_tool_calls",
                "turn_id": assistant_turn_id,
                "tool_calls": continuation_tool_calls
                    .iter()
                    .map(|(id, name, arguments)| serde_json::json!({
                        "id": id,
                        "name": name,
                        "arguments": arguments,
                    }))
                    .collect::<Vec<_>>(),
            })
            .to_string();
            state
                .add_message_with_metadata(
                    "assistant".to_string(),
                    String::new(),
                    Some(assistant_tool_call_metadata),
                    &char_id,
                )
                .await;
            for (index, tool_message) in tool_result_messages.iter().enumerate() {
                if let Some(tool_call_id) = &tool_calls[index].tool_call_id {
                    let tool_content = extract_message_text(tool_message);
                    let tool_metadata = serde_json::json!({
                        "type": "tool_result",
                        "turn_id": assistant_turn_id,
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_calls[index].name,
                    })
                    .to_string();
                    state
                        .add_message_with_metadata(
                            "tool".to_string(),
                            tool_content,
                            Some(tool_metadata),
                            &char_id,
                        )
                        .await;
                }
            }
            client_messages.push(assistant_tool_calls_message(
                if cleaned_text.is_empty() {
                    None
                } else {
                    Some(cleaned_text.clone())
                },
                continuation_tool_calls,
            ));
            client_messages.extend(tool_result_messages);
            println!("[Chat] Continuing after native tool calls with assistant/tool result messages");
            #[cfg(debug_assertions)]
            debug_log_llm_messages(
                &format!("post-tool continuation round {}", round + 1),
                &client_messages,
            );
            continue;
        }

        // Only continue the loop if at least one tool needs its result fed back to the LLM
        if !any_needs_feedback {
            if all_cleaned_text.trim().is_empty() && !forced_text_after_side_effect {
                println!("[Chat] Side-effect tools ran without any text reply, forcing one follow-up text round");
                forced_text_after_side_effect = true;
                client_messages.push(system_message(format!(
                    "[Tool results]\n\
                    {}\n\n\
                    The side-effect tool has already been executed successfully.\n\
                    Now continue with a natural reply for the user in plain dialogue text.\n\
                    Do not explain the tool call, do not output metadata, and do not repeat the same side-effect tool unless it is still necessary.",
                    tool_results.join("\n")
                )));
                #[cfg(debug_assertions)]
                debug_log_llm_messages(
                    &format!("forced follow-up round {}", round + 1),
                    &client_messages,
                );
                continue;
            }

            println!("[Chat] No feedback-requiring tools, ending loop");
            break;
        }

        // Only inject tool results — no need to replay the assistant's previous output
        client_messages.push(system_message(format!(
            "[Tool results]\n\
            {}\n\n\
            Incorporate these results naturally into your dialogue. Do NOT echo raw data or JSON.",
            tool_results.join("\n")
        )));
        #[cfg(debug_assertions)]
        debug_log_llm_messages(
            &format!("feedback continuation round {}", round + 1),
            &client_messages,
        );
    }

    let full_response = strip_leaked_tags(&all_cleaned_text);

    // Fallback translation: if main LLM missed the [TRANSLATE:...] tag, use system LLM to fill in
    if all_translations.is_empty() && !full_response.is_empty() {
        let user_lang = state.user_language.lock().await.clone();
        let resp_lang = state.response_language.lock().await.clone();
        println!("[Chat] Fallback check: user_lang={:?}, resp_lang={:?}", user_lang, resp_lang);
        if !user_lang.is_empty() && !resp_lang.is_empty() && user_lang != resp_lang {
            println!("[Chat] Translation missing, triggering fallback translation into {}", user_lang);
            let fallback_messages = vec![
                system_message(format!(
                    "You are a translator. Translate the following text into {}. Output only the translation, nothing else.",
                    user_lang
                )),
                user_text_message(full_response.clone()),
            ];
            match system_provider.chat(fallback_messages, None).await {
                Ok(translation) => {
                    let t = translation.trim().to_string();
                    if !t.is_empty() {
                        println!("[Chat] Fallback translation succeeded ({} chars)", t.len());
                        all_translations.push(t);
                    }
                }
                Err(e) => {
                    eprintln!("[Chat] Fallback translation failed: {}", e);
                }
            }
        }
    }

    // Fallback cue: if main LLM never called play_cue, infer via system LLM
    if !cue_set_by_tool && !full_response.is_empty() {
        println!("[Chat] Cue not set by tool, triggering fallback cue analysis");
        let mut emotion_messages = vec![
            system_message(crate::ai::prompts::EMOTION_ANALYZER_PROMPT.to_string()),
        ];
        if let Some(profile) = crate::commands::live2d::load_active_live2d_profile() {
            let available_cues = profile
                .cue_map
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            emotion_messages.push(system_message(format!(
                "Available cues for the active model: {}.\nChoose exactly one from this list, or return null if none fit.",
                if available_cues.is_empty() { "(none)" } else { &available_cues }
            )));
        }
        emotion_messages.push(user_text_message(full_response.clone()));
        let valid_fallback_cues = crate::commands::live2d::load_active_live2d_profile()
            .map(|profile| profile.cue_map.keys().cloned().collect::<std::collections::HashSet<_>>());
        match system_provider.chat(emotion_messages, None).await {
            Ok(json_str) => {
                let clean = json_str.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```");
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean) {
                    if let Some(cue) = val.get("cue").and_then(|v| v.as_str()) {
                        let trimmed = cue.trim();
                        let is_valid = valid_fallback_cues
                            .as_ref()
                            .map(|cues| cues.contains(trimmed))
                            .unwrap_or(false);
                        if is_valid {
                            println!("[Chat] Fallback cue: {}", trimmed);
                            let _ = app.emit(
                                "chat-cue",
                                serde_json::json!({ "cue": trimmed, "source": "fallback-cue" }),
                            );
                        } else {
                            println!("[Chat] Ignoring invalid fallback cue: {}", trimmed);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[Chat] Fallback cue analysis failed: {}", e);
            }
        }
    }

    // Emit combined translation from all rounds
    if !all_translations.is_empty() {
        let combined_translation = all_translations.join(" ");
        let _ = app.emit("chat-translation", &combined_translation);
    }

    // 8. Update History with final response
    // hidden 模式下跳过用户消息保存，但助手回复仍需持久化以便重载后显示
    if !full_response.is_empty() {
        let metadata = if !all_translations.is_empty() {
            let combined = all_translations.join(" ");
            Some(serde_json::json!({
                "translation": combined,
                "turn_id": assistant_turn_id,
            }).to_string())
        } else {
            Some(serde_json::json!({
                "turn_id": assistant_turn_id,
            }).to_string())
        };

        // Update the draft row with final content + metadata (DB already has the row)
        if let Some(row_id) = draft_row_id {
            if let Err(e) = state.update_streaming_draft(row_id, &full_response, metadata.as_deref()).await {
                eprintln!("[Chat] Failed to finalize streaming draft: {}", e);
            }
        }

        // Add to in-memory history only (DB already persisted)
        {
            let max_chars = *state.max_message_chars.lock().await;
            let content = if full_response.chars().count() > max_chars {
                let truncated: String = full_response.chars().take(max_chars).collect();
                format!("{}…[truncated]", truncated)
            } else {
                full_response.clone()
            };
            state.push_history_message(Message {
                role: "assistant".to_string(),
                content,
                metadata: None,
            }).await;
        }
    }

    // Periodic memory extraction
    let msg_count = state.get_message_count().await;
    let memory_msg_count = state.get_memory_trigger_count().await;
    println!("[Memory] User message count: {}, memory trigger count: {}", msg_count, memory_msg_count);
    if state.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 5 == 0 {
        println!("[Memory] Triggering memory extraction (count={})", msg_count);
        let history = state.get_recent_memory_history(10).await;
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_mem = char_id.clone();
        let provider_for_mem = llm_state.provider().await;
        let memory_enabled = state.memory_enabled_flag();
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

    // Periodic memory consolidation (every 20 user messages)
    if state.is_memory_enabled() && memory_msg_count > 0 && memory_msg_count % 20 == 0 {
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_consolidation = char_id.clone();
        let provider_for_consolidation = llm_state.provider().await;
        let memory_enabled = state.memory_enabled_flag();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            match memory_mgr
                .consolidate_memories(&char_id_for_consolidation, provider_for_consolidation)
                .await
            {
                Ok(count) if count > 0 => {
                    println!("[Memory] Consolidated {} memory clusters", count);
                }
                Err(e) => {
                    eprintln!("[Memory] Consolidation failed: {}", e);
                }
                _ => {}
            }
        });
    }

    // Background image generation: analyze reply and optionally generate a scene image
    // Skip if the main LLM already triggered set_background via tool call
    if request.allow_image_gen.unwrap_or(false) && !full_response.is_empty() && !bg_generated_by_tool {
        let imagegen_svc = imagegen_state.inner().clone();
        let system_provider = llm_state.system_provider().await;
        let reply_for_analysis = full_response.clone();
        let window_for_img = window.clone();
        let window_size = window_size_state.get().await;

        tauri::async_runtime::spawn(async move {
            let analyze_messages = vec![
                system_message(crate::ai::prompts::BG_IMAGE_ANALYZER_PROMPT.to_string()),
                user_text_message(format!("Character reply: {}", reply_for_analysis)),
            ];

            let json_str = match system_provider.chat(analyze_messages, None).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ImageGen] BG analyzer LLM failed: {}", e);
                    return;
                }
            };

            let clean = json_str
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```");

            #[derive(serde::Deserialize)]
            struct BgAnalysis {
                should_generate: bool,
                image_prompt: Option<String>,
            }

            let analysis: BgAnalysis = match serde_json::from_str(clean) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("[ImageGen] BG analyzer parse failed: {} | raw: {}", e, json_str);
                    return;
                }
            };

            if !analysis.should_generate {
                println!("[ImageGen] BG analyzer: no image needed");
                return;
            }

            let prompt = match analysis.image_prompt {
                Some(p) if !p.is_empty() => p,
                _ => return,
            };

            println!("[ImageGen] BG analyzer triggered generation: {}", prompt);

            match imagegen_svc.generate(prompt.clone(), None, None, Some(window_size)).await {
                Ok(result) => {
                    let _ = window_for_img.emit("imagegen:done", &result);
                    println!("[ImageGen] BG image generated: {}", result.image_url);
                }
                Err(e) => {
                    eprintln!("[ImageGen] BG generation failed: {}", e);
                    let _ = window_for_img.emit("imagegen:error", e.to_string());
                }
            }
        });
    }

    app.emit(
        "chat-done",
        serde_json::json!({
            "text": full_response,
        }),
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_translate_tags ──────────────────────────────

    #[test]
    fn test_extract_translate_tags_basic() {
        let input = "こんにちは[TRANSLATE:你好]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, Some("你好".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_none() {
        let input = "こんにちは";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, None);
    }

    #[test]
    fn test_extract_translate_tags_multiple() {
        let input = "A[TRANSLATE:甲] B[TRANSLATE:乙]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "AB");
        assert_eq!(translation, Some("甲 乙".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_unclosed() {
        let input = "hello[TRANSLATE:world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "hello");
        assert_eq!(translation, Some("world".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_empty_content() {
        let input = "hello[TRANSLATE:]world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "helloworld");
        assert_eq!(translation, None);
    }

    // ── strip_translate_tags ────────────────────────────────

    #[test]
    fn test_strip_translate_tags() {
        let input = "こんにちは[TRANSLATE:你好]";
        assert_eq!(strip_translate_tags(input), "こんにちは");
    }

    #[test]
    fn test_strip_translate_tags_no_tag() {
        let input = "こんにちは";
        assert_eq!(strip_translate_tags(input), "こんにちは");
    }

    // ── strip_leaked_tags ───────────────────────────────────

    #[test]
    fn test_strip_leaked_tags_removes_tool_result() {
        let input = "before<tool_result>leaked data</tool_result>after";
        assert_eq!(strip_leaked_tags(input), "beforeafter");
    }

    #[test]
    fn test_strip_leaked_tags_unclosed() {
        let input = "before<tool_result>leaked\nafter";
        assert_eq!(strip_leaked_tags(input), "before\nafter");
    }

    #[test]
    fn test_strip_leaked_tags_no_tag() {
        let input = "clean text";
        assert_eq!(strip_leaked_tags(input), "clean text");
    }

    // ── find_safe_emit_boundary ─────────────────────────────

    #[test]
    fn test_safe_emit_boundary_no_bracket() {
        let text = "hello world";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_tool_call() {
        let text = "hello [TOOL_CA";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_translate() {
        let text = "hello [TRANS";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_unrelated_bracket() {
        let text = "hello [world]";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    // ── parse_tool_call_tags ────────────────────────────────

    #[test]
    fn test_parse_tool_call_basic() {
        let input = "text[TOOL_CALL:play_cue|cue=happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_no_tag() {
        let input = "just text";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "just text");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_multiple_args() {
        let input = "[TOOL_CALL:set_background|prompt=beach|style=anime]";
        let (_, calls) = parse_tool_call_tags(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args.get("prompt"), Some(&"beach".to_string()));
        assert_eq!(calls[0].args.get("style"), Some(&"anime".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_format() {
        let input = "text[play_cue|cue=shy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"shy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_multiple() {
        let input = "hello[play_cue|cue=happy]world[play_cue|cue=sad]end";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "helloworldend");
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_parse_tool_call_simplified_no_false_positive() {
        // 普通方括号内容不应被误识别
        let input = "text [some words] more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text [some words] more");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_colon_format() {
        let input = "text[play_cue:happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_colon_unknown_action_no_match() {
        // 未在映射表中的 action 不应被识别为工具调用
        let input = "text[unknown_action:value]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text[unknown_action:value]more");
        assert!(calls.is_empty());
    }
}
