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
    /// If true, neither the user message nor the assistant response is saved to history.
    /// Used for touch interactions and proactive triggers where the instruction shouldn't appear in chat.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Serialize, Clone)]
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
    emotion_target: Option<String>,
    need_translation: Option<bool>,
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

    calls.reverse();
    (result.trim().to_string(), calls)
}

// ── Stream Chat Command ────────────────────────────────────

#[tauri::command]
pub async fn stream_chat(
    window: Window,
    request: ChatRequest,
    state: State<'_, AIOrchestrator>,
    _imagegen_state: State<'_, ImageGenService>,
    llm_state: State<'_, LlmService>,
    _action_registry: State<'_, std::sync::Arc<RwLock<crate::actions::ActionRegistry>>>,
    _vision_watcher: State<'_, crate::vision::watcher::VisionWatcher>,
    vision_server: State<
        '_,
        std::sync::Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>,
    >,
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

    // 1. Update History with User Message (skip for hidden/touch interactions)
    if !request.hidden {
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
            emotion_target: None,
            need_translation: None,
            extra_info: None,
            system_call: None,
        }
    });

    println!("[Chat] Parsed Intent: {:?}", intent);

    // ── EXECUTION & STATE UPDATE ────────────────────────────────

    // 1. Emotion Update
    let (current_expression, _current_mood) = if let Some(emo) = intent.emotion_target {
        let (new_expr, new_mood) = state.update_emotion(&emo, 0.5).await;

        // Emit immediate visual update
        window
            .emit(
                "chat-expression",
                ExpressionEvent {
                    expression: new_expr.clone(),
                    mood: new_mood,
                },
            )
            .map_err(|e| e.to_string())?;
        (new_expr, new_mood)
    } else {
        // Just get current state
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

    for round in 0..MAX_TOOL_ROUNDS {
        println!("[Chat] Tool loop round {}", round + 1);

        let mut stream = chat_provider
            .chat_stream(client_messages.clone(), None)
            .await?;

        let mut round_response = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(content) => {
                    round_response.push_str(&content);
                    window
                        .emit("chat-delta", &content)
                        .map_err(|e| e.to_string())?;
                }
                Err(e) => {
                    window.emit("chat-error", e).map_err(|e| e.to_string())?;
                }
            }
        }

        let (cleaned_text, tool_calls) = parse_tool_call_tags(&round_response);

        // Accumulate cleaned text for history
        if !cleaned_text.is_empty() {
            if !all_cleaned_text.is_empty() {
                all_cleaned_text.push(' ');
            }
            all_cleaned_text.push_str(&cleaned_text);
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
                    let _ = window.emit(
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
                    let _ = window.emit(
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

    // 8. Update History with final response (skip for hidden/touch interactions)
    if !full_response.is_empty() && !request.hidden {
        state
            .add_message("assistant".to_string(), full_response.clone())
            .await;
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

    window.emit("chat-done", ()).map_err(|e| e.to_string())?;

    Ok(())
}
