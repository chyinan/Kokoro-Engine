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

#[derive(serde::Deserialize, Debug)]
struct IntentResponse {
    action_request: Option<String>,
    emotion_target: Option<String>,
    need_translation: Option<bool>,
    extra_info: Option<String>,
    // Optional: catch-all for system calls if we expand this
    #[serde(default)]
    system_call: Option<String>,
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
    _imagegen_state: State<'_, ImageGenService>,
    llm_state: State<'_, LlmService>,
    action_registry: State<'_, std::sync::Arc<RwLock<crate::actions::ActionRegistry>>>,
    _vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
) -> Result<(), String> {
    // 0. Set character ID for memory isolation
    let char_id = request
        .character_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
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
        let _ = window.emit("chat-typing", &typing_params);
    }

    // 1. Update History with User Message
    if request.messages.is_none() {
        state
            .add_message("user".to_string(), request.message.clone())
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
                "User message: {}\nContext: (Character: {})",
                request.message, char_id
            )),
        },
    ];

    println!("[Chat] Running Intent Parser...");
    let intent_json_str = system_provider
        .chat(intent_messages)
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
            emotion_target: None,
            need_translation: None,
            extra_info: None,
            system_call: None,
        }
    });

    println!("[Chat] Parsed Intent: {:?}", intent);

    // ── EXECUTION & STATE UPDATE ────────────────────────────────

    // 1. Emotion Update
    let mut current_expression = "neutral".to_string();
    let mut current_mood = 0.5;

    if let Some(emo) = intent.emotion_target {
        let (new_expr, new_mood) = state.update_emotion(&emo, 0.5).await; // 0.5 as neutral mood strength change for now
        current_expression = new_expr;
        current_mood = new_mood;

        // Emit immediate visual update
        window
            .emit(
                "chat-expression",
                ExpressionEvent {
                    expression: current_expression.clone(),
                    mood: current_mood,
                },
            )
            .map_err(|e| e.to_string())?;
    } else {
        // Just get current state
        let emotion_state = state.emotion_state.lock().await;
        current_expression = emotion_state.current_emotion().to_string();
        current_mood = emotion_state.mood();
    }

    // 2. Action Execution
    if let Some(ref action) = intent.action_request {
        if action == "play_animation" || VALID_ACTIONS.contains(&action.as_str()) {
            // If valid action or generic play request with extra_info
            let action_name = if VALID_ACTIONS.contains(&action.as_str()) {
                action.clone()
            } else if let Some(ref extra) = intent.extra_info {
                extra.clone()
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
        - Translation Needed: {}.\n\
        Continue the dialogue naturally based on this state. Do NOT explicitly mention the system update.",
        current_expression,
        intent.action_request.as_deref().unwrap_or("None"),
        intent.need_translation.map(|b| b.to_string()).unwrap_or("false".to_string())
    );

    // ── LAYER 3: PERSONA GENERATION ─────────────────────────────

    // Compose Persona Prompt
    let mut client_messages = state
        .compose_prompt(
            &request.message,
            request.allow_image_gen.unwrap_or(false),
            None,
        )
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| crate::llm::openai::Message {
            role: m.role,
            content: crate::llm::openai::MessageContent::Text(m.content),
        })
        .collect::<Vec<_>>();

    // Inject System Feedback (Before the last user message or as system at end)
    // Best practice: System instruction near end of context
    client_messages.push(crate::llm::openai::Message {
        role: "system".to_string(),
        content: crate::llm::openai::MessageContent::Text(system_feedback),
    });

    // Stream Response
    let chat_provider = llm_state.provider().await;
    let mut stream = chat_provider.chat_stream(client_messages).await?;

    let mut full_response = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(content) => {
                full_response.push_str(&content);
                window
                    .emit("chat-delta", content)
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => {
                window.emit("chat-error", e).map_err(|e| e.to_string())?;
            }
        }
    }

    // 7. Update History with final response
    if !full_response.is_empty() {
        state
            .add_message("assistant".to_string(), full_response.clone())
            .await;
    }

    // Periodic memory extraction
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

    window.emit("chat-done", ()).map_err(|e| e.to_string())?;

    Ok(())
}
