// pattern: Mixed (unavoidable)
// Reason: Tauri command 文件天然承担 IPC 输入校验、状态编排与磁盘持久化副作用；Phase 1 仅在现有命令边界上低侵入扩展。
use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::{build_openai_client, create_chat};
use tauri::State;

pub use crate::config::MemoryUpgradeConfig;

fn memory_upgrade_config_path() -> std::path::PathBuf {
    crate::ai::memory::memory_upgrade_config_path()
}

#[tauri::command]
pub async fn set_memory_upgrade_config(
    config: MemoryUpgradeConfig,
) -> Result<(), KokoroError> {
    crate::config::save_memory_upgrade_config(&memory_upgrade_config_path(), &config)
}

#[tauri::command]
pub async fn get_memory_upgrade_config() -> Result<MemoryUpgradeConfig, KokoroError> {
    Ok(crate::config::load_memory_upgrade_config(
        &memory_upgrade_config_path(),
    ))
}

#[tauri::command]
pub async fn get_memory_observability_summary(
    state: State<'_, AIOrchestrator>,
) -> Result<crate::ai::memory::MemoryObservabilitySummary, KokoroError> {
    state
        .memory_manager
        .memory_observability_summary()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
pub async fn get_latest_memory_write_event(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryWriteEventRecord>, KokoroError> {
    state
        .memory_manager
        .latest_memory_write_event()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
pub async fn get_latest_memory_retrieval_log(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryRetrievalLogRecord>, KokoroError> {
    state
        .memory_manager
        .latest_memory_retrieval_log()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_upgrade_config_roundtrip_uses_shared_path_rules() {
        let path = memory_upgrade_config_path();
        assert!(path.ends_with("com.chyin.kokoro/memory_upgrade_config.json") || path.ends_with("com.chyin.kokoro\\memory_upgrade_config.json"));
    }
}


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

#[tauri::command]
pub async fn set_persona(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_system_prompt(prompt).await;
    Ok(())
}

#[tauri::command]
pub async fn set_character_name(
    name: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_character_name(name).await;
    Ok(())
}

#[tauri::command]
pub async fn set_active_character_id(
    id: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_character_id(id.clone()).await;
    crate::ai::context::AIOrchestrator::persist_active_character_id(&id);
    Ok(())
}

#[tauri::command]
pub async fn set_user_name(
    name: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
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
pub async fn get_jailbreak_prompt(state: State<'_, AIOrchestrator>) -> Result<String, KokoroError> {
    Ok(state.get_jailbreak_prompt().await)
}

#[tauri::command]
pub async fn set_proactive_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_proactive_enabled(enabled);
    tracing::info!(
        target: "ai",
        "Proactive messages {}",
        if enabled { "enabled" } else { "disabled" }
    );

    // Persist to disk
    let app_data = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let path = app_data.join("proactive_enabled.json");
    let _ = std::fs::write(&path, serde_json::json!({ "enabled": enabled }).to_string());
    Ok(())
}

#[tauri::command]
pub async fn get_proactive_enabled(state: State<'_, AIOrchestrator>) -> Result<bool, KokoroError> {
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
pub async fn get_memory_enabled(state: State<'_, AIOrchestrator>) -> Result<bool, KokoroError> {
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
    tracing::info!(
        target: "ai",
        "Deleted last {} message(s) from history (now {} messages)",
        to_remove,
        history.len()
    );

    // 从数据库末尾删除，直到删够 to_remove 条「可见」消息为止。
    // 一条可见消息可能对应多行 DB（assistant_tool_calls + tool_result + assistant），
    // 需要跳过不可见行继续计数，否则重启后残留行会重新显示。
    let conv_id = state.current_conversation_id.lock().await.clone();
    if let Some(conversation_id) = conv_id {
        // 从末尾倒序读取所有行（id + metadata）
        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id DESC"
        )
        .bind(&conversation_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        // 收集需要删除的行 ID，跳过不可见行时不计入 visible_deleted
        let mut ids_to_delete: Vec<i64> = Vec::new();
        let mut visible_deleted = 0usize;
        for (id, metadata) in &rows {
            if visible_deleted >= to_remove {
                break;
            }
            ids_to_delete.push(*id);
            // 判断是否为不可见的技术行（assistant_tool_calls 或 tool/tool_result）
            let technical_type = metadata
                .as_deref()
                .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                .and_then(|v| {
                    v.get("type")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                });
            let is_invisible = matches!(
                technical_type.as_deref(),
                Some("assistant_tool_calls") | Some("tool_result")
            );
            if !is_invisible {
                visible_deleted += 1;
            }
        }

        if !ids_to_delete.is_empty() {
            // 用事务保证原子性，避免崩溃导致部分删除
            let mut tx = state
                .db
                .begin()
                .await
                .map_err(|e| KokoroError::Database(e.to_string()))?;
            for id in &ids_to_delete {
                sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| KokoroError::Database(e.to_string()))?;
            }
            tx.commit()
                .await
                .map_err(|e| KokoroError::Database(e.to_string()))?;
            tracing::info!(
                target: "ai",
                "Deleted {} DB row(s) for {} visible message(s)",
                ids_to_delete.len(),
                visible_deleted
            );
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
                            tracing::info!(target: "ai", "Skip saving summary because memory is disabled");
                            return;
                        }
                        if let Err(e) = memory_mgr.save_session_summary(&char_id, &summary).await {
                            tracing::error!(target: "ai", "Failed to save summary: {}", e);
                        } else {
                            tracing::info!(
                                target: "ai",
                                "Saved summary for '{}': {}",
                                char_id,
                                &summary[..summary.len().min(80)]
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(target: "ai", "Summary generation failed: {}", e);
                }
            }
        });
    }

    Ok(())
}
