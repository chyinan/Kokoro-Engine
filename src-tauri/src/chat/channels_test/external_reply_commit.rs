use super::{webhook_body_with_services, ControlledProvider, CountToolExecutions};
use crate::actions::ActionRegistry;
use crate::ai::context::AIOrchestrator;
use crate::bot::{BotChatServices, BotEvent};
use crate::chat::application::{ChatService, StreamChatResponse};
use crate::chat::persistence::ExternalTurn;
use crate::chat::runtime::ChatRuntimeState;
use crate::error::KokoroError;
use crate::llm::service::LlmService;
use crate::telegram::bot::{submit_message, SessionMode};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use teloxide::types::ChatId;

const CHARACTER: &str = "external-reply-fixture";
const PROMPT: &str = "[external-commit-fixture] fixture input";
const DISPLAYED: &str = "Reply from SQLite fixture\n\nSecond paragraph";
const CANONICAL: &str = "**Reply from SQLite fixture**\n\n*Second paragraph*";
const TRANSLATION: &str = "测试译文";

struct Fixture {
    state: Arc<AIOrchestrator>,
    journal: Arc<ChatRuntimeState>,
    services: BotChatServices,
}

impl Fixture {
    async fn open(
        dir: &Path,
        provider: Arc<ControlledProvider>,
        executions: Arc<AtomicUsize>,
        memory_enabled: bool,
    ) -> Self {
        let url = format!(
            "sqlite:///{}",
            dir.join("fixture.sqlite")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let state = Arc::new(AIOrchestrator::new(&url).await.unwrap());
        sqlx::query(
            "INSERT OR IGNORE INTO characters(id, name) VALUES (?, 'External reply fixture')",
        )
        .bind(CHARACTER)
        .execute(&state.db)
        .await
        .unwrap();
        state.set_memory_enabled(memory_enabled).await;
        let journal = Arc::new(ChatRuntimeState::default());
        let mut registry = ActionRegistry::new();
        registry.register(CountToolExecutions(executions, false));
        let chat = ChatService::for_test_with_registry(
            state.clone(),
            LlmService::for_test(provider),
            dir,
            registry,
            journal.clone(),
        )
        .await;
        let services = BotChatServices::new(
            Some((chat, state.clone())),
            None,
            None,
            None,
            Arc::new(|_: BotEvent| Ok(())),
        );
        Self {
            state,
            journal,
            services,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Channel {
    Webhook,
    Telegram,
}

#[derive(Debug)]
enum Submission {
    Webhook(u16, Value),
    Telegram(Result<StreamChatResponse, KokoroError>),
}

impl Channel {
    async fn submit(self, fixture: &Fixture, id: i32, message: &str) -> Submission {
        match self {
            Self::Webhook => {
                let (status, body) = webhook_body_with_services(
                    &fixture.services,
                    json!({
                        "character_id": CHARACTER, "conversation_id": "sqlite-fixture",
                        "request_id": id.to_string(), "text": message,
                    }),
                )
                .await;
                Submission::Webhook(status, body)
            }
            Self::Telegram => Submission::Telegram(
                submit_message(
                    &fixture.services.chat().unwrap().service,
                    ChatId(522),
                    id,
                    message,
                    Vec::new(),
                    &SessionMode::Private,
                    CHARACTER,
                )
                .await,
            ),
        }
    }
}

impl Submission {
    fn completed_request(
        &self,
        text: &str,
        translation: Option<&str>,
        newly_committed: bool,
    ) -> (&str, &str) {
        match self {
            Self::Webhook(status, body) => {
                assert_eq!(*status, 200, "{body}");
                let request_id = body["request_id"].as_str().unwrap();
                let conversation = body["conversation_id"].as_str().unwrap();
                let mut wire = json!({"character_id":CHARACTER, "conversation_id":conversation, "request_id":request_id, "reply":text});
                if let Some(translation) = translation {
                    wire["translation"] = json!(translation);
                }
                // The webhook contract exposes the request identity, not the SQLite row ID.
                assert_eq!(body, &wire);
                (request_id, conversation)
            }
            Self::Telegram(result) => {
                let reply = result.as_ref().unwrap();
                assert_eq!(reply.status.as_deref(), Some("completed"));
                assert_eq!(reply.text, text);
                assert_eq!(reply.translation.as_deref(), translation);
                assert_eq!(reply.newly_committed, newly_committed);
                (
                    reply.client_request_id.as_deref().unwrap(),
                    &reply.conversation_id,
                )
            }
        }
    }

    fn assert_persistence_error(&self) {
        match self {
            Self::Webhook(status, body) => {
                assert_eq!(*status, 500, "{body}");
                assert_eq!(body["code"], KokoroError::CHAT_PERSISTENCE_CODE);
                assert!(body.get("reply").is_none());
            }
            Self::Telegram(result) => assert!(
                matches!(result, Err(KokoroError::ChatPersistence(_))),
                "{result:?}"
            ),
        }
    }
}

#[derive(Debug, PartialEq)]
struct SavedReply {
    request_id: String,
    id: i64,
    metadata: Value,
    completion_revision: u64,
}

async fn assert_committed(
    fixture: &Fixture,
    submission: &Submission,
    newly_committed: bool,
) -> SavedReply {
    let (request_id, conversation) =
        submission.completed_request(DISPLAYED, Some(TRANSLATION), newly_committed);
    let turn = fixture
        .journal
        .snapshots()
        .into_iter()
        .find(|turn| turn.client_request_id == request_id)
        .unwrap();
    assert_eq!(turn.status, "completed");
    assert_eq!(turn.text, DISPLAYED);
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, content, metadata FROM conversation_messages
         WHERE conversation_id=? AND role='assistant' AND json_extract(metadata,'$.chat_save_id')=?",
    )
    .bind(conversation).bind(format!("{request_id}:assistant"))
    .fetch_all(&fixture.state.db).await.unwrap();
    let [(id, content, raw_metadata)] = rows.as_slice() else {
        panic!("expected one final reply, got {rows:?}")
    };
    assert_eq!(content, CANONICAL);
    let metadata: Value = serde_json::from_str(raw_metadata).unwrap();
    assert_eq!(metadata["translation"], TRANSLATION);
    assert_eq!(
        metadata["external_user_message_id"].as_i64(),
        turn.user_message_id
    );
    assert_eq!(turn.assistant_message_id, Some(*id));
    let terminal = fixture.journal.outcome(request_id).unwrap();
    let terminal = terminal.borrow();
    let result = terminal.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(result.assistant_message_id, Some(*id));
    assert_eq!(result.translation.as_deref(), Some(TRANSLATION));
    if let Submission::Telegram(Ok(reply)) = submission {
        assert_eq!(reply.assistant_message_id, Some(*id));
    }
    SavedReply {
        request_id: request_id.into(),
        id: *id,
        metadata,
        completion_revision: turn.completion_revision.unwrap(),
    }
}

fn model_and_tool_calls(provider: &ControlledProvider, tools: &AtomicUsize) -> (usize, usize) {
    (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    )
}

#[tokio::test]
async fn both_adapters_return_the_committed_reply_after_duplicate_and_database_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(ControlledProvider::default());
    let tools = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::open(temp.path(), provider.clone(), tools.clone(), false).await;
    let mut replies = Vec::new();
    for channel in [Channel::Webhook, Channel::Telegram] {
        let before = model_and_tool_calls(&provider, &tools);
        let first = channel.submit(&fixture, 1, PROMPT).await;
        let saved = assert_committed(&fixture, &first, true).await;
        let after = (before.0 + 2, before.1 + 1);
        assert_eq!(model_and_tool_calls(&provider, &tools), after);

        let duplicate = channel.submit(&fixture, 1, PROMPT).await;
        assert_eq!(assert_committed(&fixture, &duplicate, false).await, saved);
        assert_eq!(model_and_tool_calls(&provider, &tools), after);
        replies.push((channel, saved));
    }

    fixture.state.db.close().await;
    assert!(fixture.state.db.is_closed());
    drop(fixture);
    let reopened = Fixture::open(temp.path(), provider.clone(), tools.clone(), false).await;
    assert!(reopened.journal.snapshots().is_empty());
    for (channel, saved) in replies {
        let replay = channel.submit(&reopened, 1, PROMPT).await;
        let committed = assert_committed(&reopened, &replay, false).await;
        assert_eq!(
            (committed.request_id, committed.id, committed.metadata),
            (saved.request_id, saved.id, saved.metadata)
        );
    }
    assert_eq!(model_and_tool_calls(&provider, &tools), (4, 2));
}

async fn assert_no_memory_rows(db: &SqlitePool) {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE character_id=?")
        .bind(CHARACTER)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn both_adapters_recover_a_rolled_back_reply_without_rerunning_model_or_tools() {
    for channel in [Channel::Webhook, Channel::Telegram] {
        let temp = tempfile::tempdir().unwrap();
        let provider = Arc::new(ControlledProvider {
            fail_memory: true,
            ..Default::default()
        });
        let tools = Arc::new(AtomicUsize::new(0));
        let fixture = Fixture::open(temp.path(), provider.clone(), tools.clone(), true).await;
        let db = &fixture.state.db;
        // The fifth committed reply should start periodic extraction. A failed
        // batch must leave the cadence at four until recovery actually commits.
        for id in 1..5 {
            channel
                .submit(&fixture, id, "fixture input")
                .await
                .completed_request("Shared reply", None, true);
        }
        sqlx::query(
            "CREATE TRIGGER refuse_fixture_reply BEFORE INSERT ON conversation_messages
                     WHEN NEW.role='assistant' AND json_extract(NEW.metadata,'$.type') IS NULL
                     BEGIN SELECT RAISE(ABORT,'fixture reply batch failed'); END",
        )
        .execute(db)
        .await
        .unwrap();
        let failed = channel.submit(&fixture, 5, PROMPT).await;
        failed.assert_persistence_error();
        let turn = fixture
            .journal
            .snapshots()
            .into_iter()
            .find(|turn| turn.message == PROMPT)
            .unwrap();
        assert_eq!(turn.status, "error");
        assert_eq!(turn.error_code, Some(KokoroError::CHAT_PERSISTENCE_CODE));
        assert_eq!(turn.assistant_message_id, None);
        let failed_outcome = fixture.journal.outcome(&turn.client_request_id).unwrap();
        assert!(matches!(
            failed_outcome.borrow().as_ref(),
            Some(Err(KokoroError::ChatPersistence(_)))
        ));
        let conversation = turn.conversation_id.as_deref().unwrap();
        let user_id = turn.user_message_id.unwrap();
        let roles = || {
            sqlx::query_scalar::<_, String>(
            "SELECT role FROM conversation_messages WHERE conversation_id=? AND id>=? ORDER BY id",
        ).bind(conversation).bind(user_id).fetch_all(db)
        };
        assert_eq!(roles().await.unwrap(), ["user"]);
        assert_no_memory_rows(db).await;
        assert!(provider.memory_prompts.lock().unwrap().is_empty());
        let after_failure = model_and_tool_calls(&provider, &tools);
        assert_eq!(after_failure, (6, 1));

        sqlx::query("DROP TRIGGER refuse_fixture_reply")
            .execute(db)
            .await
            .unwrap();
        let recovered = channel.submit(&fixture, 5, PROMPT).await;
        let saved = assert_committed(&fixture, &recovered, true).await;
        assert_eq!(saved.request_id, turn.client_request_id);
        assert_eq!(
            roles().await.unwrap(),
            ["user", "assistant", "tool", "assistant"]
        );
        tokio::time::timeout(Duration::from_secs(5), provider.memory_started.notified())
            .await
            .unwrap();
        let memory_prompts = provider.memory_prompts.lock().unwrap().clone();
        assert_eq!(memory_prompts.len(), 1);
        assert!(memory_prompts[0]
            .to_string()
            .contains("Reply from SQLite fixture"));
        assert_no_memory_rows(db).await;

        let replay = channel.submit(&fixture, 5, PROMPT).await;
        assert_eq!(assert_committed(&fixture, &replay, false).await, saved);
        assert_eq!(model_and_tool_calls(&provider, &tools), after_failure);
        assert_eq!(provider.memory_prompts.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn reply_batch_returns_the_same_row_id_without_rescheduling_memory_for_duplicate_write() {
    let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    state.set_memory_enabled(true).await;
    let turn = ExternalTurn::begin(
        &state,
        "default",
        "reply-batch".into(),
        "request".into(),
        "prompt",
        false,
    )
    .await
    .unwrap();
    let save = || {
        turn.save_reply_batch(
            &state,
            "saved reply",
            None,
            Default::default(),
            Vec::new(),
            None,
        )
    };
    let first = save().await.unwrap();
    let saved_id: i64 = sqlx::query_scalar(
        "SELECT id FROM conversation_messages WHERE role='assistant' AND content='saved reply'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(first.assistant_message_id, saved_id);
    let memory = first.memory_work.unwrap();
    assert_eq!(memory.count, 1);
    assert_eq!(memory.through_message_id, saved_id);

    let duplicate = save().await.unwrap();
    assert_eq!(duplicate.assistant_message_id, saved_id);
    assert!(duplicate.memory_work.is_none());
}
