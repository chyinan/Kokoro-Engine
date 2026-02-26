use crate::ai::context::AIOrchestrator;
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
pub struct ListMemoriesRequest {
    pub character_id: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(serde::Serialize)]
pub struct ListMemoriesResponse {
    pub memories: Vec<crate::ai::memory::MemoryRecord>,
    pub total: i64,
}

#[tauri::command]
pub async fn list_memories(
    request: ListMemoriesRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<ListMemoriesResponse, String> {
    println!(
        "[Memory] list_memories called for character_id='{}', limit={}, offset={}",
        request.character_id, request.limit, request.offset
    );
    let memories = state
        .memory_manager
        .list_memories(&request.character_id, request.limit, request.offset)
        .await
        .map_err(|e| e.to_string())?;

    let total = state
        .memory_manager
        .count_memories(&request.character_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ListMemoriesResponse { memories, total })
}

#[derive(Deserialize)]
pub struct UpdateMemoryRequest {
    pub id: i64,
    pub content: String,
    pub importance: f64,
}

#[tauri::command]
pub async fn update_memory(
    request: UpdateMemoryRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state
        .memory_manager
        .update_memory(request.id, &request.content, request.importance)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct DeleteMemoryRequest {
    pub id: i64,
}

#[tauri::command]
pub async fn delete_memory(
    request: DeleteMemoryRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    state
        .memory_manager
        .delete_memory(request.id)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct UpdateMemoryTierRequest {
    pub id: i64,
    pub tier: String,
}

#[tauri::command]
pub async fn update_memory_tier(
    request: UpdateMemoryTierRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), String> {
    if request.tier != "core" && request.tier != "ephemeral" {
        return Err("tier must be 'core' or 'ephemeral'".to_string());
    }
    state
        .memory_manager
        .update_memory_tier(request.id, &request.tier)
        .await
        .map_err(|e| e.to_string())
}
