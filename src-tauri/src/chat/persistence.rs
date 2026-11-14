use crate::error::KokoroError;
use sqlx::{Sqlite, SqlitePool, Transaction};

/// Transport limits keep oversized input out of both SQLite and prompt
/// preparation. The per-message context budget is applied separately.
pub(crate) const MAX_USER_INPUT_BYTES: usize = 256 * 1024;

pub(crate) fn validate_user_input(text: &str) -> Result<(), KokoroError> {
    if text.len() > MAX_USER_INPUT_BYTES {
        return Err(KokoroError::Validation(format!(
            "Chat input exceeds the {MAX_USER_INPUT_BYTES}-byte limit"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct ConversationTarget {
    pub character_id: String,
    pub conversation_id: String,
    pub create: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct MessageWrite {
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub save_id: Option<String>,
    draft: Option<(i64, String)>,
    replace_draft: bool,
}

impl MessageWrite {
    pub(crate) fn new(
        role: impl Into<String>,
        content: impl Into<String>,
        metadata: Option<String>,
    ) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            metadata,
            save_id: None,
            draft: None,
            replace_draft: false,
        }
    }

    pub(crate) fn with_save_id(mut self, save_id: String) -> Self {
        self.save_id = Some(save_id);
        self
    }

    pub(crate) fn finalizing(mut self, row_id: i64, turn_id: String) -> Self {
        self.draft = Some((row_id, turn_id));
        self
    }

    pub(crate) fn replacing_draft(self, row_id: i64, turn_id: String) -> Self {
        Self {
            replace_draft: true,
            ..self.finalizing(row_id, turn_id)
        }
    }

    pub(crate) fn stored_metadata(&self) -> Option<String> {
        let Some(save_id) = &self.save_id else {
            return self.metadata.clone();
        };
        let mut value = self
            .metadata
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        value["chat_save_id"] = serde_json::Value::String(save_id.clone());
        Some(value.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SavedMessage {
    pub id: i64,
    pub changed: bool,
}

#[derive(Debug, thiserror::Error)]
enum SaveError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("The conversation changed before the failed save could be retried.")]
    HistoryChanged,
}

fn is_transient_lock(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| matches!(code & 0xff, 5 | 6)) // SQLITE_BUSY, SQLITE_LOCKED
}

pub(crate) async fn save(
    db: &SqlitePool,
    target: &ConversationTarget,
    messages: &[MessageWrite],
) -> Result<Vec<SavedMessage>, KokoroError> {
    save_expected(db, target, messages, None).await
}

pub(crate) async fn save_expected(
    db: &SqlitePool,
    target: &ConversationTarget,
    messages: &[MessageWrite],
    expected_revision: Option<i64>,
) -> Result<Vec<SavedMessage>, KokoroError> {
    for delay in [Some(25), Some(75), None] {
        match save_once(db, target, messages, expected_revision).await {
            Err(SaveError::Database(error)) if delay.is_some() && is_transient_lock(&error) => {
                tokio::time::sleep(std::time::Duration::from_millis(delay.unwrap_or(0))).await;
            }
            result => {
                return result.map_err(|error| match error {
                    SaveError::Database(error) => KokoroError::chat_persistence(error),
                    SaveError::HistoryChanged => KokoroError::Validation(error.to_string()),
                })
            }
        }
    }
    unreachable!("the last save attempt returns its result")
}

async fn save_once(
    db: &SqlitePool,
    target: &ConversationTarget,
    messages: &[MessageWrite],
    expected_revision: Option<i64>,
) -> Result<Vec<SavedMessage>, SaveError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let now = chrono::Utc::now().to_rfc3339();
    let conversation = &target.conversation_id;
    let character = &target.character_id;
    if target.create {
        sqlx::query(
            "INSERT INTO conversations (id, character_id, created_at, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(conversation)
        .bind(character)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }
    match check_owner(&mut tx, target).await {
        Ok(revision) if expected_revision.is_some_and(|expected| expected != revision) => {
            return Err(SaveError::HistoryChanged);
        }
        Err(sqlx::Error::RowNotFound) if expected_revision.is_some() => {
            return Err(SaveError::HistoryChanged)
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }

    let mut saved = Vec::with_capacity(messages.len());
    for message in messages {
        saved.push(write_message(&mut tx, conversation, message, &now).await?);
    }
    if saved.iter().any(|message| message.changed) {
        let first_user_title = messages
            .iter()
            .zip(&saved)
            .find(|(message, saved)| message.role == "user" && saved.changed)
            .map(|(message, _)| {
                let mut chars = message.content.chars();
                let mut title: String = chars.by_ref().take(20).collect();
                if chars.next().is_some() {
                    title.push_str("...");
                }
                title
            });
        let updated = sqlx::query(
            "UPDATE conversations SET title = CASE WHEN title = '新对话' THEN COALESCE(?, title) ELSE title END, updated_at = ? WHERE id = ? AND character_id = ?",
        )
        .bind(first_user_title)
        .bind(&now)
        .bind(conversation)
        .bind(character)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound.into());
        }
    }
    tx.commit().await?;
    Ok(saved)
}

async fn check_owner(
    tx: &mut Transaction<'_, Sqlite>,
    target: &ConversationTarget,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT message_revision FROM conversations WHERE id = ? AND character_id = ?",
    )
    .bind(&target.conversation_id)
    .bind(&target.character_id)
    .fetch_one(&mut **tx)
    .await
}

pub(crate) async fn current_revision(
    db: &SqlitePool,
    target: &ConversationTarget,
) -> Result<Option<i64>, KokoroError> {
    sqlx::query_scalar(
        "SELECT message_revision FROM conversations WHERE id = ? AND character_id = ?",
    )
    .bind(&target.conversation_id)
    .bind(&target.character_id)
    .fetch_optional(db)
    .await
    .map_err(KokoroError::chat_persistence)
}

async fn write_message(
    tx: &mut Transaction<'_, Sqlite>,
    conversation: &str,
    message: &MessageWrite,
    now: &str,
) -> Result<SavedMessage, sqlx::Error> {
    let metadata = message.stored_metadata();
    if let Some(save_id) = &message.save_id {
        let existing: Option<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT id, content, metadata FROM conversation_messages
             WHERE conversation_id = ? AND role = ?
             AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END = ?",
        )
        .bind(conversation)
        .bind(&message.role)
        .bind(save_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some((id, content, previous_metadata)) = existing {
            if content != message.content || previous_metadata != metadata {
                return Err(sqlx::Error::Protocol(
                    "Chat save identity was reused with different content".into(),
                ));
            }
            return Ok(SavedMessage { id, changed: false });
        }
    }
    if message.replace_draft {
        let (row_id, turn_id) = message.draft.as_ref().expect("replacement has a draft");
        let removed = sqlx::query(
            "DELETE FROM conversation_messages WHERE id = ? AND conversation_id = ? AND role = 'assistant'
               AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.turn_id') END = ?
               AND CASE WHEN json_valid(metadata) THEN json_type(metadata, '$.streaming_draft') END = 'true'",
        )
        .bind(row_id)
        .bind(conversation)
        .bind(turn_id)
        .execute(&mut **tx)
        .await?;
        if removed.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
    }
    let id = if let Some((row_id, turn_id)) =
        message.draft.as_ref().filter(|_| !message.replace_draft)
    {
        let updated = sqlx::query(
            "UPDATE conversation_messages SET content = ?, metadata = ?
             WHERE id = ? AND conversation_id = ? AND role = 'assistant'
               AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.turn_id') END = ?
               AND CASE WHEN json_valid(metadata) THEN json_type(metadata, '$.streaming_draft') END = 'true'",
        )
        .bind(&message.content)
        .bind(&metadata)
        .bind(row_id)
        .bind(conversation)
        .bind(turn_id)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        *row_id
    } else {
        let inserted = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(conversation)
        .bind(&message.role)
        .bind(&message.content)
        .bind(&metadata)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        inserted.last_insert_rowid()
    };
    Ok(SavedMessage { id, changed: true })
}

pub(crate) async fn save_draft(
    db: &SqlitePool,
    target: &ConversationTarget,
    row_id: Option<i64>,
    turn_id: &str,
    text: &str,
) -> Result<i64, KokoroError> {
    let mut draft = MessageWrite::new(
        "assistant",
        text,
        Some(serde_json::json!({ "turn_id": turn_id, "streaming_draft": true }).to_string()),
    );
    if let Some(id) = row_id {
        draft = draft.finalizing(id, turn_id.to_string());
    }
    let mut tx = db
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(KokoroError::chat_persistence)?;
    check_owner(&mut tx, target)
        .await
        .map_err(KokoroError::chat_persistence)?;
    let saved = write_message(
        &mut tx,
        &target.conversation_id,
        &draft,
        &chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(KokoroError::chat_persistence)?;
    tx.commit().await.map_err(KokoroError::chat_persistence)?;
    Ok(saved.id)
}

#[derive(Clone)]
pub(crate) struct PendingUserSave {
    pub request: crate::commands::chat::ChatRequest,
    pub target: ConversationTarget,
    pub messages: Vec<MessageWrite>,
    pub expected_revision: Option<i64>,
    pub turn_id: String,
    pub generation: u64,
    pub vision: Option<crate::vision::context::VisionObservation>,
}

#[derive(Clone)]
pub(crate) struct PendingChatSave {
    pub target: ConversationTarget,
    pub messages: Vec<MessageWrite>,
    pub expected_revision: i64,
    pub turn_id: String,
    pub user_message_id: Option<i64>,
    pub creation_generation: Option<u64>,
    pub text: String,
    pub has_assistant_text: bool,
    pub speech: Option<crate::characters::activation::ResolvedTts>,
}

#[derive(Clone)]
struct PendingExternalReply {
    text: String,
    translation: Option<String>,
    expected_revision: i64,
    database_guard_generation: u64,
}

struct ExternalReplyEntry {
    conversation_id: String,
    request_id: String,
    pending: PendingExternalReply,
}

#[derive(Default)]
pub(crate) struct ExternalReplies(std::sync::Mutex<std::collections::VecDeque<ExternalReplyEntry>>);

impl ExternalReplies {
    fn get(&self, conversation: &str, request: &str) -> Option<PendingExternalReply> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.conversation_id == conversation && entry.request_id == request)
            .map(|entry| entry.pending.clone())
            .filter(|pending| {
                pending.database_guard_generation
                    == crate::ai::context::database_operation_generation()
            })
    }

    fn remember(&self, conversation: &str, request: &str, pending: PendingExternalReply) {
        const MAX_PENDING_REPLIES: usize = 32;
        const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
        let mut replies = self.0.lock().unwrap();
        replies
            .retain(|entry| entry.conversation_id != conversation || entry.request_id != request);
        if pending.text.len() + pending.translation.as_deref().map_or(0, str::len)
            > MAX_PENDING_BYTES
        {
            return;
        }
        replies.push_back(ExternalReplyEntry {
            conversation_id: conversation.to_owned(),
            request_id: request.to_owned(),
            pending,
        });
        while replies.len() > MAX_PENDING_REPLIES
            || replies
                .iter()
                .map(|entry| {
                    entry.pending.text.len()
                        + entry.pending.translation.as_deref().map_or(0, str::len)
                })
                .sum::<usize>()
                > MAX_PENDING_BYTES
        {
            replies.pop_front();
        }
    }

    fn forget(&self, conversation: &str, request: &str) {
        self.0
            .lock()
            .unwrap()
            .retain(|entry| entry.conversation_id != conversation || entry.request_id != request);
    }
}

/// Bind an external turn during its first write, then reuse that target for
/// context reads and the assistant write even when the desktop selection moves.
pub(crate) struct ExternalTurn {
    pub target: ConversationTarget,
    pub history: Vec<crate::ai::context::Message>,
    pub previous_reply: Option<(String, Option<String>)>,
    request_id: String,
}

impl ExternalTurn {
    pub(crate) async fn begin(
        state: &crate::ai::context::AIOrchestrator,
        character_id: &str,
        external_conversation_id: String,
        request_id: String,
        text: &str,
        continue_selected: bool,
    ) -> Result<Self, KokoroError> {
        validate_user_input(text)?;
        let _selection = state.conversation_switch_lock.lock().await;
        let selected = state.current_conversation_id.lock().await.clone();
        let select_new = continue_selected
            && selected.is_none()
            && state.get_character_id().await == character_id;
        let owned_selected = if continue_selected && state.get_character_id().await == character_id
        {
            selected
        } else {
            None
        };
        let save_id = format!("{request_id}:user");
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT m.conversation_id FROM conversation_messages m
             JOIN conversations c ON c.id = m.conversation_id
             WHERE c.character_id = ? AND m.role = 'user'
               AND CASE WHEN json_valid(m.metadata) THEN json_extract(m.metadata, '$.chat_save_id') END = ?",
        )
        .bind(character_id)
        .bind(&save_id)
        .fetch_optional(&state.db)
        .await
        .map_err(KokoroError::chat_persistence)?;
        let create = existing.is_none() && owned_selected.is_none();
        let target = ConversationTarget {
            character_id: character_id.to_owned(),
            conversation_id: existing.or(owned_selected).unwrap_or_else(|| {
                if select_new {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    external_conversation_id
                }
            }),
            create,
        };
        let saved = state
            .save_messages_locked(
                &target,
                &[MessageWrite::new("user", text, None).with_save_id(save_id)],
                select_new && create,
                None,
            )
            .await?;
        let previous_reply = if saved[0].changed {
            None
        } else {
            let existing: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT content, metadata FROM conversation_messages
                 WHERE conversation_id = ? AND role = 'assistant'
                   AND CASE WHEN json_valid(metadata) THEN json_extract(metadata, '$.chat_save_id') END = ?",
            )
            .bind(&target.conversation_id)
            .bind(format!("{request_id}:assistant"))
            .fetch_optional(&state.db)
            .await
            .map_err(KokoroError::chat_persistence)?;
            if let Some((text, metadata)) = existing {
                Some((
                    text,
                    metadata.as_deref().and_then(|raw| {
                        serde_json::from_str::<serde_json::Value>(raw)
                            .ok()
                            .and_then(|value| value["translation"].as_str().map(str::to_owned))
                    }),
                ))
            } else if let Some(pending) = state
                .external_replies
                .get(&target.conversation_id, &request_id)
            {
                let result = Self::save_reply_locked(
                    state,
                    &target,
                    &request_id,
                    &pending.text,
                    pending.translation.as_deref(),
                    Some(pending.expected_revision),
                )
                .await;
                if result.is_ok() || matches!(result, Err(KokoroError::Validation(_))) {
                    state
                        .external_replies
                        .forget(&target.conversation_id, &request_id);
                }
                result?;
                Some((pending.text, pending.translation))
            } else {
                return Err(KokoroError::Chat(
                    "This request has already started and has no saved reply.".into(),
                ));
            }
        };
        let history = if previous_reply.is_some() {
            Vec::new()
        } else {
            state.saved_history_for(&target, Some(saved[0].id)).await?
        };
        Ok(Self {
            target: ConversationTarget {
                create: false,
                ..target
            },
            history,
            previous_reply,
            request_id,
        })
    }

    pub(crate) async fn recent_committed_history(
        &self,
        state: &crate::ai::context::AIOrchestrator,
        count: usize,
    ) -> Vec<crate::ai::context::Message> {
        match state.saved_history_for(&self.target, None).await {
            Ok(mut history) => {
                history.retain(|message| {
                    matches!(message.role.as_str(), "user" | "assistant")
                        && !matches!(
                            message
                                .metadata
                                .as_ref()
                                .and_then(|value| value["type"].as_str()),
                            Some("assistant_tool_calls"),
                        )
                });
                history.drain(..history.len().saturating_sub(count));
                history
            }
            Err(error) => {
                tracing::warn!(target: "memory", "Failed to load committed external chat history: {error}");
                Vec::new()
            }
        }
    }

    pub(crate) async fn save_reply(
        &self,
        state: &crate::ai::context::AIOrchestrator,
        text: &str,
        translation: Option<&str>,
    ) -> Result<(), KokoroError> {
        let _selection = state.conversation_switch_lock.lock().await;
        let revision = current_revision(&state.db, &self.target)
            .await
            .ok()
            .flatten();
        let result = Self::save_reply_locked(
            state,
            &self.target,
            &self.request_id,
            text,
            translation,
            None,
        )
        .await;
        if matches!(result, Err(KokoroError::ChatPersistence(_))) {
            if let Some(expected_revision) = revision {
                state.external_replies.remember(
                    &self.target.conversation_id,
                    &self.request_id,
                    PendingExternalReply {
                        text: text.to_owned(),
                        translation: translation.map(str::to_owned),
                        expected_revision,
                        database_guard_generation:
                            crate::ai::context::database_operation_generation(),
                    },
                );
            }
        }
        result
    }

    async fn save_reply_locked(
        state: &crate::ai::context::AIOrchestrator,
        target: &ConversationTarget,
        request_id: &str,
        text: &str,
        translation: Option<&str>,
        expected_revision: Option<i64>,
    ) -> Result<(), KokoroError> {
        let metadata =
            translation.map(|text| serde_json::json!({ "translation": text }).to_string());
        state
            .save_messages_expected_locked(
                target,
                &[MessageWrite::new("assistant", text, metadata)
                    .with_save_id(format!("{request_id}:assistant"))],
                false,
                None,
                expected_revision,
            )
            .await?;
        Ok(())
    }
}

pub(crate) fn external_identity(platform: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    format!("{platform}-{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::context::AIOrchestrator;

    async fn state() -> AIOrchestrator {
        AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap()
            .fork_with_isolated_history()
            .await
    }

    fn target(conversation: &str, create: bool) -> ConversationTarget {
        ConversationTarget {
            character_id: "default".into(),
            conversation_id: conversation.into(),
            create,
        }
    }

    #[tokio::test]
    async fn failing_initial_save_rolls_back_the_conversation_and_context_without_publishing_history(
    ) {
        let state = state().await;
        sqlx::query(
            "CREATE TRIGGER reject_user BEFORE INSERT ON conversation_messages
            WHEN NEW.role = 'user' BEGIN SELECT RAISE(ABORT, 'injected user fault'); END",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let messages = [
            MessageWrite::new("context", "observation", None),
            MessageWrite::new("user", "question", None).with_save_id("request:user".into()),
        ];
        let error = {
            let _selection = state.conversation_switch_lock.lock().await;
            state
                .save_messages_locked(&target("new", true), &messages, true, None)
                .await
                .unwrap_err()
        };
        assert!(matches!(error, KokoroError::ChatPersistence(_)));
        assert_eq!(state.get_message_count().await, 0);
        assert_eq!(state.get_memory_trigger_count().await, 0);
        assert!(state.history.lock().await.is_empty());
        assert!(state.current_conversation_id.lock().await.is_none());
        let count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM conversations) +
            (SELECT COUNT(*) FROM conversation_messages)",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn commit_failure_rolls_back_message_and_metadata_that_statements_would_accept() {
        let state = state().await;
        sqlx::raw_sql(
            "CREATE TABLE commit_parent (id INTEGER PRIMARY KEY);
             CREATE TABLE commit_child (id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
             CREATE TRIGGER reject_commit AFTER INSERT ON conversation_messages
             BEGIN INSERT INTO commit_child VALUES (99); END;",
        ).execute(&state.db).await.unwrap();
        let target = target("new", true);
        let messages =
            [MessageWrite::new("user", "question", None).with_save_id("request:user".into())];
        assert!(matches!(
            save(&state.db, &target, &messages).await,
            Err(KokoroError::ChatPersistence(_))
        ));
        let count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM conversations) +
            (SELECT COUNT(*) FROM conversation_messages) + (SELECT COUNT(*) FROM commit_child)",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 0);
        sqlx::query("DROP TRIGGER reject_commit")
            .execute(&state.db)
            .await
            .unwrap();
        let first = save(&state.db, &target, &messages).await.unwrap();
        let repeat = save(&state.db, &target, &messages).await.unwrap();
        assert_eq!(first[0].id, repeat[0].id);
        assert!(first[0].changed);
        assert!(!repeat[0].changed);
    }

    #[tokio::test]
    async fn rejected_finalization_leaves_draft_and_metadata_unchanged_then_reuses_row_id() {
        let state = state().await;
        let target = target("visible", true);
        let user = MessageWrite::new("user", "question", None).with_save_id("request:user".into());
        save(&state.db, &target, &[user]).await.unwrap();
        let draft = save_draft(&state.db, &target, None, "turn", "working")
            .await
            .unwrap();
        let expected_revision = current_revision(&state.db, &target).await.unwrap();
        sqlx::query("CREATE TRIGGER reject_draft_finalization BEFORE UPDATE OF metadata ON conversation_messages
            WHEN OLD.id = NEW.id AND json_type(OLD.metadata, '$.streaming_draft') = 'true'
            BEGIN SELECT RAISE(ABORT, 'injected draft promotion fault'); END")
            .execute(&state.db).await.unwrap();
        let final_write = MessageWrite::new(
            "assistant",
            "**answer**\n\nfull reply",
            Some(r#"{"turn_id":"turn"}"#.into()),
        )
        .with_save_id("turn:assistant".into())
        .finalizing(draft, "turn".into());
        let writes = [
            MessageWrite::new("tool", "technical result", None).with_save_id("turn:tool".into()),
            final_write,
        ];
        assert!(matches!(
            save_expected(&state.db, &target, &writes, expected_revision).await,
            Err(KokoroError::ChatPersistence(_))
        ));
        let (content, draft_flag): (String, i64) = sqlx::query_as(
            "SELECT content, json_extract(metadata, '$.streaming_draft') FROM conversation_messages WHERE id = ?",
        ).bind(draft).fetch_one(&state.db).await.unwrap();
        assert_eq!((content.as_str(), draft_flag), ("working", 1));
        assert_eq!(
            current_revision(&state.db, &target).await.unwrap(),
            expected_revision
        );
        let tools: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages WHERE role = 'tool'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(tools, 0);
        assert!(state
            .saved_history_for(&target, None)
            .await
            .unwrap()
            .iter()
            .all(|message| message.role != "assistant"));
        sqlx::query("DROP TRIGGER reject_draft_finalization")
            .execute(&state.db)
            .await
            .unwrap();
        let saved = save_expected(&state.db, &target, &writes, expected_revision)
            .await
            .unwrap();
        assert_eq!(saved[1].id, draft);
        let repeated = save(&state.db, &target, &writes).await.unwrap();
        assert_eq!(repeated[1].id, draft);
        assert!(repeated.iter().all(|row| !row.changed));
    }

    #[tokio::test]
    async fn manual_save_retry_rejects_history_mutation_and_wrong_character() {
        let state = state().await;
        let mut target = target("visible", true);
        save(
            &state.db,
            &target,
            &[MessageWrite::new("user", "first", None)],
        )
        .await
        .unwrap();
        let expected_revision = current_revision(&state.db, &target).await.unwrap();
        save(
            &state.db,
            &target,
            &[MessageWrite::new("user", "second", None)],
        )
        .await
        .unwrap();
        let assistant = [MessageWrite::new("assistant", "outdated", None)];
        assert!(matches!(
            save_expected(&state.db, &target, &assistant, expected_revision).await,
            Err(KokoroError::Validation(_))
        ));
        target.character_id = "wrong".into();
        assert!(matches!(
            save(&state.db, &target, &assistant).await,
            Err(KokoroError::ChatPersistence(_))
        ));
        let assistants: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE role = 'assistant'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(assistants, 0);
    }
}
