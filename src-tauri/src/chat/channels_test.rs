use crate::actions::registry::{ActionError, ActionHandler, ActionParam, ActionPermissionLevel};
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
use warp::Filter;

type FixtureStream<T> = Pin<Box<dyn Stream<Item = Result<T, String>> + Send>>;

struct CountToolExecutions(Arc<AtomicUsize>, bool);

#[async_trait::async_trait]
impl ActionHandler for CountToolExecutions {
    fn name(&self) -> &str {
        if self.1 {
            "approved_execution"
        } else {
            "count_executions"
        }
    }
    fn description(&self) -> &str {
        "Test sentinel tool"
    }
    fn parameters(&self) -> Vec<ActionParam> {
        Vec::new()
    }

    fn permission_level(&self) -> ActionPermissionLevel {
        if self.1 {
            ActionPermissionLevel::Elevated
        } else {
            ActionPermissionLevel::Safe
        }
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
        } else if last_user.contains("[tool-only]") {
            vec![Ok("[TOOL_CALL:count_executions]".into())]
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
        if last_user.contains("[hidden-scope]") {
            let result_count = payload
                .as_array()
                .unwrap()
                .iter()
                .filter(|message| message["role"] == "tool")
                .count();
            self.prompts.lock().unwrap().push(payload);
            let response = match result_count {
                0 => vec![LlmStreamEvent::ToolCall(LlmToolCall {
                    id: "hidden-first".into(),
                    name: "count_executions".into(),
                    args: HashMap::new(),
                })],
                1 => vec![
                    LlmStreamEvent::Text("Waiting for approval".into()),
                    LlmStreamEvent::ToolCall(LlmToolCall {
                        id: "hidden-approved".into(),
                        name: "approved_execution".into(),
                        args: HashMap::new(),
                    }),
                ],
                _ => vec![LlmStreamEvent::Text("Approved action finished".into())],
            };
            return Ok(Box::pin(stream::iter(response.into_iter().map(Ok))));
        }
        let native_fixture = [
            "[native-exchange-failure]",
            "[after-tool-stream-failure]",
            "[after-tool-final-save]",
            "[native-tool-only]",
            "[formatted-replay]",
        ]
        .iter()
        .any(|label| last_user.contains(label));
        if native_fixture {
            // Earlier recovered turns now retain their complete tool exchanges.
            // Only a tool result after this user starts the continuation round.
            let messages = payload.as_array().unwrap();
            let user_index = messages
                .iter()
                .rposition(|message| message["role"] == "user")
                .unwrap();
            let has_result = messages[user_index + 1..]
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
            } else if last_user.contains("[formatted-replay]") {
                Ok(LlmStreamEvent::Text(
                    "**Review formatted answer**\n\n*Second paragraph*".into(),
                ))
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
    let body = json!({"request_id": id, "conversation_id": peer, "text": text, "image": image});
    webhook_body(app, body).await
}

async fn webhook_body(app: &tauri::AppHandle, body: Value) -> (u16, Value) {
    let config = WebhookBotConfig {
        bearer_token: Some("fixture-token".into()),
        ..Default::default()
    };
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
    registry.register(CountToolExecutions(tool_executions.clone(), false));
    registry.register(CountToolExecutions(tool_executions.clone(), true));
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

    verify_persisted_external_contracts(&handle, &provider, &tool_executions).await;

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
        matches!(save_failure, KokoroError::ChatPersistence(_)),
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
        matches!(exchange_failure, KokoroError::ChatPersistence(_)),
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
    assert_eq!(group.error_code, Some(KokoroError::CHAT_PERSISTENCE_CODE));
    assert_eq!(tool_executions.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), before + 2);
    assert_eq!(
        submit_failing_exchange().await.unwrap_err(),
        exchange_failure
    );
    assert_eq!(tool_executions.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), before + 2);
    sqlx::query("DROP TRIGGER refuse_tool_result")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER reject_tool_exchange_cleanup")
        .execute(&state.db)
        .await
        .unwrap();
    let recovered_exchange = submit_failing_exchange().await.unwrap();
    assert_eq!(tool_executions.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), before + 2);
    let committed: Vec<String> = sqlx::query_scalar("SELECT role FROM conversation_messages WHERE json_extract(metadata, '$.turn_id')=? ORDER BY id")
        .bind(group.turn_id.as_deref()).fetch_all(&state.db).await.unwrap();
    assert_eq!(committed, ["assistant", "tool", "assistant"]);
    assert!(recovered_exchange.assistant_message_id.is_some());

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
            KokoroError::ChatPersistence("fixture".into()),
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
        if expected_stage == "final_save" {
            assert_eq!(
                snapshot.error_code,
                Some(KokoroError::CHAT_PERSISTENCE_CODE)
            );
        } else {
            assert_non_retryable_diagnostic(
                &state.db,
                snapshot.turn_id.as_deref().unwrap(),
                expected_stage,
            )
            .await;
        }
        assert_eq!(submit().await.unwrap_err(), error);
        assert_eq!(tool_executions.load(Ordering::Relaxed), before + 1);
    }
    sqlx::query("DROP TRIGGER refuse_final_reply")
        .execute(&state.db)
        .await
        .unwrap();
    let calls_before_recovery = provider.prompts.lock().unwrap().len();
    let tools_before_recovery = tool_executions.load(Ordering::Relaxed);
    let recovered = submit_message(
        &handle,
        ChatId(10),
        14,
        "[after-tool-final-save]",
        Vec::new(),
        &private,
        "test-character",
    )
    .await
    .unwrap();
    assert!(recovered.assistant_message_id.is_some());
    assert_eq!(
        provider.prompts.lock().unwrap().len(),
        calls_before_recovery
    );
    assert_eq!(
        tool_executions.load(Ordering::Relaxed),
        tools_before_recovery
    );

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
    crate::chat::application::cancel_chat_turn_inner(
        "pending".into(),
        None,
        cancellation,
        handle.state::<Arc<PendingToolApprovalState>>().inner(),
    )
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

    verify_external_tool_only_replies(&handle, &provider, &tool_executions).await;
    verify_audio_admission(&handle, &provider).await;
    verify_hidden_turn_scope(&handle, &tool_executions).await;
}

/// Exercise persisted identity and memory metadata through the real production
/// handlers. Only the terminal journal is reset; SQLite remains authoritative.
async fn verify_persisted_external_contracts(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
    tools: &AtomicUsize,
) {
    use crate::chat::persistence::{ConversationTarget, MessageWrite};
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    let mut defects = Vec::new();
    for character in ["test-character", "other-character"] {
        sqlx::query("INSERT INTO characters (id,name,persona,user_nickname,source_format,created_at,updated_at) VALUES (?,?, '', '', 'test',1,1)")
            .bind(character).bind(character).execute(&state.db).await.unwrap();
    }
    let body = json!({"request_id":"format-committed","conversation_id":"review-contract","text":"[formatted-replay]"});
    let first = webhook_body(app, body.clone()).await;
    assert_eq!(first.0, 200, "{first:?}");
    assert_eq!(
        first.1["reply"],
        "Review formatted answer\n\nSecond paragraph"
    );
    let canonical: String = sqlx::query_scalar(
        "SELECT content FROM conversation_messages WHERE role='assistant' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(
        canonical,
        "**Review formatted answer**\n\n*Second paragraph*"
    );
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    runtime.forget_completed_for_restart_test();
    let replay = webhook_body(app, body.clone()).await;
    if replay != first {
        defects.push(format!(
            "P3 persisted replay differs: {replay:?} versus {first:?}"
        ));
    }
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );

    sqlx::query("CREATE TRIGGER reject_contract_reply BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' AND json_extract(NEW.metadata,'$.type') IS NULL BEGIN SELECT RAISE(ABORT,'contract save failure'); END").execute(&state.db).await.unwrap();
    let mut pending = body.clone();
    pending["request_id"] = "format-pending".into();
    assert_eq!(
        webhook_body(app, pending.clone()).await.1["code"],
        KokoroError::CHAT_PERSISTENCE_CODE
    );
    sqlx::query("DROP TRIGGER reject_contract_reply")
        .execute(&state.db)
        .await
        .unwrap();
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    let recovered = webhook_body(app, pending).await;
    if recovered != first {
        defects.push(format!(
            "P3 save recovery differs: {recovered:?} versus {first:?}"
        ));
    }
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );

    runtime.forget_completed_for_restart_test();
    state.set_character_id("other-character".into()).await;
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    let foreign = webhook_body(app, body).await;
    if foreign.0 != 409
        || (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed),
        ) != before
    {
        defects.push(format!(
            "P1 cross-character persisted request executed: {foreign:?}"
        ));
    }
    // Original PR2 uses SHA256(JSON(owner,id)), whereas main uses length prefixes.
    for (id, character) in [
        ("legacy-owned", "test-character"),
        ("legacy-foreign", "other-character"),
    ] {
        use sha2::{Digest, Sha256};
        let legacy = format!(
            "webhook-{:x}",
            Sha256::digest(serde_json::to_vec(&("private:unknown:review-contract", id)).unwrap())
        );
        sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES ('desktop','user','old accepted input',?,'now')")
            .bind(json!({"source":"webhook","client_request_id":legacy}).to_string()).execute(&state.db).await.unwrap();
        state.set_character_id(character.into()).await;
        runtime.forget_completed_for_restart_test();
        let before = (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed),
        );
        let reply = webhook_body(app, json!({"request_id":id,"conversation_id":"review-contract","text":"old accepted input"})).await;
        if reply.0 != 409
            || (
                provider.prompts.lock().unwrap().len(),
                tools.load(Ordering::Relaxed),
            ) != before
        {
            defects.push(format!("P1 legacy request {id} executed: {reply:?}"));
        }
    }

    state.set_character_id("test-character".into()).await;
    state.set_memory_enabled(true).await;
    crate::commands::chat::stream_chat(
        app.clone(),
        ChatRequest {
            message: "[origin-desktop]".into(),
            character_id: Some("test-character".into()),
            conversation_id: Some("desktop".into()),
            client_request_id: Some("origin-desktop".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let continued = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
    };
    submit_message(
        app,
        ChatId(777),
        1,
        "[origin-shared-external]",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap();
    // Telegram goes directly through shared admission (without BotRequestBinding).
    runtime.forget_completed_for_restart_test();
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    let rejected = submit_message(
        app,
        ChatId(777),
        1,
        "[origin-shared-external]",
        vec![],
        &SessionMode::Private,
        "other-character",
    )
    .await;
    assert!(
        matches!(rejected, Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
    );
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );
    for (id, character) in [(98, "test-character"), (99, "other-character")] {
        use sha2::{Digest, Sha256};
        let legacy = format!(
            "telegram-{:x}",
            Sha256::digest(serde_json::to_vec(&("777", id.to_string())).unwrap())
        );
        sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES ('desktop','user','old Telegram input',?,'now')")
            .bind(json!({"source":"telegram","client_request_id":legacy}).to_string()).execute(&state.db).await.unwrap();
        runtime.forget_completed_for_restart_test();
        let rejected = submit_message(
            app,
            ChatId(777),
            id,
            "old Telegram input",
            vec![],
            &SessionMode::Private,
            character,
        )
        .await;
        assert!(
            matches!(rejected, Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
        );
        assert_eq!(
            (
                provider.prompts.lock().unwrap().len(),
                tools.load(Ordering::Relaxed)
            ),
            before
        );
    }
    let target = ConversationTarget {
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
        create: false,
    };
    // Existing main metadata and original PR2 source metadata remain readable.
    for (role, content, metadata) in [
        (
            "user",
            "old-main-external",
            json!({"chat_save_id":"old-main:user"}),
        ),
        (
            "user",
            "old-desktop",
            json!({"chat_save_id":"old-desktop:user","client_request_id":"old-desktop"}),
        ),
        (
            "user",
            "old-pr2-external",
            json!({"source":"telegram","client_request_id":"old-pr2"}),
        ),
        (
            "assistant",
            "old-pr2-assistant",
            json!({"source":"telegram"}),
        ),
    ] {
        let message = MessageWrite::new(role, content, Some(metadata.to_string()));
        crate::chat::persistence::save(&state.db, &target, &[message])
            .await
            .unwrap();
        state
            .push_history_message(crate::ai::context::Message {
                role: role.into(),
                content: content.into(),
                metadata: Some(metadata),
            })
            .await;
    }
    let external = state
        .saved_external_memory_history(&target, 0, i64::MAX)
        .await
        .unwrap();
    for (role, text) in [
        ("user", "[origin-shared-external]"),
        ("user", "old-main-external"),
        ("user", "old-pr2-external"),
        ("assistant", "old-pr2-assistant"),
    ] {
        if !external
            .iter()
            .any(|row| row.role == role && row.content == text)
        {
            defects.push(format!("P2 external SQL lost {text}"));
        }
    }
    assert!(!external
        .iter()
        .any(|row| row.content == "[origin-desktop]" || row.content == "old-desktop"));
    let desktop = state.get_recent_memory_history(100).await;
    for text in [
        "[origin-shared-external]",
        "old-main-external",
        "old-pr2-external",
        "old-pr2-assistant",
    ] {
        if desktop.iter().any(|row| row.content == text) {
            defects.push(format!("P2 desktop SQL admitted {text}"));
        }
    }
    assert!(desktop.iter().any(|row| row.content == "[origin-desktop]"));
    assert!(desktop.iter().any(|row| row.content == "old-desktop"));
    state.set_memory_history_boundary(0).await; // exercise the in-memory fallback too
    let fallback = state.get_recent_memory_history(100).await;
    for text in [
        "[origin-shared-external]",
        "old-main-external",
        "old-pr2-external",
        "old-pr2-assistant",
    ] {
        if fallback.iter().any(|row| row.content == text) {
            defects.push(format!("P2 desktop memory admitted {text}"));
        }
    }
    let webhook_conversation: String = sqlx::query_scalar("SELECT conversation_id FROM conversation_messages WHERE role='user' AND content='[formatted-replay]' ORDER BY id LIMIT 1").fetch_one(&state.db).await.unwrap();
    let webhook_target = ConversationTarget {
        conversation_id: webhook_conversation,
        ..target
    };
    let history = state
        .saved_external_memory_history(&webhook_target, 0, i64::MAX)
        .await
        .unwrap();
    if !history
        .iter()
        .any(|row| row.role == "user" && row.content == "[formatted-replay]")
    {
        defects.push("P2 actual Webhook user absent from extraction history".into());
    }
    assert!(
        defects.is_empty(),
        "Production contract regressions: {defects:#?}"
    );

    // Return this shared fixture to the initial state for the existing scenarios.
    state.set_memory_enabled(false).await;
    state.history.lock().await.clear();
    state.set_memory_history_boundary(0).await;
    runtime.forget_completed_for_restart_test();
    sqlx::query("DELETE FROM conversation_messages")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM conversations WHERE id != 'desktop'")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM characters WHERE id IN ('test-character','other-character')")
        .execute(&state.db)
        .await
        .unwrap();
    provider.prompts.lock().unwrap().clear();
    tools.store(0, Ordering::Relaxed);
}

async fn verify_hidden_turn_scope(app: &tauri::AppHandle, executions: &AtomicUsize) {
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    let settings = app.state::<Arc<RwLock<ToolSettings>>>();
    for (approve, fail_save) in [(false, false), (true, false), (true, true)] {
        settings.write().await.max_permission_level = ActionPermissionLevel::Safe;
        let generation = {
            let _selection = state.conversation_switch_lock.lock().await;
            *state.current_conversation_id.lock().await = None;
            state.history.lock().await.clear();
            state.bump_conversation_generation()
        };
        let request = ChatRequest {
            message: "[hidden-scope]".into(),
            hidden: true,
            client_request_id: Some(format!("hidden-scope-{approve}-{fail_save}")),
            character_id: Some("test-character".into()),
            conversation_generation: Some(generation),
            ..Default::default()
        };
        let before = executions.load(Ordering::Relaxed);
        let conversations_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
            .fetch_one(&state.db)
            .await
            .unwrap();
        if fail_save {
            sqlx::query("CREATE TRIGGER refuse_hidden_final BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'hidden save fault'); END").execute(&state.db).await.unwrap();
        }
        let worker = tokio::spawn(crate::commands::chat::stream_chat(
            app.clone(),
            request.clone(),
        ));
        let (view, approval_id) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let view = super::runtime::get_chat_view(&state, &runtime, None)
                    .await
                    .unwrap()
                    .unwrap();
                let approval_id = view
                    .turns
                    .iter()
                    .find(|turn| {
                        turn.client_request_id == request.client_request_id.as_deref().unwrap()
                    })
                    .and_then(|turn| {
                        turn.tools
                            .iter()
                            .find(|event| event["approval_status"] == "requested")
                    })
                    .and_then(|event| event["approval_request_id"].as_str())
                    .map(str::to_owned);
                if let Some(approval_id) = approval_id {
                    break (view, approval_id);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let turn = view
            .turns
            .iter()
            .find(|turn| turn.client_request_id == request.client_request_id.as_deref().unwrap())
            .unwrap();
        // Tool rows now join the final atomic write: until commit, a hidden
        // turn keeps its original None scope for streaming, approval and Stop.
        assert!(view.conversation_id.is_none());
        assert_eq!(view.version.generation, generation);
        assert_eq!(
            turn.character_id.as_deref(),
            Some(view.character_id.as_str())
        );
        assert_eq!(turn.conversation_id, view.conversation_id);
        assert_eq!(turn.conversation_generation, Some(view.version.generation));
        assert_eq!(turn.status, "streaming");
        assert!(turn.text.contains("Waiting for approval"));
        assert_eq!(executions.load(Ordering::Relaxed), before + 1);
        assert!(
            turn.user_message_id.is_none(),
            "Hidden prompt must stay hidden"
        );
        if approve {
            // Execution rechecks the current permission ceiling after approval.
            settings.write().await.max_permission_level = ActionPermissionLevel::Elevated;
            crate::commands::chat::approve_tool_approval(approval_id.clone(), app.state())
                .await
                .unwrap();
        } else {
            crate::commands::chat::cancel_chat_turn(
                turn.client_request_id.clone(),
                None,
                app.state(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
        }
        let outcome = tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        let result = if fail_save {
            assert!(matches!(outcome, Err(KokoroError::ChatPersistence(_))));
            assert!(state.current_conversation_id.lock().await.is_none());
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
                .fetch_one(&state.db)
                .await
                .unwrap();
            assert_eq!(
                count, conversations_before,
                "Failed hidden commit must not leave an empty conversation"
            );
            let before_retry = executions.load(Ordering::Relaxed);
            sqlx::query("DROP TRIGGER refuse_hidden_final")
                .execute(&state.db)
                .await
                .unwrap();
            let recovered = super::application::retry_chat_persistence(
                app.clone(),
                request.client_request_id.clone().unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(executions.load(Ordering::Relaxed), before_retry);
            recovered
        } else {
            outcome.unwrap()
        };
        assert_eq!(
            result.status.as_deref(),
            Some(if approve { "completed" } else { "cancelled" })
        );
        assert_eq!(
            executions.load(Ordering::Relaxed),
            before + if approve { 2 } else { 1 }
        );
        assert!(
            !app.state::<Arc<PendingToolApprovalState>>()
                .is_pending(&approval_id)
                .await
        );
        let scope = state.conversation_state_snapshot().await;
        assert_eq!(
            crate::commands::chat::stream_chat(app.clone(), request)
                .await
                .unwrap(),
            result
        );
        assert_eq!(
            state.conversation_state_snapshot().await,
            scope,
            "Replay must not select another scope"
        );
        if approve {
            let view = super::runtime::get_chat_view(&state, &runtime, None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                view.conversation_id.as_deref(),
                Some(result.conversation_id.as_str())
            );
            assert_eq!(view.version.generation, generation + 1);
            let turn = view
                .turns
                .iter()
                .find(|turn| turn.client_request_id == result.client_request_id.as_deref().unwrap())
                .unwrap();
            assert_eq!(turn.conversation_id, view.conversation_id);
            assert_eq!(turn.conversation_generation, Some(view.version.generation));
            assert_eq!(
                persisted_assistant_text(&state.db, result.assistant_message_id.unwrap()).await,
                result.text
            );
        } else {
            assert!(state.current_conversation_id.lock().await.is_none());
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
                .fetch_one(&state.db)
                .await
                .unwrap();
            assert_eq!(
                count, conversations_before,
                "Cancelled hidden tools must not leave an empty conversation"
            );
        }
    }
}

async fn verify_external_tool_only_replies(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
    executions: &AtomicUsize,
) {
    let settings = app.state::<Arc<RwLock<ToolSettings>>>();
    let previous = settings.read().await.max_tool_rounds;
    settings.write().await.max_tool_rounds = 1;
    let state = app.state::<AIOrchestrator>();
    let db = &state.db;
    let previous_language = state.response_language.lock().await.clone();
    for (index, (language, text)) in [
        ("English", "[tool-only]"),
        ("English", "[native-tool-only]"),
        ("Русский", "[tool-only]"),
        ("Русский", "[native-tool-only]"),
    ]
    .into_iter()
    .enumerate()
    {
        *state.response_language.lock().await = language.into();
        let request_id = format!("{language}:{text}");
        let before = executions.load(Ordering::Relaxed);
        let calls = provider.prompts.lock().unwrap().len();
        let first = webhook(app, &request_id, &request_id, text, None).await;
        assert_eq!(first.0, 200);
        let notice = first.1["reply"].as_str().unwrap();
        assert!(!notice.is_empty());
        let catalog: Value = serde_json::from_str(if language == "Русский" {
            include_str!("../../../src/ui/locales/ru.json")
        } else {
            include_str!("../../../src/ui/locales/en.json")
        })
        .unwrap();
        assert_eq!(
            notice,
            catalog["chat"]["tool_only_completion"].as_str().unwrap()
        );
        assert!(
            !notice.contains("sentinel executed"),
            "Do not expose model-only tool output"
        );
        assert_eq!(
            webhook(app, &request_id, &request_id, text, None).await,
            first
        );
        assert_eq!(executions.load(Ordering::Relaxed), before + 1);
        assert_eq!(provider.prompts.lock().unwrap().len(), calls + 1);
        let saved: String = sqlx::query_scalar("SELECT content FROM conversation_messages WHERE role = 'assistant' ORDER BY id DESC LIMIT 1")
            .fetch_one(db).await.unwrap();
        assert_eq!(notice, saved);

        let session = SessionMode::Private;
        let submit = || {
            submit_message(
                app,
                // Keep each language/tool-mode fixture free of earlier tool history.
                ChatId(100 + index as i64),
                100 + index as i32,
                text,
                vec![],
                &session,
                "test-character",
            )
        };
        let reply = submit().await.unwrap();
        assert_eq!(reply.text, notice);
        assert_eq!(
            persisted_assistant_text(db, reply.assistant_message_id.unwrap()).await,
            notice
        );
        let replay = submit().await.unwrap();
        assert!(reply.newly_committed);
        assert!(
            !replay.newly_committed,
            "Only the first committed delivery may start optional media"
        );
        let mut expected_replay = reply.clone();
        expected_replay.newly_committed = false;
        assert_eq!(replay, expected_replay);
        assert_eq!(executions.load(Ordering::Relaxed), before + 2);
        assert_eq!(provider.prompts.lock().unwrap().len(), calls + 2);
    }
    *state.response_language.lock().await = previous_language;
    settings.write().await.max_tool_rounds = previous;
}

async fn verify_audio_admission(app: &tauri::AppHandle, provider: &ControlledProvider) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let route = warp::post().and(warp::body::bytes()).and_then({
        let (calls, mode, entered, release) = (
            calls.clone(),
            mode.clone(),
            entered.clone(),
            release.clone(),
        );
        move |_: bytes::Bytes| {
            let (calls, mode, entered, release) = (
                calls.clone(),
                mode.clone(),
                entered.clone(),
                release.clone(),
            );
            async move {
                let call = calls.fetch_add(1, Ordering::Relaxed) + 1;
                let mode = mode.load(Ordering::Relaxed);
                if mode == 2 {
                    entered.notify_one();
                    release.notified().await;
                }
                if mode == 4 {
                    std::future::pending::<()>().await;
                }
                let (status, text) = match mode {
                    1 => (
                        warp::http::StatusCode::SERVICE_UNAVAILABLE,
                        "unavailable".into(),
                    ),
                    3 => (warp::http::StatusCode::OK, String::new()),
                    _ => (
                        warp::http::StatusCode::OK,
                        format!("voice transcript {call}"),
                    ),
                };
                Ok::<_, std::convert::Infallible>(warp::reply::with_status(
                    warp::reply::json(&json!({"text": text})),
                    status,
                ))
            }
        }
    });
    let (address, server) = warp::serve(route).bind_ephemeral(([127, 0, 0, 1], 0));
    let server = tokio::spawn(server);
    let config = crate::stt::config::SttConfig {
        providers: vec![serde_json::from_value(json!({
            "id": "openai_whisper", "provider_type": "openai_whisper", "api_key": "fixture",
            "base_url": format!("http://{address}/v1"), "model": "fixture"
        }))
        .unwrap()],
        ..Default::default()
    };
    app.manage(crate::stt::SttService::init_from_config(&config).await);
    let body = json!({"request_id": "audio-first", "conversation_id": "audio", "audio_base64": "AQID", "audio_format": "wav"});
    let first = webhook_body(app, body.clone()).await;
    assert_eq!(first.0, 200, "{first:?}");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(provider
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .to_string()
        .contains("voice transcript 1"));
    let model_calls = provider.prompts.lock().unwrap().len();
    mode.store(1, Ordering::Relaxed);
    assert_eq!(webhook_body(app, body.clone()).await, first);
    for (field, value) in [
        ("audio_base64", "BAUG"),
        ("audio_format", "mp3"),
        ("text", "caption"),
        ("image", "data:image/png;base64,AAAB"),
    ] {
        let mut conflicting = body.clone();
        conflicting[field] = value.into();
        assert_eq!(webhook_body(app, conflicting).await.0, 409);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.prompts.lock().unwrap().len(), model_calls);

    let mut failed = body.clone();
    failed["request_id"] = "audio-failed".into();
    let failure = webhook_body(app, failed.clone()).await;
    assert_eq!(failure.0, 500);
    // The HTTP provider may retry a 503 within one transcription attempt.
    let failed_calls = calls.load(Ordering::Relaxed);
    assert_eq!(webhook_body(app, failed.clone()).await, failure);
    assert!(calls.load(Ordering::Relaxed) > failed_calls);
    mode.store(0, Ordering::Relaxed);
    assert_eq!(webhook_body(app, failed).await.0, 200);
    assert_eq!(provider.prompts.lock().unwrap().len(), model_calls + 1);

    mode.store(3, Ordering::Relaxed);
    let mut empty = body.clone();
    empty["request_id"] = "audio-empty".into();
    assert_eq!(webhook_body(app, empty).await.0, 400);

    mode.store(2, Ordering::Relaxed);
    let mut pending = body.clone();
    pending["request_id"] = "audio-pending".into();
    let first = tokio::spawn({
        let app = app.clone();
        let pending = pending.clone();
        async move { webhook_body(&app, pending).await }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let before = calls.load(Ordering::Relaxed);
    let duplicate = tokio::spawn({
        let app = app.clone();
        let pending = pending.clone();
        async move { webhook_body(&app, pending).await }
    });
    let mut busy = body.clone();
    busy["request_id"] = "audio-busy".into();
    assert_eq!(webhook_body(app, busy).await.0, 503);
    assert_eq!(
        webhook(app, "text-busy", "audio", "busy during transcription", None)
            .await
            .0,
        503
    );
    assert_eq!(calls.load(Ordering::Relaxed), before);
    first.abort(); // Only the subscriber is dropped; transcription stays application-owned.
    release.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(5), duplicate)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.0, 200);
    assert_eq!(webhook_body(app, pending).await, completed);
    assert_eq!(calls.load(Ordering::Relaxed), before);

    let mut cancelled = body.clone();
    cancelled["request_id"] = "audio-cancelled".into();
    cancelled["text"] = "cancel this voice".into();
    let task = tokio::spawn({
        let app = app.clone();
        let cancelled = cancelled.clone();
        async move { webhook_body(&app, cancelled).await }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    let snapshot = runtime
        .snapshots()
        .into_iter()
        .find(|turn| turn.message == "cancel this voice")
        .unwrap();
    crate::chat::application::cancel_chat_turn_inner(
        snapshot.client_request_id,
        None,
        app.state::<Arc<TurnCancellationState>>().inner().clone(),
        app.state::<Arc<PendingToolApprovalState>>().inner(),
    )
    .await
    .unwrap();
    let cancelled_result = task.await.unwrap();
    assert_eq!(cancelled_result.0, 500);
    assert_eq!(webhook_body(app, cancelled).await, cancelled_result);
    release.notify_one();

    let mut timed_out = body;
    mode.store(4, Ordering::Relaxed);
    timed_out["request_id"] = "audio-timeout".into();
    let timeout_started = std::time::Instant::now();
    let timeout = webhook_body(app, timed_out.clone()).await;
    assert_eq!(timeout.0, 500);
    assert!(timeout_started.elapsed() >= Duration::from_secs(10));
    assert!(
        timeout.1["error"]
            .as_str()
            .unwrap()
            .contains("Failed to transcribe audio"),
        "{timeout:?}"
    );
    assert_eq!(webhook_body(app, timed_out).await, timeout);
    assert_eq!(
        webhook(
            app,
            "after-audio-timeout",
            "audio",
            "free after timeout",
            None
        )
        .await
        .0,
        200
    );
    release.notify_one();
    server.abort();
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
