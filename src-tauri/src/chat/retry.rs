use crate::ai::context::{sync_history_window, AIOrchestrator, Message};
use crate::chat::runtime::ChatRuntimeState;
use crate::commands::chat::TURN_CANCELLED_BY_USER_MESSAGE;
use crate::commands::context::is_visible_message_raw;
use crate::error::KokoroError;
use serde_json::Value;
use sqlx::SqliteConnection;
use std::collections::VecDeque;
use tokio::sync::watch;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct FailureRetry {
    pub diagnostic_message_id: i64,
    pub expected_tail_message_id: i64,
    pub expected_history_revision: Option<i64>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct ReplyRegeneration {
    pub assistant_message_id: i64,
    pub expected_tail_message_id: i64,
    pub expected_history_revision: i64,
}

pub(crate) enum Regeneration<'a> {
    Failure(&'a FailureRetry),
    Reply(&'a ReplyRegeneration),
}

pub(crate) struct RegenerationPlan {
    pub user_message_id: i64,
    pub keep_through_message_id: i64,
    history: VecDeque<Message>,
    revision: i64,
}

pub(super) async fn conversation_message_revision(
    connection: &mut SqliteConnection,
    conversation_id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT message_revision FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .fetch_optional(connection)
        .await
}

fn changed_history() -> KokoroError {
    KokoroError::Validation("Conversation history changed before retry".into())
}

type HistoryRow = (i64, String, String, Option<String>);

impl RegenerationPlan {
    /// Prepare the prompt against the prefix while leaving the saved reply in place.
    pub(crate) async fn prepare(
        state: &AIOrchestrator,
        runtime: &ChatRuntimeState,
        regeneration: Regeneration<'_>,
        character_id: &str,
        conversation_id: &str,
        message: &str,
    ) -> Result<Self, KokoroError> {
        let (expected_tail, expected_revision) = match regeneration {
            Regeneration::Failure(retry) => (
                retry.expected_tail_message_id,
                retry.expected_history_revision,
            ),
            Regeneration::Reply(reply) => (
                reply.expected_tail_message_id,
                Some(reply.expected_history_revision),
            ),
        };
        let mut tx = state.db.begin().await?;
        let revision = conversation_message_revision(&mut tx, conversation_id)
            .await?
            .filter(|revision| expected_revision.is_none_or(|expected| expected == *revision))
            .ok_or_else(changed_history)?;
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT id, role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let (user_message_id, keep_through_message_id) = {
            let mut visible = rows
                .iter()
                .rev()
                .filter(|(_, role, _, metadata)| is_visible_message_raw(role, metadata.as_deref()))
                .peekable();
            if visible.peek().map(|(id, _, _, _)| *id) != Some(expected_tail) {
                return Err(changed_history());
            }

            let (user, keep) = match regeneration {
                Regeneration::Failure(retry) => {
                    Self::failure_boundary(&rows, runtime, retry, character_id, conversation_id)?
                }
                Regeneration::Reply(reply) => {
                    if !visible
                        .find(|(id, _, _, _)| *id == reply.assistant_message_id)
                        .is_some_and(|(_, role, _, _)| role == "assistant")
                    {
                        return Err(changed_history());
                    }
                    // Match the visible cutoff used by history deletion. Technical
                    // rows between the cutoff and the answer belong to the removed suffix.
                    let keep = visible.next().ok_or_else(changed_history)?;
                    let prompt = std::iter::once(keep)
                        .chain(visible)
                        .find(|(_, role, _, _)| role == "user")
                        .ok_or_else(changed_history)?;
                    (prompt.0, keep.0)
                }
            };
            if !rows
                .iter()
                .any(|(id, role, content, _)| *id == user && role == "user" && content == message)
            {
                return Err(changed_history());
            }
            (user, keep)
        };

        let mut history = VecDeque::new();
        let max_chars = *state.max_message_chars.lock().await;
        sync_history_window(
            &mut history,
            rows.into_iter()
                .take_while(|(id, _, _, _)| *id <= keep_through_message_id)
                .map(|(_, role, content, metadata)| (role, content, metadata)),
            max_chars,
        );
        Ok(Self {
            user_message_id,
            keep_through_message_id,
            history,
            revision,
        })
    }

    fn failure_boundary(
        rows: &[HistoryRow],
        runtime: &ChatRuntimeState,
        retry: &FailureRetry,
        character_id: &str,
        conversation_id: &str,
    ) -> Result<(i64, i64), KokoroError> {
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
        let prompt = match metadata.get("user_message_id") {
            Some(id) => id.as_i64(),
            None => runtime.failed_turn_prompt(character_id, conversation_id, turn_id),
        }
        .filter(|id| *id < retry.diagnostic_message_id)
        .ok_or_else(changed_history)?;
        let Some(boundary) = metadata.get("keep_through_message_id") else {
            return Ok((prompt, prompt));
        };
        let keep = boundary.as_i64().ok_or_else(changed_history)?;
        if keep < prompt || keep >= retry.diagnostic_message_id
            || !rows.iter().any(|(id, role, _, metadata)| {
                *id == keep && is_visible_message_raw(role, metadata.as_deref())
            })
        {
            return Err(changed_history());
        }
        Ok((prompt, keep))
    }

    pub(crate) fn prompt_history(&self, message: &str) -> Vec<Message> {
        let mut history: Vec<_> = self.history.iter().cloned().collect();
        if self.keep_through_message_id != self.user_message_id {
            // A context or unsolicited reply can sit between the original
            // prompt and selected answer. Replay the prompt for the model.
            history.push(Message {
                role: "user".into(),
                content: message.into(),
                metadata: None,
            });
        }
        history
    }

    /// The caller holds the chat execution guard from admission through streaming.
    pub(crate) async fn commit(
        self,
        state: &AIOrchestrator,
        conversation_id: &str,
        generation: u64,
        cancellation: Option<&watch::Receiver<bool>>,
    ) -> Result<(), KokoroError> {
        let _selection = state.conversation_switch_lock.lock().await;
        if state.current_conversation_generation() != generation
            || state.current_conversation_id.lock().await.as_deref() != Some(conversation_id)
        {
            return Err(changed_history());
        }
        let mut history = state.history.lock().await;
        let mut boundary = state.memory_history_boundary.lock().await;
        // Reserve the writer before the version read so a simultaneous write
        // elsewhere can't invalidate a deferred read-to-write transaction.
        let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
        if conversation_message_revision(&mut tx, conversation_id).await? != Some(self.revision) {
            return Err(changed_history());
        }
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ? AND id > ?")
            .bind(conversation_id)
            .bind(self.keep_through_message_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_turn_retry_ignores_other_conversation_writes_and_rejects_target_edits() {
        let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
        let runtime = ChatRuntimeState::default();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, created_at, updated_at)
             VALUES ('target', 'default', 'now', 'now'), ('external', 'default', 'now', 'now')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        *state.current_conversation_id.lock().await = Some("target".into());
        let generation = state.bump_conversation_generation();
        sqlx::query(
            r#"INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at)
               VALUES (1, 'target', 'user', 'question', NULL, 'now'),
                      (2, 'target', 'assistant', 'partial', NULL, 'now'),
                      (3, 'target', 'system', 'failure',
                       '{"type":"failure_event","user_message_id":1,"event":{"character_id":"default","conversation_id":"target","turn_id":"failed"}}', 'now')"#,
        )
        .execute(&state.db)
        .await
        .unwrap();
        let request = FailureRetry {
            diagnostic_message_id: 3,
            expected_tail_message_id: 3,
            expected_history_revision: Some(3),
        };
        let _turn = state.try_acquire_chat_turn("retry").unwrap();
        let prepare = |request| {
            RegenerationPlan::prepare(
                &state,
                &runtime,
                Regeneration::Failure(request),
                "default",
                "target",
                "question",
            )
        };
        let stale = prepare(&request).await.unwrap();
        // An in-place edit leaves the visible tail and the original prompt intact.
        sqlx::query("UPDATE conversation_messages SET content = 'edited' WHERE id = 2")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(matches!(
            stale.commit(&state, "target", generation, None).await,
            Err(KokoroError::Validation(_))
        ));
        assert!(matches!(
            prepare(&request).await,
            Err(KokoroError::Validation(_))
        ));
        let updated = FailureRetry {
            expected_history_revision: Some(4),
            ..request
        };
        let plan = prepare(&updated).await.unwrap();
        assert_eq!(plan.prompt_history("question").len(), 1);
        // Webhook chat uses an isolated execution guard with a shared database.
        let external = state.fork_with_isolated_history().await;
        let _external_turn = external.try_acquire_chat_turn("webhook").unwrap();
        let (_, external_id) = external
            .add_message_with_metadata_for_conversation(
                "user".into(),
                "external request".into(),
                None,
                "default",
                Some("external"),
                None,
            )
            .await
            .unwrap();
        let before: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, content FROM conversation_messages ORDER BY id")
                .fetch_all(&state.db)
                .await
                .unwrap();
        assert_eq!(
            before,
            vec![
                (1, "question".into()),
                (2, "edited".into()),
                (3, "failure".into()),
                (external_id, "external request".into())
            ]
        );
        plan.commit(&state, "target", generation, None)
            .await
            .unwrap();
        let after: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, content FROM conversation_messages ORDER BY id")
                .fetch_all(&state.db)
                .await
                .unwrap();
        assert_eq!(
            after,
            vec![
                (1, "question".into()),
                (external_id, "external request".into())
            ]
        );
    }
}
