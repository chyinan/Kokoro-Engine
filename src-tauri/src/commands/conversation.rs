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

#[derive(Serialize)]
pub struct ConversationMessage {
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
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
        "SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM conversations WHERE character_id = ? ORDER BY updated_at DESC",
    )
    .bind(&request.character_id)
    .fetch_all(&state.db)
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

    let rows = sqlx::query_as::<_, (String, String, Option<String>, String)>(
        "SELECT role, content, metadata, created_at FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
    )
    .bind(&request.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| KokoroError::Database(e.to_string()))?;

    {
        let mut history = state.history.lock().await;
        history.clear();
        for (role, content, metadata, _) in &rows {
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
        .filter_map(|(role, content, metadata, created_at)| {
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
