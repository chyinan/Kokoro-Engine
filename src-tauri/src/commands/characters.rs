use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRecord {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateCharacterRequest {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCharacterRequest {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub updated_at: i64,
}

#[tauri::command]
pub async fn list_characters(
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<Vec<CharacterRecord>, KokoroError> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, i64, i64)>(
        "SELECT id, name, persona, user_nickname, source_format, created_at, updated_at FROM characters ORDER BY created_at ASC"
    )
    .fetch_all(&orchestrator.db)
    .await?;

    Ok(rows.into_iter().map(|(id, name, persona, user_nickname, source_format, created_at, updated_at)| {
        CharacterRecord { id, name, persona, user_nickname, source_format, created_at, updated_at }
    }).collect())
}

#[tauri::command]
pub async fn create_character(
    request: CreateCharacterRequest,
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    sqlx::query(
        "INSERT OR IGNORE INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&request.id)
    .bind(&request.name)
    .bind(&request.persona)
    .bind(&request.user_nickname)
    .bind(&request.source_format)
    .bind(request.created_at)
    .bind(request.updated_at)
    .execute(&orchestrator.db)
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn update_character(
    request: UpdateCharacterRequest,
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    sqlx::query(
        "UPDATE characters SET name = ?, persona = ?, user_nickname = ?, source_format = ?, updated_at = ? WHERE id = ?"
    )
    .bind(&request.name)
    .bind(&request.persona)
    .bind(&request.user_nickname)
    .bind(&request.source_format)
    .bind(request.updated_at)
    .bind(&request.id)
    .execute(&orchestrator.db)
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn delete_character(
    id: String,
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    sqlx::query("DELETE FROM characters WHERE id = ?")
        .bind(&id)
        .execute(&orchestrator.db)
        .await?;
    Ok(())
}
