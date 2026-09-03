use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct ConversationInfo {
    pub id: String,
    pub character_id: String,
    pub title: String,
    pub topic: String,
    pub pinned_state: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct LoadedConversation {
    pub topic: String,
    pub pinned_state: String,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Deserialize)]
pub struct ListConversationsRequest {
    pub character_id: String,
}

#[derive(Deserialize)]
pub struct LoadConversationRequest {
    pub id: String,
}

#[derive(Deserialize)]
pub struct DeleteConversationRequest {
    pub id: String,
}

#[derive(Deserialize)]
pub struct RenameConversationRequest {
    pub id: String,
    pub title: String,
}

#[derive(Deserialize)]
pub struct UpdateConversationStateRequest {
    pub id: String,
    pub topic: Option<String>,
    pub pinned_state: Option<String>,
}

#[tauri::command]
pub async fn list_conversations(
    request: ListConversationsRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Vec<ConversationInfo>, KokoroError> {
    list_conversations_inner(request, &state.db).await
}

pub fn is_pinned_conversation_state(pinned_state: &str) -> bool {
    let normalized = pinned_state.trim();
    if normalized.is_empty() || normalized == "{}" {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(normalized) {
        Ok(serde_json::Value::Object(map)) => {
            map.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false)
        }
        _ => false,
    }
}

pub(crate) async fn list_conversations_inner(
    request: ListConversationsRequest,
    db: &sqlx::SqlitePool,
) -> Result<Vec<ConversationInfo>, KokoroError> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
        "SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM conversations WHERE character_id = ? ORDER BY updated_at DESC",
    )
    .bind(&request.character_id)
    .fetch_all(db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    let mut list: Vec<ConversationInfo> = rows
        .into_iter()
        .map(
            |(id, character_id, title, topic, pinned_state, created_at, updated_at)| {
                ConversationInfo {
                    id,
                    character_id,
                    title,
                    topic,
                    pinned_state,
                    created_at,
                    updated_at,
                }
            },
        )
        .collect();

    // 采用与前端 hasPinnedConversationState 完全相同的统一置顶判断标准进行稳定复合排序 (pinned DESC -> updated_at DESC)
    list.sort_by(|a, b| {
        let a_pinned = is_pinned_conversation_state(&a.pinned_state);
        let b_pinned = is_pinned_conversation_state(&b.pinned_state);
        b_pinned
            .cmp(&a_pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    Ok(list)
}

#[tauri::command]
pub async fn load_conversation(
    request: LoadConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<LoadedConversation, KokoroError> {
    let conversation_row = sqlx::query_as::<_, (String, String)>(
        "SELECT topic, pinned_state FROM conversations WHERE id = ?",
    )
    .bind(&request.id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, String)>(
        "SELECT id, role, content, metadata, created_at FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
    )
    .bind(&request.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    {
        let max_chars = *state.max_message_chars.lock().await;
        let history_messages: Vec<(String, String, Option<String>)> = rows
            .iter()
            .map(|(_, role, content, metadata, _)| {
                (role.clone(), content.clone(), metadata.clone())
            })
            .collect();
        let mut history = state.history.lock().await;
        crate::ai::context::sync_history_window(&mut history, history_messages, max_chars);
    }

    {
        let mut conv_id = state.current_conversation_id.lock().await;
        *conv_id = Some(request.id.clone());
        crate::ai::context::AIOrchestrator::persist_conversation_id(Some(&request.id));
    }

    let messages = rows
        .into_iter()
        .filter_map(|(id, role, content, metadata, created_at)| {
            let metadata_value = metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let technical_type = metadata_value
                .as_ref()
                .and_then(|meta| meta.get("type"))
                .and_then(|value| value.as_str());
            if matches!(
                technical_type,
                Some("assistant_tool_calls") | Some("translation_instruction")
            ) {
                return None;
            }
            Some(ConversationMessage {
                id,
                role,
                content,
                metadata,
                created_at,
            })
        })
        .collect();

    Ok(LoadedConversation {
        topic: conversation_row.0,
        pinned_state: conversation_row.1,
        messages,
    })
}

#[tauri::command]
pub async fn delete_conversation(
    request: DeleteConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    {
        let mut conv_id = state.current_conversation_id.lock().await;
        if conv_id.as_deref() == Some(&request.id) {
            *conv_id = None;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn list_character_ids(
    state: State<'_, AIOrchestrator>,
) -> Result<Vec<String>, KokoroError> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT character_id FROM conversations
         UNION
         SELECT DISTINCT character_id FROM memories
         ORDER BY character_id ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[tauri::command]
pub async fn create_conversation(state: State<'_, AIOrchestrator>) -> Result<String, KokoroError> {
    state.clear_history().await;
    Ok(String::new())
}

#[tauri::command]
pub async fn rename_conversation(
    request: RenameConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(&request.title)
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    Ok(())
}

#[tauri::command]
pub async fn update_conversation_state(
    request: UpdateConversationStateRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let current = sqlx::query_as::<_, (String, String)>(
        "SELECT topic, pinned_state FROM conversations WHERE id = ?",
    )
    .bind(&request.id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    let topic = request.topic.unwrap_or(current.0);
    let pinned_state = request.pinned_state.unwrap_or(current.1);
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE conversations SET topic = ?, pinned_state = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&topic)
    .bind(&pinned_state)
    .bind(&now)
    .bind(&request.id)
    .execute(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    Ok(())
}

#[derive(Deserialize, Debug, Clone)]
pub struct EditConversationMessageRequest {
    pub conversation_id: Option<String>,
    pub message_id: Option<i64>,
    pub visible_index: Option<usize>,
    pub new_content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EditConversationMessageResponse {
    pub message_id: i64,
    pub updated_content: String,
}

/// Edits one persisted message after resolving and validating its authoritative conversation.
///
/// The message content and owning conversation timestamp are updated atomically. The in-memory
/// history is refreshed only when the database-owned conversation is still active.
pub async fn edit_conversation_message_inner(
    request: EditConversationMessageRequest,
    db: &sqlx::SqlitePool,
    history_lock: &tokio::sync::Mutex<std::collections::VecDeque<crate::ai::context::Message>>,
    current_conversation_id_lock: &tokio::sync::Mutex<Option<String>>,
    max_message_chars: usize,
) -> Result<EditConversationMessageResponse, KokoroError> {
    let trimmed = request.new_content.trim();
    if trimmed.is_empty() {
        return Err(KokoroError::Validation(
            "Message content cannot be empty".to_string(),
        ));
    }
    let content_to_persist =
        crate::ai::context::truncate_message_content(trimmed.to_string(), max_message_chars);

    // Resolve message ownership and perform both database writes in one transaction. The
    // conversation stored on the message row is authoritative; a caller-provided ID is only a
    // constraint and must never redirect timestamp or history updates.
    let mut transaction = db
        .begin()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    let (target_id, authoritative_conversation_id) = if let Some(id) = request.message_id {
        let actual_conversation_id = sqlx::query_scalar::<_, String>(
            "SELECT conversation_id FROM conversation_messages WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?
        .ok_or_else(|| KokoroError::NotFound(format!("Message with id {} not found", id)))?;

        if let Some(requested_conversation_id) = request.conversation_id.as_deref() {
            if requested_conversation_id != actual_conversation_id.as_str() {
                return Err(KokoroError::Validation(format!(
                    "Message with id {} does not belong to conversation {}",
                    id, requested_conversation_id
                )));
            }
        }

        (id, actual_conversation_id)
    } else if let (Some(conv_id), Some(index)) =
        (request.conversation_id.as_deref(), request.visible_index)
    {
        let rows = sqlx::query_as::<_, (i64, String, Option<String>)>(
            "SELECT id, role, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conv_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        let visible_rows: Vec<i64> = rows
            .into_iter()
            .filter(|(_, role, meta)| {
                if role == "tool" {
                    return false;
                }
                let technical_type = meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|v| {
                        v.get("type")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    });
                !matches!(
                    technical_type.as_deref(),
                    Some("assistant_tool_calls")
                        | Some("translation_instruction")
                        | Some("tool_result")
                )
            })
            .map(|(id, _, _)| id)
            .collect();

        let message_id = *visible_rows.get(index).ok_or_else(|| {
            KokoroError::NotFound(format!(
                "Message at index {} not found in conversation {}",
                index, conv_id
            ))
        })?;

        (message_id, conv_id.to_string())
    } else {
        return Err(KokoroError::Validation(
            "Either message_id or (conversation_id, visible_index) must be provided".to_string(),
        ));
    };

    let message_update = sqlx::query(
        "UPDATE conversation_messages SET content = ? WHERE id = ? AND conversation_id = ?",
    )
    .bind(&content_to_persist)
    .bind(target_id)
    .bind(&authoritative_conversation_id)
    .execute(&mut *transaction)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    if message_update.rows_affected() != 1 {
        return Err(KokoroError::NotFound(format!(
            "Message with id {} not found in conversation {}",
            target_id, authoritative_conversation_id
        )));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let conversation_update = sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&authoritative_conversation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    if conversation_update.rows_affected() != 1 {
        return Err(KokoroError::NotFound(format!(
            "Conversation {} not found",
            authoritative_conversation_id
        )));
    }

    transaction
        .commit()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    // Refresh only the authoritative active conversation. Recheck immediately before replacing
    // history so a conversation switch during the database query cannot apply a stale snapshot.
    let should_refresh_history = current_conversation_id_lock.lock().await.as_deref()
        == Some(authoritative_conversation_id.as_str());
    if should_refresh_history {
        let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(&authoritative_conversation_id)
        .fetch_all(db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        let active_conversation_id = current_conversation_id_lock.lock().await;
        if active_conversation_id.as_deref() == Some(authoritative_conversation_id.as_str()) {
            let mut history = history_lock.lock().await;
            crate::ai::context::sync_history_window(&mut history, rows, max_message_chars);
        }
    }

    Ok(EditConversationMessageResponse {
        message_id: target_id,
        updated_content: content_to_persist,
    })
}

#[tauri::command]
pub async fn edit_conversation_message(
    request: EditConversationMessageRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<EditConversationMessageResponse, KokoroError> {
    let max_chars = *state.max_message_chars.lock().await;
    edit_conversation_message_inner(
        request,
        &state.db,
        &state.history,
        &state.current_conversation_id,
        max_chars,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
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

    #[tokio::test]
    async fn test_edit_conversation_message_by_id_and_history_sync() {
        let pool = setup_test_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-1', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'user', 'original message', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-1".to_string())));

        // Populate initial history
        history.lock().await.push_back(crate::ai::context::Message {
            role: "user".to_string(),
            content: "original message".to_string(),
            metadata: None,
        });

        // Edit via message_id
        let res = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-1".to_string()),
                message_id: Some(1),
                visible_index: None,
                new_content: "edited message content".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 1);
        assert_eq!(res.updated_content, "edited message content");

        // Verify SQLite was updated
        let (content,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content, "edited message content");

        // Verify active history was synced
        let synced = history.lock().await;
        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].content, "edited message content");
    }

    /// Verifies that contradictory message and conversation identifiers cannot mutate either
    /// conversation or replace the active conversation history.
    #[tokio::test]
    async fn test_edit_conversation_message_rejects_cross_conversation_ownership() {
        let pool = setup_test_db().await;
        let conversation_a_updated_at = "2026-01-01T00:00:00Z";
        let conversation_b_updated_at = "2026-01-02T00:00:00Z";

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-a', 'char-1', 'A', ?, ?)")
            .bind(conversation_a_updated_at)
            .bind(conversation_a_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-b', 'char-1', 'B', ?, ?)")
            .bind(conversation_b_updated_at)
            .bind(conversation_b_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-a', 'user', 'message-a', NULL, ?)")
            .bind(conversation_a_updated_at)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::from([crate::ai::context::Message {
            role: "user".to_string(),
            content: "active-message-b".to_string(),
            metadata: None,
        }])));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-b".to_string())));

        let error = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-b".to_string()),
                message_id: Some(1),
                visible_index: None,
                new_content: "must-not-be-written".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, KokoroError::Validation(_)));

        let stored_content: String =
            sqlx::query_scalar("SELECT content FROM conversation_messages WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_content, "message-a");

        let updated_times: Vec<(String, String)> =
            sqlx::query_as("SELECT id, updated_at FROM conversations ORDER BY id ASC")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            updated_times,
            vec![
                ("conv-a".to_string(), conversation_a_updated_at.to_string()),
                ("conv-b".to_string(), conversation_b_updated_at.to_string()),
            ]
        );

        let unchanged_history = history.lock().await;
        assert_eq!(unchanged_history.len(), 1);
        assert_eq!(unchanged_history[0].content, "active-message-b");
    }

    /// Verifies that a message-only edit derives its owning conversation from the database for
    /// timestamp and active-history synchronization.
    #[tokio::test]
    async fn test_edit_conversation_message_by_id_uses_authoritative_conversation() {
        let pool = setup_test_db().await;
        let conversation_a_updated_at = "2026-01-01T00:00:00Z";
        let conversation_b_updated_at = "2026-01-02T00:00:00Z";

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-a', 'char-1', 'A', ?, ?)")
            .bind(conversation_a_updated_at)
            .bind(conversation_a_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-b', 'char-1', 'B', ?, ?)")
            .bind(conversation_b_updated_at)
            .bind(conversation_b_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-a', 'user', 'message-a', NULL, ?)")
            .bind(conversation_a_updated_at)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::from([crate::ai::context::Message {
            role: "user".to_string(),
            content: "message-a".to_string(),
            metadata: None,
        }])));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-a".to_string())));

        let response = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: None,
                message_id: Some(1),
                visible_index: None,
                new_content: "edited-message-a".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap();

        assert_eq!(response.message_id, 1);
        assert_eq!(response.updated_content, "edited-message-a");

        let updated_times: Vec<(String, String)> =
            sqlx::query_as("SELECT id, updated_at FROM conversations ORDER BY id ASC")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_ne!(updated_times[0].1, conversation_a_updated_at);
        assert_eq!(updated_times[1].1, conversation_b_updated_at);

        let synced_history = history.lock().await;
        assert_eq!(synced_history.len(), 1);
        assert_eq!(synced_history[0].content, "edited-message-a");
    }

    /// Verifies that a conversation timestamp failure rolls back the preceding message update.
    #[tokio::test]
    async fn test_edit_conversation_message_rolls_back_when_timestamp_update_fails() {
        let pool = setup_test_db().await;
        let original_updated_at = "2026-01-01T00:00:00Z";

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-a', 'char-1', 'A', ?, ?)")
            .bind(original_updated_at)
            .bind(original_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-a', 'user', 'original-message', NULL, ?)")
            .bind(original_updated_at)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_conversation_timestamp_update
             BEFORE UPDATE OF updated_at ON conversations
             WHEN OLD.id = 'conv-a'
             BEGIN
                 SELECT RAISE(ABORT, 'forced timestamp failure');
             END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::from([crate::ai::context::Message {
            role: "user".to_string(),
            content: "original-message".to_string(),
            metadata: None,
        }])));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-a".to_string())));

        let error = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-a".to_string()),
                message_id: Some(1),
                visible_index: None,
                new_content: "must-roll-back".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, KokoroError::Database(_)));

        let persisted_state: (String, String) = sqlx::query_as(
            "SELECT conversation_messages.content, conversations.updated_at
             FROM conversation_messages
             JOIN conversations ON conversations.id = conversation_messages.conversation_id
             WHERE conversation_messages.id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            persisted_state,
            (
                "original-message".to_string(),
                original_updated_at.to_string()
            )
        );

        let unchanged_history = history.lock().await;
        assert_eq!(unchanged_history[0].content, "original-message");
    }

    #[tokio::test]
    async fn test_edit_conversation_message_by_visible_index() {
        let pool = setup_test_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-2', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Insert message 1: user
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'user', 'question', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Insert hidden technical message: should be skipped from visible index
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'calls', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Insert message 2: assistant
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'answer', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv_id = Arc::new(Mutex::new(None));

        // Edit visible index 1 (which corresponds to id=3, skipping id=2 technical row)
        let res = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-2".to_string()),
                message_id: None,
                visible_index: Some(1),
                new_content: "new answer content".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 3);
        assert_eq!(res.updated_content, "new answer content");

        let (content,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 3")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content, "new answer content");
    }

    #[tokio::test]
    async fn test_edit_conversation_message_with_interleaved_tools_projection() {
        let pool = setup_test_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-tools', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 1. User message (visible index 0, id 1)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'user', 'Search web for weather', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 2. Technical tool calls (hidden, id 2)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'assistant', 'call tool', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 3. Tool result (folded into assistant on frontend, role='tool', id 3)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'tool', '{\"temperature\": 25}', '{\"type\":\"tool_result\",\"tool\":\"weather\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 4. Assistant answer (visible index 1, id 4)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'assistant', 'The weather is 25C sunny.', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 5. Follow-up user message (visible index 2, id 5)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tools', 'user', 'Thanks for the weather info!', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-tools".to_string())));

        // Editing visible_index 2 should target id 5 ('Thanks for the weather info!'), NOT id 3 (tool) or id 4 (assistant)
        let res = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-tools".to_string()),
                message_id: None,
                visible_index: Some(2),
                new_content: "Thanks! What about tomorrow?".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 5, "visible_index 2 must resolve to id 5, skipping both assistant_tool_calls and tool_result");
        assert_eq!(res.updated_content, "Thanks! What about tomorrow?");

        let (content_5,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 5")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content_5, "Thanks! What about tomorrow?");

        // Verify id 3 (tool) and id 4 (assistant) were untouched
        let (content_3,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 3")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content_3, "{\"temperature\": 25}");

        let (content_4,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 4")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content_4, "The weather is 25C sunny.");

        // Also verify direct message_id edit on id 5
        let res_by_id = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-tools".to_string()),
                message_id: Some(5),
                visible_index: None,
                new_content: "Direct message_id edit on user question".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap();

        assert_eq!(res_by_id.message_id, 5);
        assert_eq!(
            res_by_id.updated_content,
            "Direct message_id edit on user question"
        );
    }

    #[tokio::test]
    async fn test_edit_conversation_message_validation_and_errors() {
        let pool = setup_test_db().await;
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv_id = Arc::new(Mutex::new(None));

        // Empty content error
        let err = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-1".to_string()),
                message_id: Some(1),
                visible_index: None,
                new_content: "   ".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, KokoroError::Validation(_)));

        // Missing identifiers error
        let err2 = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: None,
                message_id: None,
                visible_index: None,
                new_content: "valid text".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap_err();

        assert!(matches!(err2, KokoroError::Validation(_)));

        // A valid message locator that does not exist must preserve NotFound semantics.
        let err3 = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: None,
                message_id: Some(999),
                visible_index: None,
                new_content: "valid text".to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            2000,
        )
        .await
        .unwrap_err();

        assert!(matches!(err3, KokoroError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_edit_conversation_message_truncates_overlong_content() {
        let pool = setup_test_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-trunc', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-trunc', 'user', 'short text', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv_id = Arc::new(Mutex::new(Some("conv-trunc".to_string())));

        // Set max_message_chars to 20, but provide 50 chars
        let long_text = "12345678901234567890EXTRA_TEXT_EXCEEDING_LIMIT_HERE!";
        let res = edit_conversation_message_inner(
            EditConversationMessageRequest {
                conversation_id: Some("conv-trunc".to_string()),
                message_id: Some(1),
                visible_index: None,
                new_content: long_text.to_string(),
            },
            &pool,
            &history,
            &current_conv_id,
            20,
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 1);
        assert_eq!(res.updated_content, "12345678901234567890…[truncated]");

        // Verify SQLite has the truncated text
        let (content,): (String,) =
            sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content, "12345678901234567890…[truncated]");

        // Verify history has the truncated text
        let synced = history.lock().await;
        assert_eq!(synced[0].content, "12345678901234567890…[truncated]");
    }

    #[tokio::test]
    async fn test_list_conversations_pinned_priority_and_updated_at() {
        let pool = setup_test_db().await;

        // conv-1: unpinned, older (updated 2026-01-01)
        sqlx::query("INSERT INTO conversations (id, character_id, title, pinned_state, created_at, updated_at) VALUES ('conv-1', 'char-1', 'First', '', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        // conv-2: unpinned, newer (updated 2026-01-03)
        sqlx::query("INSERT INTO conversations (id, character_id, title, pinned_state, created_at, updated_at) VALUES ('conv-2', 'char-1', 'Second', '{}', '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        // conv-3: pinned, middle (updated 2026-01-02)
        sqlx::query("INSERT INTO conversations (id, character_id, title, pinned_state, created_at, updated_at) VALUES ('conv-3', 'char-1', 'Pinned', '{\"pinned\":true}', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        // conv-4: unpinned with explicit {"pinned":false}, newest updated_at (2026-01-05)
        sqlx::query("INSERT INTO conversations (id, character_id, title, pinned_state, created_at, updated_at) VALUES ('conv-4', 'char-1', 'FalsePinned', '{\"pinned\":false}', '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        // conv-5: custom topic JSON without pinned field (2026-01-04)
        sqlx::query("INSERT INTO conversations (id, character_id, title, pinned_state, created_at, updated_at) VALUES ('conv-5', 'char-1', 'TopicOnly', '{\"topic\":\"science\"}', '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        let list = list_conversations_inner(
            ListConversationsRequest {
                character_id: "char-1".to_string(),
            },
            &pool,
        )
        .await
        .unwrap();

        assert_eq!(list.len(), 5);
        // conv-3 is the ONLY truly pinned conversation, so it must be first
        assert_eq!(list[0].id, "conv-3");
        // Remaining 4 are unpinned, ordered by updated_at DESC: conv-4 (Jan 5) -> conv-5 (Jan 4) -> conv-2 (Jan 3) -> conv-1 (Jan 1)
        assert_eq!(list[1].id, "conv-4");
        assert_eq!(list[2].id, "conv-5");
        assert_eq!(list[3].id, "conv-2");
        assert_eq!(list[4].id, "conv-1");
    }

    #[test]
    fn test_is_pinned_conversation_state_matches_frontend_contract() {
        assert!(!is_pinned_conversation_state(""));
        assert!(!is_pinned_conversation_state("{}"));
        assert!(!is_pinned_conversation_state("   "));
        assert!(!is_pinned_conversation_state("not-json"));
        assert!(!is_pinned_conversation_state("{invalid"));
        assert!(!is_pinned_conversation_state("{\"topic\": \"science\"}"));
        assert!(!is_pinned_conversation_state("{\"pinned\": false}"));
        assert!(!is_pinned_conversation_state("{\"pinned\": 0}"));
        assert!(!is_pinned_conversation_state("{\"pinned\": null}"));
        assert!(!is_pinned_conversation_state("{\"pinned\": \"true\"}"));

        assert!(is_pinned_conversation_state("{\"pinned\": true}"));
        assert!(is_pinned_conversation_state(
            "{\"pinned\": true, \"pinned_at\": \"2026-01-01T00:00:00Z\"}"
        ));
    }
}
