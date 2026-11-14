use crate::actions::registry::{ActionError, ActionHandler, ActionParam};
use crate::actions::{tool_settings::ToolSettings, ActionContext, ActionRegistry, ActionResult};
use crate::ai::context::AIOrchestrator;
use crate::characters::activation::ActivationCoordinator;
use crate::chat::application::{ChatRequest, PendingToolApprovalState, TurnCancellationState};
use crate::chat::runtime::ChatRuntimeState;
use crate::commands::bot::{handle_generic_webhook, WebhookBotConfig};
use crate::error::KokoroError;
use crate::imagegen::{ImageGenService, WindowSizeState};
use crate::llm::provider::{
    LlmParams, LlmProvider, LlmStreamEvent, LlmToolCall, LlmToolDefinition,
};
use crate::llm::service::LlmService;
use crate::telegram::bot::{submit_message, SessionMode};
use async_openai::types::chat::ChatCompletionRequestMessage;
use futures::{stream, Stream, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::Manager;
use teloxide::types::ChatId;
use tokio::sync::{Notify, RwLock};

type FixtureStream<T> = Pin<Box<dyn Stream<Item = Result<T, String>> + Send>>;

struct CountToolExecutions(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl ActionHandler for CountToolExecutions {
    fn name(&self) -> &str {
        "count_executions"
    }
    fn description(&self) -> &str {
        "Test sentinel tool"
    }
    fn parameters(&self) -> Vec<ActionParam> {
        Vec::new()
    }

    async fn execute(
        &self,
        _: HashMap<String, String>,
        _: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(ActionResult::ok("sentinel executed"))
    }
}

#[derive(Default)]
struct ControlledProvider {
    prompts: Mutex<Vec<Value>>,
    entered_pending: Notify,
    finish_pending: Notify,
}

#[async_trait::async_trait]
impl LlmProvider for ControlledProvider {
    async fn chat(
        &self,
        _: Vec<ChatCompletionRequestMessage>,
        _: Option<LlmParams>,
    ) -> Result<String, String> {
        // Desktop fallback cue analysis has no target in this fixture.
        Ok("{\"cue\":null}".into())
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        _: Option<LlmParams>,
    ) -> Result<FixtureStream<String>, String> {
        let body = serde_json::to_value(messages).unwrap();
        let body_text = body.to_string();
        let last_user = body
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .map(Value::to_string)
            .unwrap_or_default();
        self.prompts.lock().unwrap().push(body);
        if last_user.contains("[wait]") {
            self.entered_pending.notify_one();
            self.finish_pending.notified().await;
        }
        let chunks = if last_user.contains("[fail-stream]") {
            vec![
                Ok("partial [TOOL_CALL:get_time]".into()),
                Err("fixture stream interrupted".into()),
            ]
        } else if last_user.contains("[tools]") && !body_text.contains("[Tool results]") {
            vec![Ok("First [TOOL_CALL:get_time]".into())]
        } else {
            vec![Ok("Shared reply".into())]
        };
        Ok(Box::pin(stream::iter(chunks)))
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        _: Vec<LlmToolDefinition>,
    ) -> Result<FixtureStream<LlmStreamEvent>, String> {
        let payload = serde_json::to_value(&messages).unwrap();
        let last_user = payload
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "user");
        let last_user = last_user.map(Value::to_string).unwrap_or_default();
        let native_fixture = [
            "[native-exchange-failure]",
            "[after-tool-stream-failure]",
            "[after-tool-final-save]",
        ]
        .iter()
        .any(|label| last_user.contains(label));
        if native_fixture {
            let has_result = payload
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "tool");
            self.prompts.lock().unwrap().push(payload);
            let response = if !has_result {
                Ok(LlmStreamEvent::ToolCall(LlmToolCall {
                    id: "fixture-call".into(),
                    name: "count_executions".into(),
                    args: HashMap::new(),
                }))
            } else if last_user.contains("[after-tool-stream-failure]") {
                Err("fixture post-tool stream failed".into())
            } else {
                Ok(LlmStreamEvent::Text("fixture final response".into()))
            };
            return Ok(Box::pin(stream::iter([response])));
        }
        Ok(Box::pin(
            self.chat_stream(messages, options)
                .await?
                .map(|result| result.map(LlmStreamEvent::Text)),
        ))
    }

    fn id(&self) -> &str {
        "chat-test"
    }
}

async fn webhook(
    app: &tauri::AppHandle,
    id: &str,
    peer: &str,
    text: &str,
    image: Option<&str>,
) -> (u16, Value) {
    let config = WebhookBotConfig {
        bearer_token: Some("fixture-token".into()),
        ..Default::default()
    };
    let body = json!({"request_id": id, "conversation_id": peer, "text": text, "image": image});
    let response = handle_generic_webhook(
        config,
        Some("Bearer fixture-token".into()),
        body.to_string().into(),
        app.clone(),
    )
    .await;
    let status = response.status().as_u16();
    let bytes = warp::hyper::body::to_bytes(response.into_body())
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn until_request_started(provider: &ControlledProvider) {
    tokio::time::timeout(Duration::from_secs(5), provider.entered_pending.notified())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn desktop_webhook_and_telegram_execute_the_same_bound_chat() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::<tauri::Wry>::default()
        .any_thread()
        .build(context)
        .unwrap();
    let handle = app.handle().clone();

    let state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    state.set_memory_enabled(false).await;
    state.set_character_id("test-character".into()).await;
    let now = "2026-11-14T00:00:00Z";
    sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('desktop', 'test-character', 'desktop', ?, ?)")
        .bind(now).bind(now).execute(&state.db).await.unwrap();
    *state.current_conversation_id.lock().await = Some("desktop".into());
    let provider = Arc::new(ControlledProvider::default());
    let mut registry = ActionRegistry::new();
    crate::actions::builtin::register_builtins(&mut registry);
    let tool_executions = Arc::new(AtomicUsize::new(0));
    registry.register(CountToolExecutions(tool_executions.clone()));
    let runtime = Arc::new(ChatRuntimeState::default());
    let cancellation = Arc::new(TurnCancellationState::default());
    let imagegen = ImageGenService::init_from_config(&Default::default()).await;
    handle.manage(state);
    handle.manage(runtime.clone());
    handle.manage(cancellation.clone());
    handle.manage(Arc::new(PendingToolApprovalState::default()));
    handle.manage(ActivationCoordinator::default());
    handle.manage(imagegen);
    handle.manage(WindowSizeState::default());
    handle.manage(crate::vision::watcher::VisionWatcher::new(
        Default::default(),
    ));
    handle.manage(Arc::new(tokio::sync::Mutex::new(
        crate::vision::server::VisionServer::new(temp.path()),
    )));
    handle.manage(Arc::new(RwLock::new(registry)));
    handle.manage(Arc::new(RwLock::new(ToolSettings::default())));
    handle.manage(LlmService::for_test(provider.clone()));

    let state = handle.state::<AIOrchestrator>();
    let desktop = |id: &'static str, text: &'static str, images: Option<Vec<String>>| {
        crate::commands::chat::stream_chat(
            handle.clone(),
            ChatRequest {
                message: text.into(),
                client_request_id: Some(id.into()),
                conversation_id: Some("desktop".into()),
                character_id: Some("test-character".into()),
                images,
                ..Default::default()
            },
        )
    };
    let image = "data:image/png;base64,AAAB";
    let text = "[tools] same input";
    let desktop_answer = desktop("desktop-tool", text, Some(vec![image.into()]))
        .await
        .unwrap();
    let before_webhook = provider.prompts.lock().unwrap().len();
    let (status, bot_answer) = webhook(&handle, "request", "one", text, Some(image)).await;
    assert_eq!(status, 200);
    let after_webhook = provider.prompts.lock().unwrap().len();
    let private = SessionMode::Private;
    let telegram_answer = submit_message(
        &handle,
        ChatId(10),
        2,
        text,
        vec![image.into()],
        &private,
        "test-character",
    )
    .await
    .unwrap();
    let prompts = provider.prompts.lock().unwrap().clone();
    assert_eq!(after_webhook - before_webhook, 2);
    assert_eq!(prompts.len() - after_webhook, 2);
    assert_eq!(desktop_answer.text, bot_answer["reply"]);
    assert_eq!(desktop_answer.text, telegram_answer.text);
    for start in [0, before_webhook, after_webhook] {
        assert!(prompts[start].to_string().contains(image));
        let continuation = prompts[start + 1].to_string();
        assert!(continuation.contains("[Tool results]"));
        assert!(continuation.contains("builtin__get_time"));
    }
    assert_eq!(
        state.current_conversation_id.lock().await.as_deref(),
        Some("desktop")
    );
    assert_ne!(
        telegram_answer.conversation_id,
        desktop_answer.conversation_id
    );
    assert_eq!(
        persisted_assistant_text(&state.db, telegram_answer.assistant_message_id.unwrap()).await,
        telegram_answer.text
    );
    assert_eq!(
        persisted_assistant_text(&state.db, desktop_answer.assistant_message_id.unwrap()).await,
        desktop_answer.text
    );

    let no_auth = handle_generic_webhook(
        WebhookBotConfig {
            bearer_token: Some("fixture-token".into()),
            ..Default::default()
        },
        None,
        br#"{"text":"[unauthorized]"}"#.as_slice().into(),
        handle.clone(),
    )
    .await;
    assert_eq!(no_auth.status().as_u16(), 401);
    assert_eq!(provider.prompts.lock().unwrap().len(), prompts.len());
    let all_messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(all_messages, 6);

    // Simulate a lost HTTP response and repeat the authenticated request.
    let duplicate = webhook(&handle, "request", "one", text, Some(image)).await;
    assert_eq!(duplicate, (200, bot_answer));
    assert_eq!(provider.prompts.lock().unwrap().len(), prompts.len());
    assert_eq!(
        webhook(&handle, "request", "one", "conflicting input", None)
            .await
            .0,
        409
    );
    assert_eq!(provider.prompts.lock().unwrap().len(), prompts.len());
    let desktop_text = desktop("plain", "ordinary text", None).await.unwrap();
    let (status, webhook_text) = webhook(&handle, "request", "two", "ordinary text", None).await;
    assert_eq!(status, 200);
    let other_peer = submit_message(
        &handle,
        ChatId(11),
        2,
        "ordinary text",
        vec![],
        &private,
        "test-character",
    )
    .await
    .unwrap();
    assert_ne!(other_peer.conversation_id, telegram_answer.conversation_id);
    assert_eq!(desktop_text.text, webhook_text["reply"]);
    assert_eq!(desktop_text.text, other_peer.text);
    assert!(!provider
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .to_string()
        .contains(text));

    let (status, failure) = webhook(&handle, "bad-model", "one", "[fail-stream]", None).await;
    assert_eq!(status, 500);
    assert!(failure["error"]
        .as_str()
        .unwrap()
        .contains("fixture stream interrupted"));
    let failed_snapshot = runtime
        .snapshots()
        .into_iter()
        .find(|turn| turn.message == "[fail-stream]")
        .unwrap();
    assert_eq!(failed_snapshot.status, "error");
    assert!(failed_snapshot.assistant_message_id.is_none());

    sqlx::query("CREATE TRIGGER refuse_assistant BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(FAIL, 'fixture save failed'); END")
        .execute(&state.db).await.unwrap();
    let save_failure = submit_message(
        &handle,
        ChatId(10),
        3,
        "save me",
        vec![],
        &private,
        "test-character",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(save_failure, KokoroError::Database(ref message) if message.contains("fixture save failed")),
        "{save_failure:?}"
    );
    sqlx::query("DROP TRIGGER refuse_assistant")
        .execute(&state.db)
        .await
        .unwrap();

    // A failing tool row cannot leave the preceding assistant call in history.
    sqlx::query(
        "CREATE TRIGGER refuse_tool_result BEFORE INSERT ON conversation_messages
         WHEN NEW.role = 'tool' BEGIN SELECT RAISE(FAIL, 'fixture tool result save failed'); END",
    )
    .execute(&state.db)
    .await
    .unwrap();
    // Prevent cleanup from masking a committed orphan and verify the transaction
    // itself rolls it back.
    sqlx::query(
        "CREATE TRIGGER reject_tool_exchange_cleanup BEFORE DELETE ON conversation_messages
         WHEN json_extract(OLD.metadata, '$.type') = 'assistant_tool_calls'
         BEGIN SELECT RAISE(FAIL, 'orphaned assistant call'); END",
    )
    .execute(&state.db)
    .await
    .unwrap();

    let before = provider.prompts.lock().unwrap().len();
    let submit_failing_exchange = || {
        submit_message(
            &handle,
            ChatId(10),
            12,
            "[native-exchange-failure]",
            Vec::new(),
            &private,
            "test-character",
        )
    };
    let exchange_failure = submit_failing_exchange().await.unwrap_err();
    assert!(
        matches!(exchange_failure, KokoroError::Database(ref message)
            if message.contains("fixture tool result save failed")),
        "{exchange_failure:?}"
    );
    let group = runtime
        .snapshots()
        .into_iter()
        .find(|turn| turn.message == "[native-exchange-failure]")
        .unwrap();
    assert_eq!(group.status, "error");
    assert!(group.assistant_message_id.is_none());
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_messages WHERE json_extract(metadata, '$.turn_id') = ? AND role IN ('assistant','tool')",
    ).bind(group.turn_id.as_deref()).fetch_one(&state.db).await.unwrap();
    assert_eq!(count, 0);
    assert_non_retryable_diagnostic(&state.db, group.turn_id.as_deref().unwrap(), "tool_save")
        .await;
    assert_eq!(tool_executions.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), before + 1);
    assert_eq!(
        submit_failing_exchange().await.unwrap_err(),
        exchange_failure
    );
    assert_eq!(tool_executions.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), before + 1);
    sqlx::query("DROP TRIGGER refuse_tool_result")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER reject_tool_exchange_cleanup")
        .execute(&state.db)
        .await
        .unwrap();

    sqlx::query(
        "CREATE TRIGGER refuse_final_reply BEFORE INSERT ON conversation_messages
         WHEN NEW.role = 'assistant' AND json_extract(NEW.metadata, '$.type') IS NULL
         BEGIN SELECT RAISE(FAIL, 'fixture final save after tool failed'); END",
    )
    .execute(&state.db)
    .await
    .unwrap();

    for (id, message, expected_stage, expected_error) in [
        (
            13,
            "[after-tool-stream-failure]",
            "stream_receive",
            KokoroError::Llm("fixture post-tool stream failed".into()),
        ),
        (
            14,
            "[after-tool-final-save]",
            "final_save",
            KokoroError::Database("fixture final save after tool failed".into()),
        ),
    ] {
        let submit = || {
            submit_message(
                &handle,
                ChatId(10),
                id,
                message,
                Vec::new(),
                &private,
                "test-character",
            )
        };
        let before = tool_executions.load(Ordering::Relaxed);
        let error = submit().await.unwrap_err();
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected_error)
        );
        let snapshot = runtime
            .snapshots()
            .into_iter()
            .find(|turn| turn.message == message)
            .unwrap();
        assert_eq!(snapshot.status, "error");
        assert_non_retryable_diagnostic(
            &state.db,
            snapshot.turn_id.as_deref().unwrap(),
            expected_stage,
        )
        .await;
        assert_eq!(submit().await.unwrap_err(), error);
        assert_eq!(tool_executions.load(Ordering::Relaxed), before + 1);
    }
    sqlx::query("DROP TRIGGER refuse_final_reply")
        .execute(&state.db)
        .await
        .unwrap();

    let fresh = SessionMode::Fresh {
        id: "fresh-session".into(),
    };
    let fresh_answer = submit_message(
        &handle,
        ChatId(10),
        6,
        "telegram fresh",
        vec![],
        &fresh,
        "test-character",
    )
    .await
    .unwrap();
    assert_ne!(
        fresh_answer.conversation_id,
        telegram_answer.conversation_id
    );
    assert!(!provider
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .to_string()
        .contains(text));
    assert_eq!(
        state.current_conversation_id.lock().await.as_deref(),
        Some("desktop")
    );

    // An in-memory journal could be empty after a restart; the indexed
    // persistence check still recognizes a message written under /new.
    let request_id = fresh_answer.client_request_id.as_deref().unwrap();
    let restarted = ChatRuntimeState::default();
    assert!(restarted.outcome(request_id).is_none());
    assert!(
        super::application::external_request_exists(&state.db, "telegram", request_id)
            .await
            .unwrap()
    );
    assert!(
        !super::application::external_request_exists(&state.db, "webhook", request_id)
            .await
            .unwrap()
    );

    let continued = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
    };
    let first = tokio::spawn(desktop("pending", "[wait] desktop owner", None));
    until_request_started(&provider).await;
    let other = submit_message(
        &handle,
        ChatId(10),
        4,
        "same conversation",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap_err();
    assert!(other.to_string().contains("chat_turn_busy"));
    first.abort(); // The subscriber goes away while the application keeps the turn.
    let (_, snapshots) = runtime.desktop_snapshots(Some("desktop"));
    assert!(snapshots
        .iter()
        .any(|turn| turn.client_request_id == "pending" && turn.status == "streaming"));
    let mut completion = runtime.outcome("pending").unwrap();
    crate::chat::application::cancel_chat_turn_inner("pending".into(), None, cancellation)
        .await
        .unwrap();
    completion
        .wait_for(|outcome| outcome.is_some())
        .await
        .unwrap();
    let result = completion.borrow().clone().unwrap().unwrap();
    assert_eq!(result.status.as_deref(), Some("cancelled"));

    // A new caller can rejoin and get the terminal outcome from the same task.
    assert_eq!(
        desktop("pending", "[wait] desktop owner", None)
            .await
            .unwrap(),
        result
    );
    let activation = state.acquire_activation_lock().await;
    let during_activation = submit_message(
        &handle,
        ChatId(10),
        7,
        "during activation",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap_err();
    assert!(during_activation
        .to_string()
        .contains("activation is in progress"));
    drop(activation);

    let answer = submit_message(
        &handle,
        ChatId(10),
        5,
        "shared once free",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap();
    assert_eq!(answer.conversation_id, "desktop");
    assert_eq!(
        persisted_assistant_text(&state.db, answer.assistant_message_id.unwrap()).await,
        answer.text
    );

    let first = tokio::spawn(desktop("detached", "[wait] detached subscriber", None));
    until_request_started(&provider).await;
    first.abort();
    let mut completion = runtime.outcome("detached").unwrap();
    provider.finish_pending.notify_one();
    tokio::time::timeout(
        Duration::from_secs(5),
        completion.wait_for(|outcome| outcome.is_some()),
    )
    .await
    .unwrap()
    .unwrap();
    let result = completion.borrow().clone().unwrap().unwrap();
    assert_eq!(result.status.as_deref(), Some("completed"));
    assert_eq!(
        desktop("detached", "[wait] detached subscriber", None)
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        persisted_assistant_text(&state.db, result.assistant_message_id.unwrap()).await,
        result.text
    );
}

async fn assert_non_retryable_diagnostic(db: &sqlx::SqlitePool, turn_id: &str, stage: &str) {
    let diagnostic: String = sqlx::query_scalar(
        "SELECT metadata FROM conversation_messages
         WHERE role = 'system' AND json_extract(metadata, '$.event.turn_id') = ?",
    )
    .bind(turn_id)
    .fetch_one(db)
    .await
    .unwrap();
    let diagnostic: Value = serde_json::from_str(&diagnostic).unwrap();
    assert_eq!(diagnostic["event"]["stage"], stage);
    assert_eq!(diagnostic["event"]["retryable"], false);
    assert!(diagnostic["user_message_id"].is_null());
}

async fn persisted_assistant_text(db: &sqlx::SqlitePool, message_id: i64) -> String {
    sqlx::query_scalar(
        "SELECT content FROM conversation_messages WHERE id = ? AND role = 'assistant'",
    )
    .bind(message_id)
    .fetch_one(db)
    .await
    .unwrap()
}
