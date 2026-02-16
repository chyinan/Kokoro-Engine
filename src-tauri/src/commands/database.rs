use crate::ai::context::AIOrchestrator;
use tauri::State;

#[derive(serde::Serialize)]
pub struct DbTestResult {
    pub success: bool,
    pub message: String,
    pub record_count: usize,
}

#[tauri::command]
pub async fn init_db(_state: State<'_, AIOrchestrator>) -> Result<String, String> {
    // Migration logic could go here, but context::new does basic setup
    // For now, we can clear or re-initialize if needed
    Ok("Database is managed by AI Orchestrator.".to_string())
}

#[tauri::command]
pub async fn test_vector_store(state: State<'_, AIOrchestrator>) -> Result<DbTestResult, String> {
    // 1. Add a test memory
    state
        .memory_manager
        .add_memory("Test memory: Kokoro loves apples.", "test")
        .await
        .map_err(|e| e.to_string())?;

    // 2. Search
    let results = state
        .memory_manager
        .search_memories("What does Kokoro love?", 1, "test")
        .await
        .map_err(|e| e.to_string())?;

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
