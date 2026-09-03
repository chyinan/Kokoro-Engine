use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::{build_openai_client, create_chat};
use tauri::{AppHandle, Manager, State};

pub use crate::config::MemoryUpgradeConfig;

fn memory_upgrade_config_path() -> std::path::PathBuf {
    crate::ai::memory::memory_upgrade_config_path()
}

const USER_PROFILE_SETTINGS_FILE: &str = "user_profile.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserProfileSettings {
    pub user_name: String,
    pub user_persona: String,
}

impl Default for UserProfileSettings {
    fn default() -> Self {
        Self {
            user_name: "User".to_string(),
            user_persona: String::new(),
        }
    }
}

fn app_data_dir(app: &AppHandle) -> Result<std::path::PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Failed to resolve app data dir: {}", e)))
}

fn user_profile_settings_path(app_data: &std::path::Path) -> std::path::PathBuf {
    app_data.join(USER_PROFILE_SETTINGS_FILE)
}

pub fn load_user_profile_settings_from_app_data(
    app_data: &std::path::Path,
) -> Option<UserProfileSettings> {
    let path = user_profile_settings_path(app_data);
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_user_profile_settings_to_app_data(
    app_data: &std::path::Path,
    settings: &UserProfileSettings,
) -> Result<(), KokoroError> {
    std::fs::create_dir_all(app_data).map_err(KokoroError::from)?;
    let path = user_profile_settings_path(app_data);
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| KokoroError::Config(format!("Serialize error: {}", e)))?;
    std::fs::write(path, json).map_err(KokoroError::from)
}

fn update_user_profile_settings<F>(app: &AppHandle, update: F) -> Result<(), KokoroError>
where
    F: FnOnce(&mut UserProfileSettings),
{
    let app_data = app_data_dir(app)?;
    let mut settings = load_user_profile_settings_from_app_data(&app_data).unwrap_or_default();
    update(&mut settings);
    save_user_profile_settings_to_app_data(&app_data, &settings)
}

#[tauri::command]
pub async fn set_memory_upgrade_config(config: MemoryUpgradeConfig) -> Result<(), KokoroError> {
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

#[tauri::command]
pub async fn get_latest_memory_retrieval_eval_summary(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryRetrievalEvalSummary>, KokoroError> {
    state
        .memory_manager
        .latest_memory_retrieval_eval_summary()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_upgrade_config_roundtrip_uses_shared_path_rules() {
        let path = memory_upgrade_config_path();
        assert!(
            path.ends_with("com.chyin.kokoro/memory_upgrade_config.json")
                || path.ends_with("com.chyin.kokoro\\memory_upgrade_config.json")
        );
    }

    #[tokio::test]
    async fn jailbreak_prompt_recovers_from_backup_when_target_corrupted() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let jailbreak_path = temp_dir.path().join("jailbreak_prompt.json");
        let backup_path = temp_dir.path().join(".jailbreak_prompt.json.backup");

        // 1. Write corrupted content to target
        std::fs::write(&jailbreak_path, "INVALID_CORRUPTED_JSON{{{{").expect("write corrupted");

        // 2. Write valid prompt to backup
        let valid_json = serde_json::to_string(&crate::config::JailbreakConfig {
            prompt: "system prompt to be recovered".to_string(),
        })
        .unwrap();
        std::fs::write(&backup_path, valid_json).expect("write backup");

        // 3. Create orchestrator
        let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();

        // 4. Perform load / startup recovery
        let prompt_opt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(
            prompt_opt,
            Some("system prompt to be recovered".to_string())
        );

        if let Some(ref prompt) = prompt_opt {
            orchestrator.set_jailbreak_prompt(prompt.clone()).await;
        }

        // Verify MEMORY is recovered
        assert_eq!(
            orchestrator.get_jailbreak_prompt().await,
            Some("system prompt to be recovered".to_string())
        );

        // Verify DISK target is recovered and parseable
        let disk_content = std::fs::read_to_string(&jailbreak_path).expect("read restored target");
        let disk_val: crate::config::JailbreakConfig =
            serde_json::from_str(&disk_content).expect("parse restored target");
        assert_eq!(disk_val.prompt, "system prompt to be recovered");

        // Verify NEXT STARTUP recovers cleanly from the now-restored target (even if backup is removed)
        std::fs::remove_file(&backup_path).expect("remove backup");
        let next_startup_prompt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(
            next_startup_prompt,
            Some("system prompt to be recovered".to_string())
        );
    }

    #[test]
    fn jailbreak_prompt_recovers_from_backup_when_target_missing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let jailbreak_path = temp_dir.path().join("jailbreak_prompt.json");
        let backup_path = temp_dir.path().join(".jailbreak_prompt.json.backup");

        let valid_json = serde_json::to_string(&crate::config::JailbreakConfig {
            prompt: "backup only prompt".to_string(),
        })
        .unwrap();
        std::fs::write(&backup_path, valid_json).expect("write backup");

        assert!(!jailbreak_path.exists());
        let prompt_opt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(prompt_opt, Some("backup only prompt".to_string()));
        assert!(jailbreak_path.exists());
    }

    async fn setup_test_context_db() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePool::connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                character_id TEXT NOT NULL,
                title TEXT NOT NULL,
                topic TEXT NOT NULL DEFAULT '',
                pinned_state TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE conversation_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[test]
    fn test_is_visible_message_classification() {
        // Invisible cases
        assert!(!is_visible_message_raw("tool", None));
        assert!(!is_visible_message_raw(
            "tool",
            Some(r#"{"type":"tool_result"}"#)
        ));
        assert!(!is_visible_message_raw(
            "assistant",
            Some(r#"{"type":"assistant_tool_calls"}"#)
        ));
        assert!(!is_visible_message_raw(
            "assistant",
            Some(r#"{"type":"translation_instruction"}"#)
        ));

        // Visible cases
        assert!(is_visible_message_raw("user", None));
        assert!(is_visible_message_raw("user", Some(r#"{"images":[]}"#)));
        assert!(is_visible_message_raw("assistant", None));
        assert!(is_visible_message_raw(
            "assistant",
            Some(r#"{"turn_id":"t-1","translation":"hello"}"#)
        ));
        assert!(is_visible_message_raw(
            "context",
            Some(r#"{"type":"vision_context"}"#)
        ));
    }

    #[tokio::test]
    async fn test_delete_last_messages_cleans_assistant_tool_calls_and_results() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-1', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 1. User message (id 1)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'user', 'What is the weather?', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 2. Technical tool call message (id 2)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'assistant', 'call weather', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 3. Technical tool result message (id 3)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'tool', '{\"temp\": 25}', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 4. Final assistant answer (id 4)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'assistant', 'It is 25C.', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-1".to_string())));

        // Call delete_last_messages_inner(1) to delete the final assistant response
        delete_last_messages_inner(1, &pool, &history, &current_conv, 2000, None)
            .await
            .unwrap();

        // Ensure rows 2, 3, 4 are completely deleted, and only id 1 remains
        let remaining_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-1' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids, vec![1], "Orphan assistant_tool_calls and tool_result rows must be deleted along with the assistant message");

        // Ensure history synchronization contains only the user message
        let hist = history.lock().await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "What is the weather?");
    }

    #[tokio::test]
    async fn test_delete_last_messages_preserves_earlier_turns_tool_calls() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-2', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Turn 1
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'user', 'Turn 1 User', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 1 ToolCall', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'tool', 'Turn 1 Result', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 1 Answer', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();

        // Turn 2
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'user', 'Turn 2 User', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 2 ToolCall', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'tool', 'Turn 2 Result', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 2 Answer', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-2".to_string())));

        // Delete 1 visible message (Turn 2 Answer)
        delete_last_messages_inner(1, &pool, &history, &current_conv, 2000, None)
            .await
            .unwrap();

        // Should keep ids 1..=5 (Turn 1 completely intact + Turn 2 User), delete ids 6, 7, 8
        let remaining_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-2' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids, vec![1, 2, 3, 4, 5]);

        // Delete another 2 visible messages (Turn 2 User [5] and Turn 1 Answer [4])
        delete_last_messages_inner(2, &pool, &history, &current_conv, 2000, None)
            .await
            .unwrap();

        // Only Turn 1 User (id 1) should remain, all subsequent tool rows and answers removed
        let remaining_ids2: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-2' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids2, vec![1]);
    }

    #[tokio::test]
    async fn test_delete_last_messages_in_memory_mode() {
        let pool = setup_test_context_db().await;
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(None)); // In-memory mode

        {
            let mut hist = history.lock().await;
            hist.push_back(crate::ai::context::Message {
                role: "user".to_string(),
                content: "User prompt".to_string(),
                metadata: None,
            });
            hist.push_back(crate::ai::context::Message {
                role: "assistant".to_string(),
                content: "calling".to_string(),
                metadata: Some(serde_json::json!({"type": "assistant_tool_calls"})),
            });
            hist.push_back(crate::ai::context::Message {
                role: "tool".to_string(),
                content: "result".to_string(),
                metadata: Some(serde_json::json!({"type": "tool_result"})),
            });
            hist.push_back(crate::ai::context::Message {
                role: "assistant".to_string(),
                content: "Final answer".to_string(),
                metadata: None,
            });
        }

        // Delete last 1 visible message
        delete_last_messages_inner(1, &pool, &history, &current_conv, 2000, None)
            .await
            .unwrap();

        let hist = history.lock().await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "User prompt");
    }

    #[tokio::test]
    async fn test_delete_last_messages_enforces_20_limit_and_truncation() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-long', 'char-1', 'Long Conv', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Insert 35 messages (17 user-assistant pairs + 1 extra user)
        for i in 0..35 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            let content = if i == 30 {
                // An extra long message to verify truncation
                "L".repeat(100)
            } else {
                format!("Message {i}")
            };
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-long', ?, ?, NULL, ?)")
                .bind(role)
                .bind(&content)
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-long".to_string())));
        let memory_boundary = Arc::new(Mutex::new(0));

        // Delete the last visible message (message 34, leaving 34 messages: 0..34)
        delete_last_messages_inner(1, &pool, &history, &current_conv, 40, Some(&memory_boundary))
            .await
            .unwrap();

        // Check DB row count: 34 rows remain
        let total_db_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = 'conv-long'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(total_db_rows, 34);

        // Check in-memory history: must NOT have 34 messages! Must be strictly capped at 20!
        let hist = history.lock().await;
        assert_eq!(hist.len(), 20, "Memory history must enforce 20 message window limit");
        // Messages remaining in DB are 0..34. The last 20 are indices 14..34.
        assert_eq!(hist[0].content, "Message 14");
        // Message 30 should be present in history and truncated to 40 chars
        let msg_30 = hist.iter().find(|m| m.content.starts_with("LLLL")).expect("Message 30 should be in history");
        assert!(msg_30.content.ends_with("…[truncated]"));
        assert_eq!(msg_30.content, format!("{}…[truncated]", "L".repeat(40)));

        // Memory boundary should be synchronized to 20
        assert_eq!(*memory_boundary.lock().await, 20);
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
pub async fn get_user_profile_settings(
    app: AppHandle,
) -> Result<Option<UserProfileSettings>, KokoroError> {
    let app_data = app_data_dir(&app)?;
    Ok(load_user_profile_settings_from_app_data(&app_data))
}

#[tauri::command]
pub async fn set_user_name(
    name: String,
    app: AppHandle,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_user_name(name.clone()).await;
    update_user_profile_settings(&app, |settings| {
        settings.user_name = name.clone();
    })?;
    Ok(())
}

#[tauri::command]
pub async fn set_user_persona(persona: String, app: AppHandle) -> Result<(), KokoroError> {
    update_user_profile_settings(&app, |settings| {
        settings.user_persona = persona;
    })?;
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

pub(crate) fn jailbreak_prompt_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("jailbreak_prompt.json")
}

#[tauri::command]
pub async fn set_jailbreak_prompt(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    // Persist to disk first to avoid split-brain if disk write fails
    let path = jailbreak_prompt_path();
    crate::config::save_json_config(
        &path,
        &crate::config::JailbreakConfig {
            prompt: prompt.clone(),
        },
        "JAILBREAK",
    )?;

    state.set_jailbreak_prompt(prompt).await;
    Ok(())
}

#[tauri::command]
pub async fn get_jailbreak_prompt(state: State<'_, AIOrchestrator>) -> Result<String, KokoroError> {
    if let Some(memory_prompt) = state.get_jailbreak_prompt().await {
        return Ok(memory_prompt);
    }

    // Fallback read from disk with backup recovery
    let path = jailbreak_prompt_path();
    let prompt = crate::config::load_jailbreak_prompt(&path).unwrap_or_default();
    if !prompt.is_empty() {
        state.set_jailbreak_prompt(prompt.clone()).await;
    }
    Ok(prompt)
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

/// 判断消息是否为前端/用户视角的可见消息。
/// 不可见技术消息包括：
/// 1. `role == "tool"`
/// 2. `metadata.type` 为 `"assistant_tool_calls" | "tool_result" | "translation_instruction"`
pub fn is_visible_message_meta(role: &str, meta: Option<&serde_json::Value>) -> bool {
    if role == "tool" {
        return false;
    }
    let technical_type = meta
        .and_then(|v| v.get("type"))
        .and_then(|t| t.as_str());
    !matches!(
        technical_type,
        Some("assistant_tool_calls") | Some("translation_instruction") | Some("tool_result")
    )
}

pub fn is_visible_message_raw(role: &str, metadata: Option<&str>) -> bool {
    if role == "tool" {
        return false;
    }
    let meta_val = metadata
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    is_visible_message_meta(role, meta_val.as_ref())
}

pub async fn delete_last_messages_inner(
    count: usize,
    db: &sqlx::SqlitePool,
    history: &Arc<Mutex<VecDeque<crate::ai::context::Message>>>,
    current_conversation_id: &Arc<Mutex<Option<String>>>,
    max_chars: usize,
    memory_history_boundary: Option<&Arc<Mutex<usize>>>,
) -> Result<(), KokoroError> {
    if count == 0 {
        return Ok(());
    }

    let conv_id = current_conversation_id.lock().await.clone();
    if let Some(ref conversation_id) = conv_id {
        // 读取该会话的所有行（id, role, metadata），按 id 升序排列
        let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT id, role, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        // 收集所有可见消息在 rows 中的索引
        let visible_indices: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, role, metadata))| is_visible_message_raw(role, metadata.as_deref()))
            .map(|(idx, _)| idx)
            .collect();

        let total_visible = visible_indices.len();
        let ids_to_delete: Vec<i64> = if count >= total_visible {
            // 如果要删除的可见条数 >= 当前总可见条数，则删除该会话的所有消息（包括不可见技术行）
            rows.iter().map(|(id, _, _)| *id).collect()
        } else {
            // 保留前 total_visible - count 条可见消息
            let keep_visible_count = total_visible - count;
            let last_keep_idx = visible_indices[keep_visible_count - 1];
            // 从最后一条保留的可见消息之后的所有行（包括伴生的 assistant_tool_calls / tool_result 等不可见技术行与待删可见消息）全部删除
            rows[last_keep_idx + 1..].iter().map(|(id, _, _)| *id).collect()
        };

        if !ids_to_delete.is_empty() {
            // 用事务保证原子性，避免崩溃导致部分删除
            let mut tx = db
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
                count.min(total_visible)
            );
        }

        // 用权威数据库的剩余行重新同步 history，彻底杜绝孤儿技术行（assistant_tool_calls/tool_result）残留，并应用统一的 20 条滑动窗口与字符截断
        let remaining_rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        let mut hist = history.lock().await;
        let count = crate::ai::context::sync_history_window(&mut hist, remaining_rows, max_chars);
        if let Some(boundary_lock) = memory_history_boundary {
            *boundary_lock.lock().await = count;
        }
    } else {
        // 纯内存模式同步：同样按可见消息边界截断
        let mut hist = history.lock().await;
        let visible_indices: Vec<usize> = hist
            .iter()
            .enumerate()
            .filter(|(_, msg)| is_visible_message_meta(&msg.role, msg.metadata.as_ref()))
            .map(|(idx, _)| idx)
            .collect();
        let total_visible = visible_indices.len();
        if count >= total_visible {
            hist.clear();
        } else {
            let keep_visible_count = total_visible - count;
            let last_keep_idx = visible_indices[keep_visible_count - 1];
            hist.truncate(last_keep_idx + 1);
        }
        let len = hist.len();
        if let Some(boundary_lock) = memory_history_boundary {
            let mut boundary = boundary_lock.lock().await;
            if *boundary > len {
                *boundary = len;
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_last_messages(
    count: usize,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let max_chars = *state.max_message_chars.lock().await;
    delete_last_messages_inner(
        count,
        &state.db,
        &state.history,
        &state.current_conversation_id,
        max_chars,
        Some(&state.memory_history_boundary),
    )
    .await
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
    let summary_language = state.response_language.lock().await.clone();

    // Clear history immediately so the user can start fresh
    state.clear_history().await;

    // Generate session summary in the background
    if history.len() >= 2 {
        tauri::async_runtime::spawn(async move {
            let transcript = history
                .iter()
                .filter(|m| crate::ai::context::is_summary_candidate_message(m))
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");
            let language_rule = if summary_language.trim().is_empty() {
                "Write the summary in the language the users were speaking.".to_string()
            } else {
                let language = summary_language.trim();
                format!(
                    "Write the summary in {language}. If the conversation uses another language, translate or summarize it into {language}."
                )
            };

            let messages = vec![
                system_message(format!(
                    "You are a conversation summarizer. Write a brief 2-3 sentence summary of this conversation. \
                     {language_rule} Focus on key topics discussed, any emotional moments, and important \
                     information shared. Write from a third-person perspective.\n\
                     Output ONLY the summary, no labels or formatting."
                )),
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
