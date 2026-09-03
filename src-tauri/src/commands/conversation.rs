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

pub(crate) async fn list_conversations_inner(
    request: ListConversationsRequest,
    db: &sqlx::SqlitePool,
) -> Result<Vec<ConversationInfo>, KokoroError> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
        "SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM conversations WHERE character_id = ? ORDER BY CASE WHEN pinned_state != '' AND pinned_state != '{}' THEN 1 ELSE 0 END DESC, updated_at DESC",
    )
    .bind(&request.character_id)
    .fetch_all(db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    Ok(rows
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
        .collect())
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
        let mut history = state.history.lock().await;
        history.clear();
        for (_, role, content, metadata, _) in &rows {
            history.push_back(crate::ai::context::Message {
                role: role.clone(),
                content: content.clone(),
                metadata: metadata
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
            });
        }
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

pub async fn edit_conversation_message_inner(
    request: EditConversationMessageRequest,
    db: &sqlx::SqlitePool,
    history_lock: &tokio::sync::Mutex<std::collections::VecDeque<crate::ai::context::Message>>,
    current_conversation_id_lock: &tokio::sync::Mutex<Option<String>>,
) -> Result<EditConversationMessageResponse, KokoroError> {
    let trimmed = request.new_content.trim();
    if trimmed.is_empty() {
        return Err(KokoroError::Validation(
            "Message content cannot be empty".to_string(),
        ));
    }

    // Resolve target message_id
    let target_id = if let Some(id) = request.message_id {
        id
    } else if let (Some(conv_id), Some(index)) = (&request.conversation_id, request.visible_index) {
        let rows = sqlx::query_as::<_, (i64, Option<String>)>(
            "SELECT id, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conv_id)
        .fetch_all(db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        let visible_rows: Vec<i64> = rows
            .into_iter()
            .filter(|(_, meta)| {
                let technical_type = meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()));
                !matches!(
                    technical_type.as_deref(),
                    Some("assistant_tool_calls") | Some("translation_instruction")
                )
            })
            .map(|(id, _)| id)
            .collect();

        *visible_rows.get(index).ok_or_else(|| {
            KokoroError::NotFound(format!(
                "Message at index {} not found in conversation {}",
                index, conv_id
            ))
        })?
    } else {
        return Err(KokoroError::Validation(
            "Either message_id or (conversation_id, visible_index) must be provided".to_string(),
        ));
    };

    // 1. Update database row
    let update_res = sqlx::query("UPDATE conversation_messages SET content = ? WHERE id = ?")
        .bind(trimmed)
        .bind(target_id)
        .execute(db)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    if update_res.rows_affected() == 0 {
        return Err(KokoroError::NotFound(format!(
            "Message with id {} not found",
            target_id
        )));
    }

    // 2. Resolve associated conversation_id to update updated_at
    let conv_id_opt = if let Some(ref cid) = request.conversation_id {
        Some(cid.clone())
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT conversation_id FROM conversation_messages WHERE id = ?",
        )
        .bind(target_id)
        .fetch_optional(db)
        .await
        .unwrap_or(None)
    };

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ref conv_id) = conv_id_opt {
        let _ = sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(conv_id)
            .execute(db)
            .await;
    }

    // 3. If editing active conversation, sync state.history
    let active_conv_id = current_conversation_id_lock.lock().await.clone();
    if let Some(active_id) = active_conv_id {
        if conv_id_opt.as_deref() == Some(&active_id) {
            let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
                "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
            )
            .bind(&active_id)
            .fetch_all(db)
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;

            let mut history = history_lock.lock().await;
            history.clear();
            for (role, content, metadata) in rows {
                history.push_back(crate::ai::context::Message {
                    role,
                    content,
                    metadata: metadata
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
                });
            }
        }
    }

    Ok(EditConversationMessageResponse {
        message_id: target_id,
        updated_content: trimmed.to_string(),
    })
}

#[tauri::command]
pub async fn edit_conversation_message(
    request: EditConversationMessageRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<EditConversationMessageResponse, KokoroError> {
    edit_conversation_message_inner(
        request,
        &state.db,
        &state.history,
        &state.current_conversation_id,
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
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 1);
        assert_eq!(res.updated_content, "edited message content");

        // Verify SQLite was updated
        let (content,): (String,) = sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(content, "edited message content");

        // Verify active history was synced
        let synced = history.lock().await;
        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].content, "edited message content");
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
        )
        .await
        .unwrap();

        assert_eq!(res.message_id, 3);
        assert_eq!(res.updated_content, "new answer content");

        let (content,): (String,) = sqlx::query_as("SELECT content FROM conversation_messages WHERE id = 3")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(content, "new answer content");
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
        )
        .await
        .unwrap_err();

        assert!(matches!(err2, KokoroError::Validation(_)));
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

        let list = list_conversations_inner(
            ListConversationsRequest {
                character_id: "char-1".to_string(),
            },
            &pool,
        )
        .await
        .unwrap();

        assert_eq!(list.len(), 3);
        // conv-3 is pinned, so it must be first
        assert_eq!(list[0].id, "conv-3");
        // Between conv-2 and conv-1 (both unpinned), conv-2 has newer updated_at
        assert_eq!(list[1].id, "conv-2");
        assert_eq!(list[2].id, "conv-1");
    }
}
