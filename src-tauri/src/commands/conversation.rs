use crate::ai::context::AIOrchestrator;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct ConversationInfo {
    pub id: String,
    pub character_id: String,
    pub title: String,
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

#[derive(Deserialize)]
pub struct ListConversationsRequest {
    pub character_id: String,
}

#[tauri::command]
pub async fn list_conversations(
    request: ListConversationsRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Vec<ConversationInfo>, String> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, character_id, title, created_at, updated_at FROM conversations WHERE character_id = ? ORDER BY updated_at DESC"
    )
    .bind(&request.character_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, character_id, title, created_at, updated_at)| ConversationInfo {
            id,
            character_id,
            title,
            created_at,
            updated_at,
        })
        .collect())
}

#[derive(Deserialize)]
pub struct LoadConversationRequest {
    pub id: String,
}

#[tauri::command]
pub async fn load_conversation(
    request: LoadConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Vec<ConversationMessage>, String> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String)>(
        "SELECT role, content, metadata, created_at FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
    )
    .bind(&request.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // 恢复到内存 history
    {
        let mut history = state.history.lock().await;
        history.clear();
        for (role, content, _, _) in &rows {
            history.push_back(crate::ai::context::Message {
                role: role.clone(),
                content: content.clone(),
                metadata: None,
            });
        }
    }

    // 设置当前对话 ID
    {
        let mut conv_id = state.current_conversation_id.lock().await;
        *conv_id = Some(request.id);
    }

    Ok(rows
        .into_iter()
        .map(|(role, content, metadata, created_at)| ConversationMessage {
            role,
            content,
            metadata,
            created_at,
        })
        .collect())
}

#[derive(Deserialize)]
pub struct DeleteConversationRequest {
    pub id: String,
}

#[tauri::command]
pub async fn delete_conversation(
    request: DeleteConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    // 先删消息，再删对话
    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // 如果删除的是当前活跃对话，清空引用
    {
        let mut conv_id = state.current_conversation_id.lock().await;
        if conv_id.as_deref() == Some(&request.id) {
            *conv_id = None;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn create_conversation(
    state: State<'_, AIOrchestrator>,
) -> Result<String, String> {
    // 清空内存 history
    state.clear_history().await;

    // current_conversation_id 已在 clear_history 中被置为 None
    // 返回空字符串表示新对话将在第一条消息时自动创建
    Ok(String::new())
}

#[derive(Deserialize)]
pub struct RenameConversationRequest {
    pub id: String,
    pub title: String,
}

#[tauri::command]
pub async fn rename_conversation(
    request: RenameConversationRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(&request.title)
        .bind(&request.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}
