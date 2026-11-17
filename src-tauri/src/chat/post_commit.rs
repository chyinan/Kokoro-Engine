//! Work admitted by a committed desktop reply. A failed save retains this exact
//! context; replay never rereads a newly selected character's provider or history.
use super::events::{ChatEvent, ChatEvents};
use crate::ai::context::{database_operation_generation, AIOrchestrator, Message};
use crate::ai::memory_event_ingress::{build_cooldown_key, MemoryIngressDecision};
use crate::ai::memory_extractor::{self, MemoryExtractionOptions};
use crate::imagegen::ImageGenService;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::LlmProvider;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct DesktopMemoryWork {
    pub epoch: u64,
    pub count: u64,
    pub history: Vec<Message>,
    pub language: String,
    pub event: Option<MemoryIngressDecision>,
    pub structured: bool,
}

#[derive(Clone)]
pub(crate) struct DesktopBackgroundWork {
    pub reply: String,
    pub size: (u32, u32),
    pub service: ImageGenService,
}

#[derive(Clone)]
pub(crate) struct DesktopPostCommit {
    pub scope: Option<Arc<super::ownership::MemoryWriteScope>>,
    pub request_id: String,
    pub turn_id: String,
    pub character_id: String,
    pub conversation_id: Option<String>,
    pub database_generation: u64,
    pub provider: Arc<dyn LlmProvider>,
    pub memory: Option<DesktopMemoryWork>,
    pub background: Option<DesktopBackgroundWork>,
}

impl DesktopPostCommit {
    pub(crate) fn byte_len(&self) -> usize {
        self.character_id.len()
            + self.conversation_id.as_deref().map_or(0, str::len)
            + self.memory.as_ref().map_or(0, |memory| {
                memory.language.len()
                    + memory
                        .history
                        .iter()
                        .map(|message| {
                            message.content.len()
                                + message
                                    .metadata
                                    .as_ref()
                                    .map_or(0, |value| value.to_string().len())
                        })
                        .sum::<usize>()
            })
            + self
                .background
                .as_ref()
                .map_or(0, |background| background.reply.len())
    }

    async fn is_current(&self, state: &AIOrchestrator) -> bool {
        if self.database_generation != database_operation_generation()
            || !self
                .scope
                .as_ref()
                .is_some_and(|scope| scope.owner_is_current())
        {
            return false;
        }
        let Some(conversation) = &self.conversation_id else {
            return false;
        };
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM conversations c JOIN characters ch ON ch.id=c.character_id WHERE c.id=? AND c.character_id=?)")
            .bind(conversation).bind(&self.character_id).fetch_one(&state.db).await.unwrap_or(false)
    }

    fn memory_current(&self, state: &AIOrchestrator) -> bool {
        state.is_memory_enabled()
            && self
                .memory
                .as_ref()
                .is_some_and(|work| state.external_memory.epoch() == work.epoch)
    }

    /// Called once by the original worker, or after consuming its pending save.
    /// All best-effort work stays outside the persistence/selection locks.
    pub(crate) fn dispatch(self, state: Arc<AIOrchestrator>, events: ChatEvents) {
        if self.memory.is_some() {
            let mut memory = self.clone();
            memory.background = None;
            memory.spawn(state.clone(), events.clone());
        }
        if self.background.is_some() {
            let mut background = self;
            background.memory = None;
            background.spawn(state, events);
        }
    }

    fn spawn(self, state: Arc<AIOrchestrator>, events: ChatEvents) {
        tokio::spawn(async move {
            // Memory operations acquire their own database guards. Holding one
            // here across a nested acquisition can deadlock a queued restore.
            if !self.is_current(&state).await {
                return;
            }
            if let Some(work) = self.memory.as_ref().filter(|_| self.memory_current(&state)) {
                let mut extractions = Vec::new();
                if let (Some(event), Some(conversation)) = (&work.event, &self.conversation_id) {
                    let key = build_cooldown_key(
                        &self.character_id,
                        conversation,
                        event.event.event_type,
                    );
                    if state
                        .should_trigger_memory_event(&key, event.event.cooldown_secs)
                        .await
                    {
                        extractions.push((event.trigger_label, work.structured));
                    }
                }
                if work.count > 0 && work.count.is_multiple_of(5) {
                    extractions.push(("periodic", false));
                }
                for (trigger, structured) in extractions {
                    let task = self.clone();
                    let state = state.clone();
                    tokio::spawn(async move {
                        let work = task.memory.as_ref().expect("memory task");
                        if !task.memory_current(&state) || !task.is_current(&state).await {
                            return;
                        }
                        let started = std::time::Instant::now();
                        if trigger == "periodic" {
                            let _ = state
                                .memory_manager
                                .periodic_write_observation_for_chat(&task.character_id, started)
                                .await;
                        } else {
                            let _ = state
                                .memory_manager
                                .record_periodic_write_if_enabled(
                                    &task.character_id,
                                    "chat",
                                    trigger,
                                    started,
                                )
                                .await;
                        }
                        memory_extractor::extract_and_store_memories_with_options(
                            &work.history,
                            &state.memory_manager,
                            task.provider.clone(),
                            task.character_id.clone(),
                            MemoryExtractionOptions {
                                scope: task.scope.clone(),
                                structured_memory_enabled: structured,
                                target_language: Some(work.language.clone()),
                            },
                        )
                        .await;
                    });
                }
                if work.count > 0
                    && work.count.is_multiple_of(20)
                    && self.memory_current(&state)
                    && self.is_current(&state).await
                {
                    let task = self.clone();
                    let state = state.clone();
                    tokio::spawn(async move {
                        let work = task.memory.as_ref().expect("memory task");
                        if !task.memory_current(&state) || !task.is_current(&state).await {
                            return;
                        }
                        let _ = state
                            .memory_manager
                            .periodic_consolidation_observation(
                                &task.character_id,
                                "chat",
                                std::time::Instant::now(),
                            )
                            .await;
                        if let Err(error) = state
                            .memory_manager
                            .consolidate_chat_memories(
                                &task.character_id,
                                task.provider.clone(),
                                Some(work.language.clone()),
                                task.scope.as_ref().expect("admitted memory scope"),
                            )
                            .await
                        {
                            tracing::warn!(target:"memory", "Deferred consolidation failed: {error}");
                        }
                    });
                }
            }
            if let Some(background) = self
                .background
                .as_ref()
                .filter(|work| !work.reply.is_empty())
            {
                if !self.is_current(&state).await {
                    return;
                }
                let result = self
                    .provider
                    .chat(
                        vec![
                            system_message(crate::ai::prompts::BG_IMAGE_ANALYZER_PROMPT.to_owned()),
                            user_text_message(format!("Character reply: {}", background.reply)),
                        ],
                        None,
                    )
                    .await;
                let text = match result {
                    Ok(text) => text,
                    Err(error) => {
                        tracing::warn!(target:"imagegen", "Background analysis failed: {error}");
                        return;
                    }
                };
                #[derive(serde::Deserialize)]
                struct Analysis {
                    should_generate: bool,
                    image_prompt: Option<String>,
                }
                let clean = text
                    .trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```");
                let Ok(analysis) = serde_json::from_str::<Analysis>(clean) else {
                    return;
                };
                if !analysis.should_generate || !self.is_current(&state).await {
                    return;
                }
                let Some(prompt) = analysis.image_prompt.filter(|text| !text.is_empty()) else {
                    return;
                };
                match background
                    .service
                    .generate(prompt, None, None, Some(background.size))
                    .await
                {
                    Ok(result) if self.is_current(&state).await => {
                        let mut event = serde_json::to_value(result).unwrap_or_default();
                        event["character_id"] = self.character_id.clone().into();
                        event["conversation_id"] = self.conversation_id.clone().into();
                        event["client_request_id"] = self.request_id.clone().into();
                        event["turn_id"] = self.turn_id.clone().into();
                        events.notify(ChatEvent::ImageGenerated, event);
                    }
                    Err(error) => {
                        events.notify(ChatEvent::ImageGenerationError, serde_json::json!({"character_id":self.character_id,"conversation_id":self.conversation_id,"client_request_id":self.request_id,"turn_id":self.turn_id,"message":error.to_string()}));
                    }
                    _ => {}
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deferred_scope_fences_deletion_and_revocation_but_not_selection() {
        let state = AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .clone();
        let character = state.get_character_id().await;
        sqlx::query("INSERT OR IGNORE INTO characters(id,name,persona,user_nickname,source_format,created_at,updated_at) VALUES (?,'owner','','','test',1,1)").bind(&character).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('owned',?,'now','now')").bind(&character).execute(&state.db).await.unwrap();
        let mut work = DesktopPostCommit {
            scope: Some(state.memory_write_scope(
                &character,
                "owned",
                "request",
                state.external_memory.epoch(),
            )),
            request_id: "request".into(),
            turn_id: "turn".into(),
            character_id: state.get_character_id().await,
            conversation_id: Some("owned".into()),
            database_generation: database_operation_generation(),
            provider: Arc::new(crate::llm::provider::OpenAIProvider::new(
                "fixture".into(),
                Some("http://127.0.0.1:9".into()),
                None,
            )),
            memory: Some(DesktopMemoryWork {
                epoch: state.external_memory.epoch(),
                count: 5,
                history: vec![],
                language: "en".into(),
                event: None,
                structured: false,
            }),
            background: None,
        };
        assert!(work.is_current(&state).await);
        assert!(work.memory_current(&state));
        state.set_memory_enabled(false).await;
        state.set_memory_enabled(true).await;
        assert!(
            !work.memory_current(&state),
            "re-enabling must not revive old extraction"
        );
        work.database_generation = database_operation_generation().wrapping_add(1);
        assert!(!work.is_current(&state).await);
        work.database_generation = database_operation_generation();
        work.character_id = "different-character".into();
        assert!(!work.is_current(&state).await);
        work.character_id = state.get_character_id().await;
        state.clear_history().await;
        assert!(
            work.is_current(&state).await,
            "selection is not task authority"
        );
        sqlx::query("DELETE FROM conversations WHERE id='owned'")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(!work.is_current(&state).await);
    }
}
