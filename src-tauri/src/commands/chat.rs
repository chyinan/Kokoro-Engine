use crate::ai::context::AIOrchestrator;
use crate::ai::memory_extractor;
use crate::imagegen::ImageGenService;
use crate::llm::service::LlmService;
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{Emitter, Manager, State, Window};
use tokio::sync::RwLock;

#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub allow_image_gen: Option<bool>,
    pub images: Option<Vec<String>>,
    pub character_id: Option<String>,
    /// Optional override for the full message history (used for proactive triggers)
    pub messages: Option<Vec<crate::llm::openai::Message>>,
}

#[derive(Serialize, Clone)]
struct ExpressionEvent {
    expression: String,
    mood: f32,
}

#[derive(Serialize, Clone)]
struct ChatImageGenEvent {
    prompt: String,
}

#[derive(Serialize, Clone)]
struct ActionEvent {
    action: String,
}

/// Valid emotion names that the LLM can output
const VALID_EMOTIONS: &[&str] = &[
    "neutral",
    "happy",
    "sad",
    "angry",
    "surprised",
    "thinking",
    "shy",
    "smug",
    "worried",
    "excited",
];

/// Valid action names that the LLM can output
const VALID_ACTIONS: &[&str] = &[
    "idle", "nod", "shake", "wave", "dance", "shy", "think", "surprise", "cheer", "tap",
];

/// Mood value ranges for the keyword fallback
const EMOTION_MOODS: &[(&str, f32)] = &[
    ("excited", 0.95),
    ("happy", 0.85),
    ("smug", 0.7),
    ("surprised", 0.65),
    ("shy", 0.6),
    ("thinking", 0.5),
    ("neutral", 0.5),
    ("worried", 0.35),
    ("sad", 0.2),
    ("angry", 0.15),
];

const TAG_PREFIX: &str = "[EMOTION:";
const IMAGE_TAG_PREFIX: &str = "[IMAGE_PROMPT:";
const ACTION_TAG_PREFIX: &str = "[ACTION:";
const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";

// ── Tag Buffering ──────────────────────────────────────────

/// Returns the byte position up to which it's safe to emit text to the user.
/// Holds back any text that could be the start of an `[EMOTION:...]` or `[IMAGE_PROMPT:...]` tag.
fn find_safe_emit_boundary(text: &str) -> usize {
    if let Some(last_bracket) = text.rfind('[') {
        let suffix = &text[last_bracket..];

        // Check for EMOTION tag
        if suffix.len() < TAG_PREFIX.len() {
            if TAG_PREFIX.starts_with(suffix) {
                return last_bracket;
            }
        } else if suffix.starts_with(TAG_PREFIX) {
            return last_bracket;
        }

        // Check for IMAGE_PROMPT tag
        if suffix.len() < IMAGE_TAG_PREFIX.len() {
            if IMAGE_TAG_PREFIX.starts_with(suffix) {
                return last_bracket;
            }
        } else if suffix.starts_with(IMAGE_TAG_PREFIX) {
            return last_bracket;
        }

        // Check for ACTION tag
        if suffix.len() < ACTION_TAG_PREFIX.len() {
            if ACTION_TAG_PREFIX.starts_with(suffix) {
                return last_bracket;
            }
        } else if suffix.starts_with(ACTION_TAG_PREFIX) {
            return last_bracket;
        }

        // Check for TOOL_CALL tag
        if suffix.len() < TOOL_CALL_TAG_PREFIX.len() {
            if TOOL_CALL_TAG_PREFIX.starts_with(suffix) {
                return last_bracket;
            }
        } else if suffix.starts_with(TOOL_CALL_TAG_PREFIX) {
            return last_bracket;
        }
    }

    text.len()
}

// ── Tag Parsing ────────────────────────────────────────────

/// Parse `[IMAGE_PROMPT:...]` from the end of the text.
/// Returns (cleaned_text, Option<prompt>).
fn parse_image_prompt_tag(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim_end();

    if let Some(bracket_start) = trimmed.rfind(IMAGE_TAG_PREFIX) {
        let tag_text = &trimmed[bracket_start..];

        if let Some(bracket_end) = tag_text.find(']') {
            let inner = &tag_text[IMAGE_TAG_PREFIX.len()..bracket_end];
            let prompt = inner.trim().to_string();

            let cleaned = trimmed[..bracket_start].trim_end().to_string();
            return (cleaned, Some(prompt));
        }
    }

    (text.to_string(), None)
}

/// Parse `[ACTION:xxx]` from the text.
/// Returns (cleaned_text, Option<ActionEvent>).
fn parse_action_tag(text: &str) -> (String, Option<ActionEvent>) {
    let trimmed = text.trim_end();

    if let Some(bracket_start) = trimmed.rfind(ACTION_TAG_PREFIX) {
        let tag_text = &trimmed[bracket_start..];

        if let Some(bracket_end) = tag_text.find(']') {
            let inner = &tag_text[ACTION_TAG_PREFIX.len()..bracket_end];
            let action = inner.trim().to_lowercase();

            if VALID_ACTIONS.contains(&action.as_str()) {
                let cleaned = trimmed[..bracket_start].trim_end().to_string();
                return (cleaned, Some(ActionEvent { action }));
            }
        }
    }

    (text.to_string(), None)
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

    // Find all TOOL_CALL tags (iterate from end to preserve positions)
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

            // Remove the tag from the text
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

    // Reverse so calls are in order of appearance
    calls.reverse();
    (result.trim().to_string(), calls)
}

/// Parse `[EMOTION:xxx|MOOD:0.xx]` from the end of the response.
/// Returns (cleaned_text, ExpressionEvent).
/// Falls back to keyword detection if no valid tag is found.
fn parse_expression_tag(text: &str) -> (String, ExpressionEvent) {
    let trimmed = text.trim_end();

    if let Some(bracket_start) = trimmed.rfind("[EMOTION:") {
        let tag_text = &trimmed[bracket_start..];

        if let Some(bracket_end) = tag_text.find(']') {
            let inner = &tag_text[9..bracket_end]; // skip "[EMOTION:"

            if let Some(pipe_pos) = inner.find("|MOOD:") {
                let emotion = inner[..pipe_pos].trim().to_lowercase();
                let mood_str = inner[pipe_pos + 6..].trim();

                if VALID_EMOTIONS.contains(&emotion.as_str()) {
                    let mood = mood_str.parse::<f32>().unwrap_or(0.5).clamp(0.0, 1.0);
                    let cleaned = trimmed[..bracket_start].trim_end().to_string();

                    return (
                        cleaned,
                        ExpressionEvent {
                            expression: emotion,
                            mood,
                        },
                    );
                }
            }
        }
    }

    // Fallback: keyword-based detection
    let expression = detect_expression_keywords(text);
    (text.to_string(), expression)
}

/// Fallback keyword-based emotion detection.
fn detect_expression_keywords(text: &str) -> ExpressionEvent {
    let lower = text.to_lowercase();

    let checks: &[(&str, &[&str])] = &[
        (
            "excited",
            &["!!", "amazing", "awesome", "fantastic", "incredible", "wow"],
        ),
        (
            "happy",
            &[
                "glad",
                "happy",
                "great",
                "wonderful",
                "love",
                "enjoy",
                "pleased",
            ],
        ),
        ("smug", &["obviously", "of course", "naturally", "clearly"]),
        ("shy", &["blush", "embarrass", "flatter", "oh my"]),
        ("surprised", &["surprise", "unexpected", "no way", "whoa"]),
        (
            "thinking",
            &[
                "hmm",
                "let me think",
                "consider",
                "perhaps",
                "maybe",
                "interesting",
            ],
        ),
        (
            "worried",
            &["worry", "concern", "unfortunately", "afraid", "anxious"],
        ),
        ("sad", &["sad", "sorry", "unfortunate", "miss", "regret"]),
        ("angry", &["angry", "frustrat", "unacceptable", "terrible"]),
    ];

    for (emotion, keywords) in checks {
        for kw in *keywords {
            if lower.contains(kw) {
                let mood = EMOTION_MOODS
                    .iter()
                    .find(|(e, _)| *e == *emotion)
                    .map(|(_, m)| *m)
                    .unwrap_or(0.5);
                return ExpressionEvent {
                    expression: emotion.to_string(),
                    mood,
                };
            }
        }
    }

    ExpressionEvent {
        expression: "neutral".to_string(),
        mood: 0.5,
    }
}

// ── Stream Chat Command ────────────────────────────────────

#[tauri::command]
pub async fn stream_chat(
    window: Window,
    request: ChatRequest,
    state: State<'_, AIOrchestrator>,
    imagegen_state: State<'_, ImageGenService>,
    llm_state: State<'_, LlmService>,
    action_registry: State<'_, std::sync::Arc<RwLock<crate::actions::ActionRegistry>>>,
    vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
) -> Result<(), String> {
    // 0. Set character ID for memory isolation
    let char_id = request
        .character_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    state.set_character_id(char_id.clone()).await;

    // Record user activity (resets idle timer for proactive behavior)
    state.touch_activity().await;

    // Sentiment analysis — detect user's emotional tone for emotion contagion
    let user_sentiment = crate::ai::sentiment::analyze(&request.message);
    if user_sentiment.confidence > 0.2 {
        let mut emotion = state.emotion_state.lock().await;
        emotion.absorb_user_sentiment(user_sentiment.mood, user_sentiment.confidence);
    }

    // Typing simulation — emit typing indicator before response
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
        let _ = window.emit("chat-typing", &typing_params);
    }

    // 1. Update History with User Message (only if not proactive/override)
    if request.messages.is_none() {
        state
            .add_message("user".to_string(), request.message.clone())
            .await;
    }

    // 2. Compose Prompt or Use Override
    let mut client_messages = if let Some(msgs) = request.messages {
        msgs
    } else {
        // Default allow_image_gen to false if not provided, to prevent unwanted costs
        let allow_gen = request.allow_image_gen.unwrap_or(false);

        // Generate tool definitions for prompt injection
        let tool_prompt = {
            let registry = action_registry.read().await;
            let prompt = registry.generate_tool_prompt();
            if prompt.is_empty() {
                None
            } else {
                Some(prompt)
            }
        };

        let context_messages = state
            .compose_prompt(&request.message, allow_gen, tool_prompt)
            .await
            .map_err(|e| e.to_string())?;

        context_messages
            .into_iter()
            .map(|m| crate::llm::openai::Message {
                role: m.role,
                content: crate::llm::openai::MessageContent::Text(m.content),
            })
            .collect()
    };

    // 3b. If images are provided, upgrade the last user message to multimodal
    if let Some(ref imgs) = request.images {
        if !imgs.is_empty() {
            if let Some(last_user) = client_messages.iter_mut().rev().find(|m| m.role == "user") {
                let text = last_user.content.text();
                last_user.content =
                    crate::llm::openai::MessageContent::with_images(text, imgs.clone());
            }
        }
    }

    // 3c. Inject vision context into system message (if available)
    if let Some(vision_desc) = vision_watcher.context.get_context_string().await {
        if let Some(system_msg) = client_messages.iter_mut().find(|m| m.role == "system") {
            let existing = system_msg.content.text();
            system_msg.content = crate::llm::openai::MessageContent::Text(format!(
                "{}\n\n[VISION CONTEXT]: You can currently see on the user's screen: {}",
                existing, vision_desc
            ));
        }
    }

    // Get the active LLM provider from managed state
    let provider = llm_state.provider().await;

    // ── Tool Call Feedback Loop ──────────────────────────────
    // The LLM may output [TOOL_CALL:...] tags. After executing them, we feed the
    // results back and re-prompt so the LLM can use the output (e.g. MCP multi-step).
    // Max iterations prevents infinite loops.
    const MAX_TOOL_ITERATIONS: usize = 3;

    let mut final_expression = ExpressionEvent {
        expression: "neutral".to_string(),
        mood: 0.5,
    };
    let mut final_action: Option<ActionEvent> = None;
    let mut final_image_prompt: Option<String> = None;
    let mut final_clean_text = String::new();

    for iteration in 0..=MAX_TOOL_ITERATIONS {
        // 4. Stream with real-time tag filtering
        let mut stream = provider.chat_stream(client_messages.clone()).await?;
        let mut full_response = String::new();
        let mut emitted_len: usize = 0;

        while let Some(result) = stream.next().await {
            match result {
                Ok(content) => {
                    full_response.push_str(&content);
                    let safe_end = find_safe_emit_boundary(&full_response);
                    if safe_end > emitted_len {
                        let delta = full_response[emitted_len..safe_end].to_string();
                        window
                            .emit("chat-delta", delta)
                            .map_err(|e| e.to_string())?;
                        emitted_len = safe_end;
                    }
                }
                Err(e) => {
                    window.emit("chat-error", e).map_err(|e| e.to_string())?;
                }
            }
        }

        // 5. Parse tags
        let (text_no_emotion, expression) = parse_expression_tag(&full_response);
        let (text_no_action, action) = parse_action_tag(&text_no_emotion);
        let (text_no_image, image_prompt) = parse_image_prompt_tag(&text_no_action);
        let (clean_text, tool_calls) = parse_tool_call_tags(&text_no_image);

        // Save parsed results for post-loop processing
        final_expression = expression;
        if action.is_some() {
            final_action = action;
        }
        if image_prompt.is_some() {
            final_image_prompt = image_prompt;
        }
        final_clean_text = clean_text.clone();

        // 6. Flush remaining displayable text
        if emitted_len < clean_text.len() {
            let remaining = clean_text[emitted_len..].to_string();
            if !remaining.is_empty() {
                window
                    .emit("chat-delta", remaining)
                    .map_err(|e| e.to_string())?;
            }
        }

        // If no tool calls or we've hit the iteration limit, we're done
        if tool_calls.is_empty() || iteration == MAX_TOOL_ITERATIONS {
            if !tool_calls.is_empty() {
                println!(
                    "[Chat] Tool call iteration limit ({}) reached, stopping loop",
                    MAX_TOOL_ITERATIONS
                );
            }
            break;
        }

        // ── Execute Tool Calls & Feed Results Back ──────────
        println!(
            "[Chat] Iteration {}: executing {} tool call(s)",
            iteration,
            tool_calls.len()
        );

        let registry = action_registry.read().await;
        let mut tool_results_text = String::new();

        for tc in &tool_calls {
            println!("[Chat] Executing tool: {} {:?}", tc.name, tc.args);
            let ctx = crate::actions::ActionContext {
                app: window.app_handle().clone(),
                character_id: char_id.clone(),
            };
            let result = registry.execute(&tc.name, tc.args.clone(), ctx).await;

            match &result {
                Ok(r) => {
                    println!("[Chat] Tool result: {} - {}", tc.name, r.message);
                    let _ = window.emit(
                        "chat-tool-result",
                        serde_json::json!({
                            "tool": tc.name,
                            "result": r,
                        }),
                    );
                    // Format result for LLM feedback
                    tool_results_text
                        .push_str(&format!("[Tool '{}' returned]: {}\n", tc.name, r.message));
                    if let Some(ref data) = r.data {
                        // Include data payload (truncated to avoid token overflow)
                        let data_str =
                            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string());
                        let truncated = if data_str.len() > 4000 {
                            format!("{}...(truncated)", &data_str[..4000])
                        } else {
                            data_str
                        };
                        tool_results_text.push_str(&truncated);
                        tool_results_text.push('\n');
                    }
                }
                Err(e) => {
                    eprintln!("[Chat] Tool error: {} - {}", tc.name, e);
                    let _ = window.emit(
                        "chat-tool-result",
                        serde_json::json!({
                            "tool": tc.name,
                            "error": e.to_string(),
                        }),
                    );
                    tool_results_text.push_str(&format!("[Tool '{}' error]: {}\n", tc.name, e));
                }
            }
        }
        drop(registry);

        // Append assistant's response (with tool calls) and tool results to messages
        // so the LLM can see what happened and continue
        client_messages.push(crate::llm::openai::Message {
            role: "assistant".to_string(),
            content: crate::llm::openai::MessageContent::Text(full_response),
        });
        client_messages.push(crate::llm::openai::Message {
            role: "system".to_string(),
            content: crate::llm::openai::MessageContent::Text(format!(
                "Tool execution results:\n{}\n\
                 Now continue your response to the user, incorporating the tool results above. \
                 Keep your response natural and in character. \
                 Remember to append [EMOTION:<emotion>|MOOD:<value>] at the end.",
                tool_results_text
            )),
        });

        println!(
            "[Chat] Re-prompting LLM with tool results (iteration {})",
            iteration
        );
    }

    // 7. Update History with final cleaned response
    if !final_clean_text.is_empty() {
        state
            .add_message("assistant".to_string(), final_clean_text)
            .await;
    }

    // 7b. Periodic memory extraction (every 5 user messages)
    let msg_count = state.get_message_count().await;
    if msg_count > 0 && msg_count % 5 == 0 {
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

    // 8. Smooth emotion through state machine and emit
    let (smoothed_emotion, smoothed_mood) = state
        .update_emotion(&final_expression.expression, final_expression.mood)
        .await;
    window
        .emit(
            "chat-expression",
            ExpressionEvent {
                expression: smoothed_emotion,
                mood: smoothed_mood,
            },
        )
        .map_err(|e| e.to_string())?;

    // 8b. Emit action event (if any)
    if let Some(action_event) = final_action {
        println!("[Chat] Detected action: {}", action_event.action);
        window
            .emit("chat-action", action_event)
            .map_err(|e| e.to_string())?;
    }

    // 9. Handle Image Generation
    if let Some(prompt) = final_image_prompt {
        println!("[Chat] Detected image prompt: {}", prompt);

        // Notify frontend
        window
            .emit(
                "chat-imagegen",
                ChatImageGenEvent {
                    prompt: prompt.clone(),
                },
            )
            .map_err(|e| e.to_string())?;

        // Trigger generation in background
        let imagegen_service = imagegen_state.inner().clone();
        let app_handle = window.app_handle().clone();
        let prompt_clone = prompt.clone();

        tauri::async_runtime::spawn(async move {
            match imagegen_service.generate(prompt_clone, None, None).await {
                Ok(result) => {
                    println!("[Chat] Image generated successfully: {}", result.image_url);
                    let _ = app_handle.emit("imagegen:done", result);
                }
                Err(e) => {
                    eprintln!("[Chat] Image generation failed: {}", e);
                    let _ = app_handle.emit("imagegen:error", e.to_string());
                }
            }
        });
    }

    window.emit("chat-done", ()).map_err(|e| e.to_string())?;

    Ok(())
}
