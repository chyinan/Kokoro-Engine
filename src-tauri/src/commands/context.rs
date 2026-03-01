use crate::ai::context::AIOrchestrator;
use crate::llm::openai::{Message as LLMMessage, MessageContent, OpenAIClient};
use tauri::State;

#[tauri::command]
pub async fn set_persona(prompt: String, state: State<'_, AIOrchestrator>) -> Result<(), String> {
    state.set_system_prompt(prompt).await;
    Ok(())
}

#[tauri::command]
pub async fn set_response_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state.set_response_language(language).await;
    Ok(())
}

#[tauri::command]
pub async fn set_user_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state.set_user_language(language).await;
    Ok(())
}

#[tauri::command]
pub async fn set_jailbreak_prompt(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state.set_jailbreak_prompt(prompt.clone()).await;

    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let path = app_data.join("jailbreak_prompt.json");
    let _ = std::fs::write(&path, serde_json::json!({ "prompt": prompt }).to_string());

    Ok(())
}

#[tauri::command]
pub async fn get_jailbreak_prompt(
    state: State<'_, AIOrchestrator>,
) -> Result<String, String> {
    Ok(state.get_jailbreak_prompt().await)
}

#[tauri::command]
pub async fn set_proactive_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state.set_proactive_enabled(enabled);
    println!("[AI] Proactive messages {}", if enabled { "enabled" } else { "disabled" });

    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let path = app_data.join("proactive_enabled.json");
    let _ = std::fs::write(&path, serde_json::json!({ "enabled": enabled }).to_string());
    Ok(())
}

#[tauri::command]
pub async fn get_proactive_enabled(
    state: State<'_, AIOrchestrator>,
) -> Result<bool, String> {
    Ok(state.is_proactive_enabled())
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AIOrchestrator>) -> Result<(), String> {
    state.clear_history().await;
    Ok(())
}

#[tauri::command]
pub async fn delete_last_messages(
    count: usize,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    let mut history = state.history.lock().await;
    let current_len = history.len();
    let to_remove = count.min(current_len);
    history.truncate(current_len - to_remove);
    println!("[AI] Deleted last {} message(s) from history", to_remove);
    Ok(())
}

/// End the current session: generate a summary from recent history, save it,
/// then clear conversation history. The summary generation runs in background.
#[derive(serde::Deserialize)]
pub struct EndSessionRequest {
    pub api_key: String,
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

#[tauri::command]
pub async fn end_session(
    request: EndSessionRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    let history = state.get_recent_history(20).await;
    let char_id = state.get_character_id().await;
    let memory_mgr = state.memory_manager.clone();

    // Clear history immediately so the user can start fresh
    state.clear_history().await;

    // Generate session summary in the background
    if history.len() >= 2 {
        tauri::async_runtime::spawn(async move {
            let transcript = history
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");

            let messages = vec![
                LLMMessage {
                    role: "system".to_string(),
                    content: MessageContent::Text(
                        concat!(
                        "You are a conversation summarizer. Write a brief 2-3 sentence summary ",
                        "of this conversation in the language the users were speaking. ",
                        "Focus on key topics discussed, any emotional moments, and important ",
                        "information shared. Write from a third-person perspective.\n",
                        "Output ONLY the summary, no labels or formatting."
                    )
                        .to_string(),
                    ),
                },
                LLMMessage {
                    role: "user".to_string(),
                    content: MessageContent::Text(format!(
                        "Summarize this conversation:\n\n{}",
                        transcript
                    )),
                },
            ];

            let client = OpenAIClient::new(request.api_key, request.endpoint, request.model);

            match client.chat(messages, None).await {
                Ok(summary) => {
                    let summary = summary.trim().to_string();
                    if !summary.is_empty() {
                        if let Err(e) = memory_mgr.save_session_summary(&char_id, &summary).await {
                            eprintln!("[Session] Failed to save summary: {}", e);
                        } else {
                            println!(
                                "[Session] Saved summary for '{}': {}",
                                char_id,
                                &summary[..summary.len().min(80)]
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Session] Summary generation failed: {}", e);
                }
            }
        });
    }

    Ok(())
}
