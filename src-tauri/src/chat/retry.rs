use crate::ai::context::{sync_history_window, AIOrchestrator, Message};
use crate::chat::runtime::{ChatRuntimeState, HISTORY_REVISION};
use crate::commands::chat::TURN_CANCELLED_BY_USER_MESSAGE;
use crate::commands::context::is_visible_message_raw;
use crate::error::KokoroError;
use serde_json::Value;
use std::collections::VecDeque;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct FailureRetry {
    pub diagnostic_message_id: i64,
    pub expected_tail_message_id: i64,
}

pub(crate) struct FailureRetryPlan {
    pub user_message_id: i64,
    pub history: VecDeque<Message>,
    revision: i64,
}

fn changed_history() -> KokoroError {
    KokoroError::Validation("Conversation history changed before retry".into())
}

impl FailureRetryPlan {
    /// Resolve the anchor from stored history (or an exact legacy journal entry)
    /// and prepare the prompt against the prefix without mutating live history.
    pub(crate) async fn prepare(
        state: &AIOrchestrator,
        runtime: &ChatRuntimeState,
        retry: &FailureRetry,
        character_id: &str,
        conversation_id: &str,
        message: &str,
    ) -> Result<Self, KokoroError> {
        let mut tx = state.db.begin().await?;
        let revision = sqlx::query_scalar(HISTORY_REVISION)
            .fetch_one(&mut *tx)
            .await?;
        let rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let tail = rows
            .iter()
            .rev()
            .find(|(_, role, _, metadata)| is_visible_message_raw(role, metadata.as_deref()));
        if tail.map(|(id, _, _, _)| *id) != Some(retry.expected_tail_message_id) {
            return Err(changed_history());
        }
        let metadata = rows
            .iter()
            .find(|(id, role, _, _)| *id == retry.diagnostic_message_id && role == "system")
            .and_then(|(_, _, _, metadata)| {
                serde_json::from_str::<Value>(metadata.as_deref()?).ok()
            })
            .ok_or_else(changed_history)?;
        let event = &metadata["event"];
        let turn_id = event["turn_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(changed_history)?;
        if metadata["type"].as_str() != Some("failure_event")
            || event["character_id"].as_str() != Some(character_id)
            || event["conversation_id"].as_str() != Some(conversation_id)
        {
            return Err(changed_history());
        }
        let user_message_id = match metadata.get("user_message_id") {
            Some(id) => id.as_i64(),
            None => runtime.failed_turn_prompt(character_id, conversation_id, turn_id),
        }
        .filter(|id| *id < retry.diagnostic_message_id)
        .ok_or_else(changed_history)?;
        if !rows.iter().any(|(id, role, content, _)| {
            *id == user_message_id && role == "user" && content == message
        }) {
            return Err(changed_history());
        }

        let mut history = VecDeque::new();
        let max_chars = *state.max_message_chars.lock().await;
        sync_history_window(
            &mut history,
            rows.into_iter()
                .take_while(|(id, _, _, _)| *id <= user_message_id)
                .map(|(_, role, content, metadata)| (role, content, metadata)),
            max_chars,
        );
        Ok(Self {
            user_message_id,
            history,
            revision,
        })
    }

    /// The caller holds the chat execution guard from admission through streaming.
    pub(crate) async fn commit(
        self,
        state: &AIOrchestrator,
        conversation_id: &str,
        generation: u64,
        cancellation: Option<&tokio::sync::watch::Receiver<bool>>,
    ) -> Result<(), KokoroError> {
        let _selection = state.conversation_switch_lock.lock().await;
        if state.current_conversation_generation() != generation
            || state.current_conversation_id.lock().await.as_deref() != Some(conversation_id)
        {
            return Err(changed_history());
        }
        let mut history = state.history.lock().await;
        let mut boundary = state.memory_history_boundary.lock().await;
        let mut tx = state.db.begin().await?;
        let revision: i64 = sqlx::query_scalar(HISTORY_REVISION)
            .fetch_one(&mut *tx)
            .await?;
        if revision != self.revision {
            return Err(changed_history());
        }
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ? AND id > ?")
            .bind(conversation_id)
            .bind(self.user_message_id)
            .execute(&mut *tx)
            .await?;
        if cancellation.is_some_and(|signal| *signal.borrow()) {
            return Err(KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.into()));
        }
        tx.commit().await?;
        *boundary = self.history.len();
        *history = self.history;
        Ok(())
    }
}
