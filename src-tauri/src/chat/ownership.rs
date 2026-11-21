//! Explicit request scope and the small amount of state which is not derived
//! from committed SQLite rows. UI selection is deliberately absent here.
use crate::ai::{
    curiosity::CuriosityModule, idle_behaviors::IdleBehaviorSystem, initiative::InitiativeSystem,
};
use crate::error::KokoroError;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TurnIdentity {
    pub character_id: String,
    pub conversation_id: String,
    pub request_id: String,
}

impl TurnIdentity {
    pub fn new(character: &str, conversation: &str, request: &str) -> Result<Self, KokoroError> {
        if [character, conversation, request]
            .iter()
            .any(|id| id.trim().is_empty())
        {
            return Err(KokoroError::Validation(
                "Chat requires character, conversation and request identities".into(),
            ));
        }
        Ok(Self {
            character_id: character.into(),
            conversation_id: conversation.into(),
            request_id: request.into(),
        })
    }
}

/// Immutable after admission. Provider credentials stay in the application
/// service; a turn only retains its selected provider Arc.
#[derive(Clone)]
pub(crate) struct PromptSnapshot {
    /// The implicit webhook default can carry history without owning memory.
    pub skip_memory: bool,
    pub cue_map: Option<std::collections::HashMap<String, crate::live2d::Live2dCueBinding>>,
    pub system_prompt: String,
    pub character_name: String,
    pub user_name: String,
    pub response_language: String,
    pub user_language: String,
    pub jailbreak_prompt: String,
    pub vision_context_history_mode: String,
}

#[derive(Default)]
pub(crate) struct ConversationProgress {
    pub message_count: u64,
    pub memory_count: u64,
    pub memory_epoch: u64,
    pub first_memory_message: Option<i64>,
}

pub(crate) struct CharacterBehavior {
    pub last_activity: Instant,
    pub conversation_count: u64,
    pub last_proactive: Instant,
    pub last_prune: Instant,
    pub last_dream_day: Option<chrono::NaiveDate>,
    pub curiosity: CuriosityModule,
    pub initiative: InitiativeSystem,
    pub idle_behaviors: IdleBehaviorSystem,
}

impl Default for CharacterBehavior {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            conversation_count: 0,
            last_proactive: Instant::now(),
            last_prune: Instant::now(),
            last_dream_day: None,
            curiosity: CuriosityModule::new(),
            initiative: InitiativeSystem::new(),
            idle_behaviors: IdleBehaviorSystem::new(),
        }
    }
}

/// A removed owner stays revoked even if a platform later reuses its SQL ID.
struct OwnerState<T> {
    alive: Arc<AtomicBool>,
    state: Arc<tokio::sync::Mutex<T>>,
}
impl<T: Default> Default for OwnerState<T> {
    fn default() -> Self {
        Self {
            alive: Arc::new(AtomicBool::new(true)),
            state: Arc::default(),
        }
    }
}
impl<T> OwnerState<T> {
    fn revoke(&self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}
type ConversationStates = HashMap<(String, String), OwnerState<ConversationProgress>>;

#[derive(Default)]
pub(crate) struct RuntimeOwners {
    /// Covers only final memory SQL work and deletion/toggle commits, never
    /// embedding or model requests.
    pub memory_commit: Arc<tokio::sync::RwLock<()>>,
    conversations: Mutex<ConversationStates>,
    characters: Mutex<HashMap<String, OwnerState<CharacterBehavior>>>,
}

#[derive(Clone)]
pub(crate) struct OwnerLifetime {
    character: Arc<AtomicBool>,
    conversation: Option<Arc<AtomicBool>>,
}
impl OwnerLifetime {
    fn is_current(&self) -> bool {
        self.character.load(Ordering::SeqCst)
            && self
                .conversation
                .as_ref()
                .is_none_or(|alive| alive.load(Ordering::SeqCst))
    }
}

#[derive(Clone)]
pub(crate) struct MemoryWriteScope {
    pub lifetime: OwnerLifetime,
    pub character_id: String,
    pub conversation_id: Option<String>,
    /// Granted only while admitting a validated, not-yet-materialized target.
    /// The captured lifetime still revokes this permission on deletion/reset.
    pub allow_reserved_conversation: bool,
    pub request_id: String,
    pub epoch: u64,
    pub database_generation: u64,
    pub enabled: Arc<std::sync::atomic::AtomicBool>,
    pub cadence: Arc<crate::chat::external_memory::ExternalMemory>,
    pub owners: Arc<RuntimeOwners>,
    pub db: sqlx::SqlitePool,
    pub shutdown: tokio_util::sync::CancellationToken,
}

impl std::fmt::Debug for MemoryWriteScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryWriteScope")
            .field("character_id", &self.character_id)
            .field("conversation_id", &self.conversation_id)
            .field("request_id", &self.request_id)
            .finish()
    }
}

impl MemoryWriteScope {
    pub fn owner_is_current(&self) -> bool {
        self.lifetime.is_current()
            && self.database_generation == crate::ai::context::database_operation_generation()
    }

    pub fn ensure_owner_current(&self) -> Result<(), KokoroError> {
        if self.owner_is_current() {
            Ok(())
        } else {
            Err(KokoroError::Validation(
                "Chat owner was deleted or replaced".into(),
            ))
        }
    }

    pub async fn admit(&self) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        let guard =
            until_memory_shutdown(Some(self), self.owners.memory_commit.clone().read_owned())
                .await?;
        if !self.owner_is_current()
            || !self.enabled.load(std::sync::atomic::Ordering::SeqCst)
            || self.cadence.epoch() != self.epoch
            || self.database_generation != crate::ai::context::database_operation_generation()
        {
            return None;
        }
        let exists = if let Some(conversation) = &self.conversation_id {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM characters WHERE id=?) AND (EXISTS(SELECT 1 FROM conversations WHERE id=? AND character_id=?) OR (? AND NOT EXISTS(SELECT 1 FROM conversations WHERE id=?)))")
                .bind(&self.character_id).bind(conversation).bind(&self.character_id)
                .bind(self.allow_reserved_conversation).bind(conversation)
                .fetch_one(&self.db).await.unwrap_or(false)
        } else {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM characters WHERE id=?)")
                .bind(&self.character_id)
                .fetch_one(&self.db)
                .await
                .unwrap_or(false)
        };
        (exists && !self.shutdown.is_cancelled()).then_some(guard)
    }
}

pub(crate) async fn until_memory_shutdown<F: std::future::Future>(
    scope: Option<&MemoryWriteScope>,
    future: F,
) -> Option<F::Output> {
    if let Some(scope) = scope {
        tokio::select! {
            biased;
            _ = scope.shutdown.cancelled() => None,
            result = future => Some(result),
        }
    } else {
        Some(future.await)
    }
}

impl RuntimeOwners {
    pub fn conversation(
        &self,
        character: &str,
        conversation: &str,
    ) -> Arc<tokio::sync::Mutex<ConversationProgress>> {
        self.conversations
            .lock()
            .unwrap()
            .entry((character.into(), conversation.into()))
            .or_default()
            .state
            .clone()
    }
    pub fn character(&self, character: &str) -> Arc<tokio::sync::Mutex<CharacterBehavior>> {
        self.characters
            .lock()
            .unwrap()
            .entry(character.into())
            .or_default()
            .state
            .clone()
    }
    pub fn lifetime(&self, character: &str, conversation: Option<&str>) -> OwnerLifetime {
        let character_token = self
            .characters
            .lock()
            .unwrap()
            .entry(character.into())
            .or_default()
            .alive
            .clone();
        let conversation = conversation.map(|conversation| {
            self.conversations
                .lock()
                .unwrap()
                .entry((character.into(), conversation.into()))
                .or_default()
                .alive
                .clone()
        });
        OwnerLifetime {
            character: character_token,
            conversation,
        }
    }
    pub fn remove_conversation(&self, conversation: &str) {
        self.conversations.lock().unwrap().retain(|(_, id), owner| {
            if id == conversation {
                owner.revoke();
                false
            } else {
                true
            }
        });
    }
    pub fn remove_character(&self, character: &str) {
        self.conversations.lock().unwrap().retain(|(id, _), owner| {
            if id == character {
                owner.revoke();
                false
            } else {
                true
            }
        });
        if let Some(owner) = self.characters.lock().unwrap().remove(character) {
            owner.revoke();
        }
    }
    pub fn clear(&self) {
        let mut conversations = self.conversations.lock().unwrap();
        for owner in conversations.values() {
            owner.revoke();
        }
        conversations.clear();
        let mut characters = self.characters.lock().unwrap();
        for owner in characters.values() {
            owner.revoke();
        }
        characters.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::{
            context::{AIOrchestrator, Message},
            memory_extractor::{extract_and_store_memories_with_options, MemoryExtractionOptions},
        },
        llm::provider::{LlmParams, LlmProvider},
    };
    use async_openai::types::chat::ChatCompletionRequestMessage;
    use std::pin::Pin;
    use tokio::sync::Notify;

    #[derive(Default)]
    struct PausedMemory {
        entered: Notify,
        release: Notify,
        response: Option<String>,
    }
    #[async_trait::async_trait]
    impl LlmProvider for PausedMemory {
        async fn chat(
            &self,
            _: Vec<ChatCompletionRequestMessage>,
            _: Option<LlmParams>,
        ) -> Result<String, String> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(self
                .response
                .clone()
                .unwrap_or_else(|| r#"[{"fact":"Owns a blue bicycle","importance":0.8}]"#.into()))
        }
        async fn chat_stream(
            &self,
            _: Vec<ChatCompletionRequestMessage>,
            _: Option<LlmParams>,
        ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
        {
            unreachable!()
        }
        fn id(&self) -> &str {
            "paused-memory"
        }
    }

    #[tokio::test]
    async fn memory_commit_rechecks_ownership_and_revocation_after_provider_response() {
        for mutation in [
            "none",
            "conversation",
            "character",
            "memory-toggle",
            "conversation-recreated",
            "character-recreated",
            "owners-reset",
        ] {
            let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
            sqlx::query("INSERT INTO characters(id,name) VALUES ('owner','Owner')")
                .execute(&state.db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('conversation','owner','now','now')").execute(&state.db).await.unwrap();
            let provider = Arc::new(PausedMemory::default());
            let options = MemoryExtractionOptions {
                scope: Some(state.memory_write_scope(
                    "owner",
                    "conversation",
                    "request",
                    state.external_memory.epoch(),
                )),
                ..Default::default()
            };
            let manager = state.memory_manager.clone();
            let worker_provider = provider.clone();
            let worker = tokio::spawn(async move {
                extract_and_store_memories_with_options(
                    &[Message {
                        role: "user".into(),
                        content: "I own a blue bicycle".into(),
                        metadata: None,
                    }],
                    &manager,
                    worker_provider,
                    "owner".into(),
                    options,
                )
                .await;
            });
            provider.entered.notified().await;
            match mutation {
                "conversation" | "conversation-recreated" => {
                    sqlx::query("DELETE FROM conversations WHERE id='conversation'")
                        .execute(&state.db)
                        .await
                        .unwrap();
                }
                "character" | "character-recreated" => {
                    sqlx::query("DELETE FROM characters WHERE id='owner'")
                        .execute(&state.db)
                        .await
                        .unwrap();
                }
                "memory-toggle" => {
                    state.set_memory_enabled(false).await;
                    state.set_memory_enabled(true).await;
                }
                _ => {}
            }
            if mutation.starts_with("conversation") {
                state.forget_conversation("conversation").await;
            }
            if mutation.starts_with("character") {
                state.forget_character("owner").await;
            }
            if mutation == "owners-reset" {
                state.reset_runtime_owners().await;
            }
            if mutation == "character-recreated" {
                sqlx::query("DELETE FROM conversations WHERE id='conversation'")
                    .execute(&state.db)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO characters(id,name) VALUES ('owner','New owner')")
                    .execute(&state.db)
                    .await
                    .unwrap();
            }
            if mutation.ends_with("recreated") {
                sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('conversation','owner','new','new')").execute(&state.db).await.unwrap();
            }
            provider.release.notify_one();
            worker.await.unwrap();
            let memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
                .fetch_one(&state.db)
                .await
                .unwrap();
            let candidates: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_candidates")
                .fetch_one(&state.db)
                .await
                .unwrap();
            if mutation == "none" {
                assert_eq!(memories, 1, "positive control must actually persist memory");
                assert_eq!(candidates, 1);
            } else {
                assert_eq!(
                    (memories, candidates),
                    (0, 0),
                    "late {mutation} result must not write derived rows"
                );
                if mutation.ends_with("recreated") || mutation == "owners-reset" {
                    // A new admitted task for the reused IDs is still allowed.
                    let provider = Arc::new(PausedMemory::default());
                    provider.release.notify_one();
                    extract_and_store_memories_with_options(
                        &[Message {
                            role: "user".into(),
                            content: "I own a blue bicycle".into(),
                            metadata: None,
                        }],
                        &state.memory_manager,
                        provider,
                        "owner".into(),
                        MemoryExtractionOptions {
                            scope: Some(state.memory_write_scope(
                                "owner",
                                "conversation",
                                "new-request",
                                state.external_memory.epoch(),
                            )),
                            ..Default::default()
                        },
                    )
                    .await;
                    let rows: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM memories), (SELECT COUNT(*) FROM memory_candidates)").fetch_one(&state.db).await.unwrap();
                    assert_eq!(
                        rows,
                        (1, 1),
                        "new lifetime must accept its own result: {mutation}"
                    );
                }
            }
        }
    }
    #[tokio::test]
    async fn queued_scope_keeps_revoked_owner_before_worker_starts() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        sqlx::query("INSERT INTO characters(id,name) VALUES ('owner','Owner')")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('conversation','owner','now','now')").execute(&state.db).await.unwrap();
        let queued = state.memory_write_scope(
            "owner",
            "conversation",
            "old",
            state.external_memory.epoch(),
        );
        state.forget_character("owner").await;
        let replacement = state.memory_write_scope(
            "owner",
            "conversation",
            "new",
            state.external_memory.epoch(),
        );
        assert!(replacement.admit().await.is_some());
        let provider = Arc::new(PausedMemory::default());
        provider.release.notify_one();
        extract_and_store_memories_with_options(
            &[Message {
                role: "user".into(),
                content: "I own a blue bicycle".into(),
                metadata: None,
            }],
            &state.memory_manager,
            provider,
            "owner".into(),
            MemoryExtractionOptions {
                scope: Some(queued),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM memories")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn consolidation_revalidates_after_paused_model_before_proposal_write() {
        for mutation in ["none", "conversation", "memory-toggle"] {
            let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
            sqlx::query("INSERT INTO characters(id,name) VALUES ('owner','Owner')")
                .execute(&state.db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('conversation','owner','now','now')").execute(&state.db).await.unwrap();
            for (id, content, vector) in [
                (1, "Likes climbing", vec![1.0f32, 0.0]),
                (2, "Works remotely", vec![0.0f32, 1.0]),
            ] {
                sqlx::query("INSERT INTO memories(id,content,character_id,embedding,created_at,importance) VALUES (?,?,'owner',?,1,0.8)").bind(id).bind(content).bind(bincode::serialize(&vector).unwrap()).execute(&state.db).await.unwrap();
            }
            let provider = Arc::new(PausedMemory { response:Some(r#"{"proposals":[{"source_memory_ids":[1,2],"decision":"update","confidence":0.95,"proposed_content":"Works remotely and enjoys climbing"}]}"#.into()),..Default::default() });
            let scope = state.memory_write_scope(
                "owner",
                "conversation",
                "consolidation",
                state.external_memory.epoch(),
            );
            let worker_provider = provider.clone();
            let manager = state.memory_manager.clone();
            let worker = tokio::spawn(async move {
                manager
                    .consolidate_chat_memories("owner", worker_provider, None, &scope)
                    .await
            });
            provider.entered.notified().await;
            match mutation {
                "conversation" => {
                    sqlx::query("DELETE FROM conversations WHERE id='conversation'")
                        .execute(&state.db)
                        .await
                        .unwrap();
                }
                "memory-toggle" => {
                    state.set_memory_enabled(false).await;
                    state.set_memory_enabled(true).await;
                }
                _ => {}
            }
            provider.release.notify_one();
            let result = worker.await.unwrap();
            let proposals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_dream_proposals")
                .fetch_one(&state.db)
                .await
                .unwrap();
            if mutation == "none" {
                assert!(result.is_ok());
                assert_eq!(proposals, 1);
            } else {
                assert!(result.is_err());
                assert_eq!(proposals, 0);
            }
            let active: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE status='active'")
                    .fetch_one(&state.db)
                    .await
                    .unwrap();
            assert_eq!(active, 2);
        }
    }
}
