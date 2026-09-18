use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use tauri::State;

#[derive(serde::Serialize)]
pub struct DbTestResult {
    pub success: bool,
    pub message: String,
    pub record_count: usize,
}

#[tauri::command]
pub async fn init_db(_state: State<'_, AIOrchestrator>) -> Result<String, KokoroError> {
    // Migration logic could go here, but context::new does basic setup
    // For now, we can clear or re-initialize if needed
    Ok("Database is managed by AI Orchestrator.".to_string())
}

#[tauri::command]
pub async fn test_vector_store(
    state: State<'_, AIOrchestrator>,
) -> Result<DbTestResult, KokoroError> {
    // Memories only exist for a real character instance, so the diagnostic runs
    // against the first one instead of inventing an owner.
    let character_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM characters ORDER BY created_at ASC, id ASC LIMIT 1")
            .fetch_optional(&state.db)
            .await
            .map_err(|e| KokoroError::Database(e.to_string()))?;
    let Some(character_id) = character_id else {
        return Ok(DbTestResult {
            success: false,
            message: "No character exists to own a test memory.".to_string(),
            record_count: 0,
        });
    };

    // 1. Add a test memory
    state
        .memory_manager
        .add_memory("Test memory: Kokoro loves apples.", &character_id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    // 2. Search
    let results = state
        .memory_manager
        .search_memories("What does Kokoro love?", 1, &character_id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    let success = !results.is_empty();
    let message = if success {
        format!("Found: {}", results[0].content)
    } else {
        "No results found".to_string()
    };

    Ok(DbTestResult {
        success,
        message,
        record_count: results.len(),
    })
}
