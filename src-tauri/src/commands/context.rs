use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::{build_openai_client, create_chat};
use tauri::State;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct MemorySystemConfig {
    enabled: bool,
}

fn memory_config_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("memory_system_config.json")
}

#[derive(serde::Serialize)]
pub struct EmotionStateResponse {
    pub emotion: String,
    pub mood: f32,
}

#[tauri::command]
pub async fn get_emotion_state(
    state: State<'_, AIOrchestrator>,
) -> Result<EmotionStateResponse, KokoroError> {
    let emotion = state.emotion_state.lock().await;
    Ok(EmotionStateResponse {
        emotion: emotion.current_emotion().to_string(),
        mood: emotion.mood(),
    })
}

#[tauri::command]
pub async fn set_persona(prompt: String, state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    state.set_system_prompt(prompt).await;
    Ok(())
}

#[tauri::command]
pub async fn set_character_name(name: String, state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    state.set_character_name(name).await;
    Ok(())
}

#[tauri::command]
pub async fn set_active_character_id(id: String, state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    state.set_character_id(id.clone()).await;
    crate::ai::context::AIOrchestrator::persist_active_character_id(&id);
    Ok(())
}

#[tauri::command]
pub async fn set_user_name(name: String, state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    state.set_user_name(name).await;
    Ok(())
}

#[tauri::command]
pub async fn set_response_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_response_language(language).await;
    Ok(())
}

#[tauri::command]
pub async fn set_user_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_user_language(language).await;
    Ok(())
}

#[tauri::command]
pub async fn set_jailbreak_prompt(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
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
) -> Result<String, KokoroError> {
    Ok(state.get_jailbreak_prompt().await)
}

#[tauri::command]
pub async fn set_proactive_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
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
) -> Result<bool, KokoroError> {
    Ok(state.is_proactive_enabled())
}

#[tauri::command]
pub async fn set_memory_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_memory_enabled(enabled).await;
    crate::config::save_json_config(
        &memory_config_path(),
        &MemorySystemConfig { enabled },
        "MEMORY",
    )
}

#[tauri::command]
pub async fn get_memory_enabled(
    state: State<'_, AIOrchestrator>,
) -> Result<bool, KokoroError> {
    Ok(state.is_memory_enabled())
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    state.clear_history().await;
    Ok(())
}

#[tauri::command]
pub async fn delete_last_messages(
    count: usize,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let mut history = state.history.lock().await;
    let current_len = history.len();
    let to_remove = count.min(current_len);

    if to_remove == 0 {
        return Ok(());
    }

    history.truncate(current_len - to_remove);
    let new_len = history.len();
    println!("[AI] Deleted last {} message(s) from history (now {} messages)", to_remove, new_len);

    // 同时删除数据库中的消息，保留到 new_len 条
    let conv_id = state.current_conversation_id.lock().await.clone();
    if let Some(conversation_id) = conv_id {
        // 获取该对话的所有消息 ID，按顺序排列
        let message_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
        )
        .bind(&conversation_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        // 删除超过 new_len 的所有消息
        if message_ids.len() > new_len {
            let ids_to_delete = &message_ids[new_len..];
            for id in ids_to_delete {
                sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
                    .bind(id)
                    .execute(&state.db)
                    .await
                    .map_err(|e| KokoroError::Database(e.to_string()))?;
            }
            println!("[AI] Deleted {} message(s) from database (kept {} messages)", ids_to_delete.len(), new_len);
        }
    }

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
) -> Result<(), KokoroError> {
    if !state.is_memory_enabled() {
        state.clear_history().await;
        return Ok(());
    }

    let history = state.get_recent_history(20).await;
    let char_id = state.get_character_id().await;
    let memory_mgr = state.memory_manager.clone();
    let memory_enabled = state.memory_enabled_flag();

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
                system_message(
                    concat!(
                        "You are a conversation summarizer. Write a brief 2-3 sentence summary ",
                        "of this conversation in the language the users were speaking. ",
                        "Focus on key topics discussed, any emotional moments, and important ",
                        "information shared. Write from a third-person perspective.\n",
                        "Output ONLY the summary, no labels or formatting."
                    )
                    .to_string(),
                ),
                user_text_message(format!("Summarize this conversation:\n\n{}", transcript)),
            ];

            let client = build_openai_client(request.api_key, request.endpoint);
            let model = request.model.unwrap_or_else(|| "gpt-4".to_string());

            match create_chat(&client, &model, messages, None).await {
                Ok(summary) => {
                    let summary = summary.trim().to_string();
                    if !summary.is_empty() {
                        if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                            println!("[Session] Skip saving summary because memory is disabled");
                            return;
                        }
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
