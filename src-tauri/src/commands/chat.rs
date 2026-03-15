use crate::ai::context::AIOrchestrator;
use crate::ai::memory_extractor;
use crate::commands::system::WindowSizeState;
use crate::imagegen::ImageGenService;
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
) -> Result<ContextSettings, String> {
    let (strategy, max_message_chars) = state.get_context_settings().await;
    Ok(ContextSettings { strategy, max_message_chars })
}

#[tauri::command]
pub async fn set_context_settings(
    state: State<'_, AIOrchestrator>,
    settings: ContextSettings,
) -> Result<(), String> {
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
struct ExpressionEvent {
    expression: String,
    mood: f32,
}

#[derive(Serialize, Clone)]
#[allow(dead_code)]
struct ChatImageGenEvent {
    prompt: String,
}

#[derive(Serialize, Clone)]
struct ActionEvent {
    action: String,
}

#[derive(serde::Deserialize, Debug)]
struct IntentResponse {
    action_request: Option<String>,
    extra_info: Option<String>,
    // Optional: catch-all for system calls if we expand this
    #[serde(default)]
    #[allow(dead_code)]
    system_call: Option<String>,
}

/// Valid action names for the intent parser
const VALID_ACTIONS: &[&str] = &[
    "idle", "nod", "shake", "wave", "dance", "shy", "think", "surprise", "cheer", "tap",
];

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

    // 额外支持简化格式: [action_name|key=val|key=val]
    // 例: [change_expression|expression=shy]
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
                extra_calls.push(ToolCall { name, args });
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
    calls.reverse();
    (cleaned.trim().to_string(), calls)
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
    _vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
    window_size_state: State<'_, WindowSizeState>,
    vision_server: State<
        '_,
        std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>,
    >,
) -> Result<(), String> {
    // 0. Resolve character ID for this request (not stored in shared state)
    let char_id = request
        .character_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    // Keep shared character_id in sync for modules that still read it (emotion snapshot, heartbeat)
    state.set_character_id(char_id.clone()).await;

    // Record user activity
    state.touch_activity().await;

    // Sentiment analysis
    let user_sentiment = crate::ai::sentiment::analyze(&request.message);
    if user_sentiment.confidence > 0.2 {
        let mut emotion = state.emotion_state.lock().await;
        emotion.absorb_user_sentiment(user_sentiment.mood, user_sentiment.confidence);
    }

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

    // ── LAYER 1 & 2: INTENT PARSING ─────────────────────────────

    // Get System Provider
    let system_provider = llm_state.system_provider().await;

    // Construct Intent Prompt
    // We strictly want JSON.
    let intent_messages = vec![
        crate::llm::openai::Message {
            role: "system".to_string(),
            content: crate::llm::openai::MessageContent::Text(
                crate::ai::prompts::INTENT_PARSER_SYSTEM_PROMPT.to_string(),
            ),
        },
        crate::llm::openai::Message {
            role: "user".to_string(),
            content: crate::llm::openai::MessageContent::Text(format!(
                "Classify the following text and return JSON only.\nText: [{}]\nCharacter context: {}",
                request.message, char_id
            )),
        },
    ];

    println!("[Chat] Running Intent Parser...");
    let intent_json_str = system_provider
        .chat(intent_messages, None)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[Chat] Intent Parser failed: {}", e);
            "{}".to_string() // Fallback to empty JSON object
        });

    // Clean JSON (remove markdown code blocks if any)
    let clean_json = intent_json_str
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```");

    let intent: IntentResponse = serde_json::from_str(clean_json).unwrap_or_else(|e| {
        eprintln!(
            "[Chat] Failed to parse Intent JSON: {} | Raw: {}",
            e, intent_json_str
        );
        IntentResponse {
            action_request: None,
            extra_info: None,
            system_call: None,
        }
    });

    println!("[Chat] Parsed Intent: {:?}", intent);

    // ── EXECUTION & STATE UPDATE ────────────────────────────────

    // 1. Get current emotion state (emotion is driven by change_expression tool call in main LLM response)
    let (current_expression, _current_mood) = {
        let emotion_state = state.emotion_state.lock().await;
        (emotion_state.current_emotion().to_string(), emotion_state.mood())
    };

    // 2. Action Execution
    if let Some(ref action) = intent.action_request {
        if action == "play_animation" || VALID_ACTIONS.contains(&action.as_str()) {
            // If valid action or generic play request with extra_info
            let action_name = if VALID_ACTIONS.contains(&action.as_str()) {
                action.clone()
            } else if let Some(ref extra) = intent.extra_info {
                if VALID_ACTIONS.contains(&extra.as_str()) {
                    extra.clone()
                } else {
                    "idle".to_string()
                }
            } else {
                "idle".to_string()
            };

            window
                .emit(
                    "chat-action",
                    ActionEvent {
                        action: action_name.clone(),
                    },
                )
                .map_err(|e| e.to_string())?;
        }
    }

    // 3. System Calls / Tools (Simplified for now - can expand to full tool loop if needed)
    // For this refactor, we are assuming 'system_call' might map to existing actions
    // If we need the full while loop for tools, we can re-introduce it here, but driven by intent.
    // user asked for strict 3-layer, so let's keep it clean.

    // Prepare System Feedback for Persona
    let system_feedback = format!(
        "(Internal System Note)\n\
        - State Updated: Emotion is now '{}'.\n\
        - Action Performed: {}.\n\
        Continue the dialogue naturally based on this state. Do NOT explicitly mention the system update.",
        current_expression,
        intent.action_request.as_deref().unwrap_or("None"),
    );

    // ── LAYER 3: PERSONA GENERATION ─────────────────────────────

    // Generate tool prompt from action registry
    let tool_prompt = {
        let registry = _action_registry.read().await;
        let prompt = registry.generate_tool_prompt();
        if prompt.is_empty() { None } else { Some(prompt) }
    };

    // Compose Persona Prompt
    let prompt_messages = state
        .compose_prompt(
            &request.message,
            request.allow_image_gen.unwrap_or(false),
            tool_prompt,
            &char_id,
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut client_messages: Vec<crate::llm::openai::Message> = prompt_messages
        .into_iter()
        .map(|m| crate::llm::openai::Message {
            role: m.role,
            content: crate::llm::openai::MessageContent::Text(m.content),
        })
        .collect();

    // 注入视觉上下文（如果有最近的屏幕观察）
    if let Some(vision_desc) = _vision_watcher.context.get_context_string().await {
        client_messages.push(crate::llm::openai::Message {
            role: "system".to_string(),
            content: crate::llm::openai::MessageContent::Text(format!(
                "[Vision] The user's screen currently shows: {}",
                vision_desc
            )),
        });
    }

    // Attach images to the last user message if present
    if let Some(images) = &request.images {
        if !images.is_empty() {
            // Find the last message with role "user"
            if let Some(last_user_msg) = client_messages.iter_mut().rfind(|m| m.role == "user") {
                let text_content = last_user_msg.content.text();

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
                last_user_msg.content =
                    crate::llm::openai::MessageContent::with_images(text_content, processed_images);
                println!("[Chat] Attached {} images to user message", images.len());
            }
        }
    }

    // For hidden messages (touch interactions), the user message wasn't added to
    // history, so we must explicitly include it in the context for the LLM to see.
    if request.hidden {
        client_messages.push(crate::llm::openai::Message {
            role: "user".to_string(),
            content: crate::llm::openai::MessageContent::Text(request.message.clone()),
        });
    }

    // Inject System Feedback (Before the last user message or as system at end)
    // Best practice: System instruction near end of context
    client_messages.push(crate::llm::openai::Message {
        role: "system".to_string(),
        content: crate::llm::openai::MessageContent::Text(system_feedback),
    });

    // Stream Response with Tool Call Feedback Loop
    const MAX_TOOL_ROUNDS: usize = 5;
    let chat_provider = llm_state.provider().await;
    let mut all_cleaned_text = String::new();
    let mut all_translations = Vec::new();
    let mut bg_generated_by_tool = false;
    let mut expression_set_by_tool = false;
    let mut draft_row_id: Option<i64> = None;

    for round in 0..MAX_TOOL_ROUNDS {
        println!("[Chat] Tool loop round {}", round + 1);

        let mut stream = chat_provider
            .chat_stream(client_messages.clone(), None)
            .await?;

        let mut round_response = String::new();
        let mut emit_buffer = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(content) => {
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
                Err(e) => {
                    app.emit("chat-error", e).map_err(|e| e.to_string())?;
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
                    .map_err(|e| e.to_string())?;
            }
        }

        let (cleaned_text, tool_calls) = parse_tool_call_tags(&round_response);
        let (cleaned_text, round_translation) = extract_translate_tags(&cleaned_text);

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
        if !cleaned_text.is_empty() {
            if !all_cleaned_text.is_empty() {
                all_cleaned_text.push(' ');
            }
            all_cleaned_text.push_str(&cleaned_text);
        }

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
        let mut any_needs_feedback = false;

        for tc in &tool_calls {
            println!("[ToolCall] Executing: {} with args {:?}", tc.name, tc.args);
            if tc.name == "set_background" {
                bg_generated_by_tool = true;
            }
            if tc.name == "change_expression" {
                expression_set_by_tool = true;
            }
            if registry.needs_feedback(&tc.name) {
                any_needs_feedback = true;
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
                            "result": result.message,
                        }),
                    );
                    tool_results.push(format!("- {}: {}", tc.name, result.message));
                }
                Err(e) => {
                    eprintln!("[ToolCall] {} failed: {}", tc.name, e.0);
                    let _ = app.emit(
                        "chat-tool-result",
                        serde_json::json!({
                            "tool": tc.name,
                            "result": format!("Error: {}", e.0),
                        }),
                    );
                    tool_results.push(format!("- {}: Error: {}", tc.name, e.0));
                }
            }
        }
        drop(registry);

        // Only continue the loop if at least one tool needs its result fed back to the LLM
        if !any_needs_feedback {
            println!("[Chat] No feedback-requiring tools, ending loop");
            break;
        }

        // Append assistant message + tool results to context for next round
        client_messages.push(crate::llm::openai::Message {
            role: "assistant".to_string(),
            content: crate::llm::openai::MessageContent::Text(round_response),
        });
        client_messages.push(crate::llm::openai::Message {
            role: "system".to_string(),
            content: crate::llm::openai::MessageContent::Text(format!(
                "[Internal tool callback — NOT a user message]\n\
                The assistant message above is YOUR own previous output. You called some tools, and here are the results:\n\
                {}\n\n\
                Instructions:\n\
                - Incorporate these results naturally into your dialogue.\n\
                - Do NOT echo raw data, JSON, or any <tool_result> tags.\n\
                - Do NOT say the message was repeated or sent again — it was not.\n\
                - Simply continue talking to the user as if you just learned this information.",
                tool_results.join("\n")
            )),
        });
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
                crate::llm::openai::Message {
                    role: "system".to_string(),
                    content: crate::llm::openai::MessageContent::Text(
                        format!("You are a translator. Translate the following text into {}. Output only the translation, nothing else.", user_lang)
                    ),
                },
                crate::llm::openai::Message {
                    role: "user".to_string(),
                    content: crate::llm::openai::MessageContent::Text(full_response.clone()),
                },
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

    // Fallback emotion: if main LLM never called change_expression, infer via system LLM
    if !expression_set_by_tool && !full_response.is_empty() {
        println!("[Chat] Expression not set by tool, triggering fallback emotion analysis");
        let emotion_messages = vec![
            crate::llm::openai::Message {
                role: "system".to_string(),
                content: crate::llm::openai::MessageContent::Text(
                    crate::ai::prompts::EMOTION_ANALYZER_PROMPT.to_string(),
                ),
            },
            crate::llm::openai::Message {
                role: "user".to_string(),
                content: crate::llm::openai::MessageContent::Text(full_response.clone()),
            },
        ];
        match system_provider.chat(emotion_messages, None).await {
            Ok(json_str) => {
                let clean = json_str.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```");
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean) {
                    if let Some(expr) = val.get("expression").and_then(|v| v.as_str()) {
                        println!("[Chat] Fallback expression: {}", expr);
                        let _ = app.emit("chat-expression", serde_json::json!({ "expression": expr, "mood": 0.5 }));
                    }
                }
            }
            Err(e) => {
                eprintln!("[Chat] Fallback emotion analysis failed: {}", e);
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
            Some(serde_json::json!({ "translation": combined }).to_string())
        } else {
            None
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
            let mut history = state.history.lock().await;
            history.push_back(crate::ai::context::Message {
                role: "assistant".to_string(),
                content,
                metadata: None,
            });
            if history.len() > 30 {
                history.pop_front();
            }
        }
    }

    // Periodic memory extraction
    let msg_count = state.get_message_count().await;
    println!("[Memory] User message count: {}, trigger at next multiple of 5", msg_count);
    if msg_count > 0 && msg_count % 5 == 0 {
        println!("[Memory] Triggering memory extraction (count={})", msg_count);
        let history = state.get_recent_history(10).await;
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_mem = char_id.clone();
        let provider_for_mem = llm_state.provider().await;
        tauri::async_runtime::spawn(async move {
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
    if msg_count > 0 && msg_count % 20 == 0 {
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_consolidation = char_id.clone();
        let provider_for_consolidation = llm_state.provider().await;
        tauri::async_runtime::spawn(async move {
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
                crate::llm::openai::Message {
                    role: "system".to_string(),
                    content: crate::llm::openai::MessageContent::Text(
                        crate::ai::prompts::BG_IMAGE_ANALYZER_PROMPT.to_string(),
                    ),
                },
                crate::llm::openai::Message {
                    role: "user".to_string(),
                    content: crate::llm::openai::MessageContent::Text(
                        format!("Character reply: {}", reply_for_analysis)
                    ),
                },
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

    app.emit("chat-done", ()).map_err(|e| e.to_string())?;

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
        let input = "text[TOOL_CALL:change_expression|expression=happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "change_expression");
        assert_eq!(calls[0].args.get("expression"), Some(&"happy".to_string()));
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
        let input = "text[change_expression|expression=shy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "change_expression");
        assert_eq!(calls[0].args.get("expression"), Some(&"shy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_multiple() {
        let input = "hello[change_expression|expression=happy]world[change_expression|expression=sad]end";
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
}
