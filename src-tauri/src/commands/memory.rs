// pattern: Imperative Shell
use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use crate::ipc::wire::{emit, Response};
use crate::llm::service::LlmService;
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize, specta::Type)]
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

#[derive(serde::Serialize, specta::Type)]
pub struct ListMemoriesResponse {
    pub memories: Vec<crate::ai::memory::MemoryRecord>,
    pub total: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn list_memories(
    request: ListMemoriesRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Response<ListMemoriesResponse>, KokoroError> {
    tracing::info!(
        target: "memory",
        "[Memory] list_memories called for character_id='{}', limit={}, offset={}",
        request.character_id, request.limit, request.offset
    );
    let memories = state
        .memory_manager
        .list_memories(&request.character_id, request.limit, request.offset)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    let total = state
        .memory_manager
        .count_memories(&request.character_id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

    Ok(Response(ListMemoriesResponse { memories, total }))
}

#[derive(Deserialize, specta::Type)]
pub struct UpdateMemoryRequest {
    pub character_id: String,
    pub id: i64,
    pub content: String,
    pub importance: f64,
}

#[tauri::command]
#[specta::specta]
pub async fn update_memory(
    request: UpdateMemoryRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    if request.character_id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character_id must not be empty".to_string(),
        ));
    }
    state
        .memory_manager
        .update_memory(
            request.id,
            &request.character_id,
            &request.content,
            request.importance,
        )
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[derive(Deserialize, specta::Type)]
pub struct DeleteMemoryRequest {
    pub character_id: String,
    pub id: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn delete_memory(
    request: DeleteMemoryRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    if request.character_id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character_id must not be empty".to_string(),
        ));
    }
    state
        .memory_manager
        .delete_memory(request.id, &request.character_id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[derive(Deserialize, specta::Type)]
pub struct UpdateMemoryTierRequest {
    pub character_id: String,
    pub id: i64,
    pub tier: String,
}

#[tauri::command]
#[specta::specta]
pub async fn update_memory_tier(
    request: UpdateMemoryTierRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    if request.character_id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character_id must not be empty".to_string(),
        ));
    }
    if request.tier != "core" && request.tier != "ephemeral" {
        return Err(KokoroError::Validation(
            "tier must be 'core' or 'ephemeral'".to_string(),
        ));
    }
    state
        .memory_manager
        .update_memory_tier(request.id, &request.character_id, &request.tier)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[derive(Deserialize, specta::Type)]
pub struct DreamCharacterRequest {
    pub character_id: String,
}

#[derive(Deserialize, specta::Type)]
pub struct ListDreamJobsRequest {
    pub character_id: String,
    #[serde(default = "default_dream_limit")]
    pub limit: i64,
}

#[derive(Deserialize, specta::Type)]
pub struct ListDreamProposalsRequest {
    pub character_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_dream_limit")]
    pub limit: i64,
}

#[derive(Deserialize, specta::Type)]
pub struct DreamProposalRequest {
    pub id: i64,
}

fn default_dream_limit() -> i64 {
    50
}

#[tauri::command]
#[specta::specta]
pub async fn run_dream_now(
    request: DreamCharacterRequest,
    state: State<'_, AIOrchestrator>,
    llm_state: State<'_, LlmService>,
) -> Result<Response<crate::ai::memory::MemoryDreamRunResult>, KokoroError> {
    let provider = llm_state.system_provider().await;
    let target_language = state.response_language.lock().await.clone();
    state
        .memory_manager
        .run_dream_now_with_provider(
            &request.character_id,
            "manual",
            Some(provider),
            Some(target_language),
        )
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn get_dreaming_summary(
    request: DreamCharacterRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Response<crate::ai::memory::MemoryDreamingSummary>, KokoroError> {
    state
        .memory_manager
        .dreaming_summary(&request.character_id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn list_dream_jobs(
    request: ListDreamJobsRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Response<Vec<crate::ai::memory::MemoryDreamJobRecord>>, KokoroError> {
    state
        .memory_manager
        .list_dream_jobs(&request.character_id, request.limit)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn list_dream_proposals(
    request: ListDreamProposalsRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<Response<Vec<crate::ai::memory::MemoryDreamProposalRecord>>, KokoroError> {
    state
        .memory_manager
        .list_dream_proposals(
            &request.character_id,
            request.status.as_deref(),
            request.limit,
        )
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
        .map(Response)
}

#[tauri::command]
#[specta::specta]
pub async fn approve_dream_proposal(
    request: DreamProposalRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state
        .memory_manager
        .approve_dream_proposal(request.id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn reject_dream_proposal(
    request: DreamProposalRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state
        .memory_manager
        .reject_dream_proposal(request.id)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn get_memory_embedding_model_status(
) -> Result<crate::ai::memory::MemoryEmbeddingModelStatus, KokoroError> {
    Ok(crate::ai::memory::memory_embedding_model_status())
}

#[tauri::command]
#[specta::specta]
pub async fn download_memory_embedding_model(
    app: tauri::AppHandle,
) -> Result<crate::ai::memory::MemoryEmbeddingModelStatus, KokoroError> {

    crate::ai::memory::download_memory_embedding_model(move |progress| {
        emit(&app, progress).map_err(|error| error.to_string())
    })
    .await
    .map_err(KokoroError::Internal)
}

#[cfg(test)]
mod tests {
    use super::{DeleteMemoryRequest, UpdateMemoryRequest, UpdateMemoryTierRequest};

    #[test]
    fn memory_mutation_requests_require_character_id() {
        let update: Result<UpdateMemoryRequest, _> =
            serde_json::from_str(r#"{"id":1,"content":"updated","importance":0.8}"#);
        let delete: Result<DeleteMemoryRequest, _> = serde_json::from_str(r#"{"id":1}"#);
        let tier: Result<UpdateMemoryTierRequest, _> =
            serde_json::from_str(r#"{"id":1,"tier":"core"}"#);

        assert!(update.is_err(), "update must include character_id");
        assert!(delete.is_err(), "delete must include character_id");
        assert!(tier.is_err(), "tier update must include character_id");
    }
}
