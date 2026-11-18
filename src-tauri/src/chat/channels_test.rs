use super::ownership_test::verify_state_ownership;
use crate::actions::registry::{ActionError, ActionHandler, ActionParam, ActionPermissionLevel};
use crate::actions::{tool_settings::ToolSettings, ActionContext, ActionRegistry, ActionResult};
use crate::ai::context::AIOrchestrator;
use crate::bot::{handle_generic_webhook, BotChatServices, WebhookBotConfig};
use crate::characters::activation::ActivationCoordinator;
use crate::chat::application::{
    ChatRequest, ChatService, PendingToolApprovalState, TurnCancellationState,
};
use crate::chat::runtime::ChatRuntimeState;
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

#[derive(Default)]
pub(super) struct ToolScopes(pub std::sync::Mutex<Vec<ActionContext>>);
struct RecordOwner(Arc<ToolScopes>);
#[async_trait::async_trait]
impl ActionHandler for RecordOwner {
    fn name(&self) -> &str {
        "record_owner"
    }
    fn description(&self) -> &str {
        "Record bound identity"
    }
    fn parameters(&self) -> Vec<ActionParam> {
        vec![]
    }
    async fn execute(
        &self,
        _: HashMap<String, String>,
        context: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        self.0 .0.lock().unwrap().push(context);
        Ok(ActionResult::ok("owner recorded"))
    }
}

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
pub(super) struct ControlledProvider {
    pub(super) prompts: Mutex<Vec<Value>>,
    entered_pending: Notify,
    pub(super) finish_pending: Notify,
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
        if last_user.contains("[reserved-memory]") {
            let messages = payload.as_array().unwrap();
            let user_index = messages
                .iter()
                .rposition(|message| message["role"] == "user")
                .unwrap();
            let continued = messages[user_index + 1..]
                .iter()
                .any(|message| message["role"] == "tool");
            self.prompts.lock().unwrap().push(payload);
            if !continued {
                self.entered_pending.notify_one();
                self.finish_pending.notified().await;
            }
            let event = if continued {
                LlmStreamEvent::Text("Memory tool finished".into())
            } else {
                LlmStreamEvent::ToolCall(LlmToolCall {
                    id: "reserved-memory-call".into(),
                    name: "store_memory".into(),
                    args: HashMap::from([("fact".into(), "User collects model trains.".into())]),
                })
            };
            return Ok(Box::pin(stream::iter([Ok(event)])));
        }
        if last_user.contains("[owner-approval]") {
            let messages = payload.as_array().unwrap();
            let user_index = messages
                .iter()
                .rposition(|message| message["role"] == "user")
                .unwrap();
            let continued = messages[user_index + 1..]
                .iter()
                .any(|message| message["role"] == "tool");
            self.prompts.lock().unwrap().push(payload);
            let events = if continued {
                vec![LlmStreamEvent::Text("owner action completed".into())]
            } else {
                vec![
                    LlmStreamEvent::Text("owner action pending".into()),
                    LlmStreamEvent::ToolCall(LlmToolCall {
                        id: "owner-write".into(),
                        name: "owner_write".into(),
                        args: HashMap::new(),
                    }),
                ]
            };
            return Ok(Box::pin(stream::iter(events.into_iter().map(Ok))));
        }
        if last_user.contains("[ownership-tools]") {
            let messages = payload.as_array().unwrap();
            let user_index = messages
                .iter()
                .rposition(|message| message["role"] == "user")
                .unwrap();
            let continued = messages[user_index + 1..]
                .iter()
                .any(|message| message["role"] == "tool");
            self.prompts.lock().unwrap().push(payload);
            if !continued {
                self.entered_pending.notify_one();
                self.finish_pending.notified().await;
            }
            let event = if continued {
                LlmStreamEvent::Text("owned continuation".into())
            } else {
                LlmStreamEvent::ToolCall(LlmToolCall {
                    id: "owned-call".into(),
                    name: "record_owner".into(),
                    args: HashMap::new(),
                })
            };
            return Ok(Box::pin(stream::iter([Ok(event)])));
        }
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
            "[memory-native]",
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
        app.state::<BotChatServices>().inner(),
    )
    .await;
    let status = response.status().as_u16();
    let bytes = warp::hyper::body::to_bytes(response.into_body())
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

pub(super) async fn until_request_started(provider: &ControlledProvider) {
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

    let mut state = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    Arc::get_mut(&mut state.selection)
        .unwrap()
        .persist_conversation_selection = false;
    state.set_memory_enabled(false).await;
    state.set_character_id("test-character".into()).await;
    let now = "2026-11-14T00:00:00Z";
    sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('desktop', 'test-character', 'desktop', ?, ?)")
        .bind(now).bind(now).execute(&state.db).await.unwrap();
    *state.selection.current_conversation_id.lock().await = Some("desktop".into());
    let provider = Arc::new(ControlledProvider::default());
    let mut registry = ActionRegistry::new();
    crate::actions::builtin::register_builtins(&mut registry);
    let tool_executions = Arc::new(AtomicUsize::new(0));
    registry.register(CountToolExecutions(tool_executions.clone(), false));
    registry.register(CountToolExecutions(tool_executions.clone(), true));
    let scopes = Arc::new(ToolScopes::default());
    registry.register(RecordOwner(scopes.clone()));
    handle.manage(scopes);
    let runtime = Arc::new(ChatRuntimeState::default());
    let cancellation = Arc::new(TurnCancellationState::default());
    let imagegen = ImageGenService::init_from_config(&Default::default()).await;
    handle.manage(state);
    handle.manage(runtime.clone());
    handle.manage(cancellation.clone());
    handle.manage(Arc::new(PendingToolApprovalState::default()));
    handle.manage(ActivationCoordinator::default());
    handle.manage(crate::characters::catalog::CharacterCatalog::new(
        temp.path().join("characters"),
        semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap(),
    ));
    handle.manage(imagegen);
    handle.manage(crate::tts::TtsService::new());
    handle.manage(crate::stt::SttService::new());
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
    let desktop_host = Arc::new(handle.clone());
    crate::desktop_chat::install_services(
        &handle,
        temp.path(),
        crate::desktop_chat::TauriChatDesktop::new(&desktop_host),
    );

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
        handle.state::<ChatService>().inner(),
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
        state
            .selection
            .current_conversation_id
            .lock()
            .await
            .as_deref(),
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
        handle.state::<BotChatServices>().inner(),
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
        handle.state::<ChatService>().inner(),
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
        handle.state::<ChatService>().inner(),
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
            handle.state::<ChatService>().inner(),
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
                handle.state::<ChatService>().inner(),
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
        handle.state::<ChatService>().inner(),
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
        handle.state::<ChatService>().inner(),
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
        state
            .selection
            .current_conversation_id
            .lock()
            .await
            .as_deref(),
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
        handle.state::<ChatService>().inner(),
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
            .unwrap()
            .0,
        result
    );
    let before_rejected_replay = (
        provider.prompts.lock().unwrap().len(),
        tool_executions.load(Ordering::Relaxed),
    );
    for index in 0..256 {
        let id = format!("evict-first-busy-{index}");
        runtime
            .begin(&ChatRequest {
                message: "fixture".into(),
                client_request_id: Some(id.clone()),
                ..Default::default()
            })
            .unwrap();
        runtime.complete(
            &id,
            Ok(super::application::StreamChatResponse {
                status: Some("completed".into()),
                ..Default::default()
            }),
        );
    }
    assert!(!runtime
        .snapshots()
        .iter()
        .any(|snapshot| snapshot.message == "same conversation"));
    let rejected_again = submit_message(
        handle.state::<ChatService>().inner(),
        ChatId(10),
        4,
        "same conversation",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap_err();
    assert_eq!(rejected_again.to_string(), other.to_string());
    let conflicting_replay = submit_message(
        handle.state::<ChatService>().inner(),
        ChatId(10),
        4,
        "changed rejected input",
        vec![],
        &continued,
        "test-character",
    )
    .await;
    assert!(
        matches!(conflicting_replay, Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
    );
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tool_executions.load(Ordering::Relaxed)
        ),
        before_rejected_replay
    );
    let activation = state.acquire_activation_lock().await;
    let during_activation = submit_message(
        handle.state::<ChatService>().inner(),
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
        handle.state::<ChatService>().inner(),
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
            .unwrap()
            .0,
        result
    );
    assert_eq!(
        persisted_assistant_text(&state.db, result.assistant_message_id.unwrap()).await,
        result.text
    );

    verify_external_tool_only_replies(&handle, &provider, &tool_executions).await;
    verify_audio_admission(&handle, &provider).await;
    verify_hidden_turn_scope(&handle, &tool_executions).await;
    verify_state_ownership(&handle, &provider, temp.path()).await;
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
    super::ownership_test::verify_snapshot_preparation_cancellation(app, provider).await;
    super::ownership_test::verify_reserved_memory_tools(app, provider).await;
    super::ownership_test::verify_admission_selection_generation(app, provider).await;
    verify_legacy_metadata_and_retired_media(app, provider, tools).await;
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
    assert_ne!(
        recovered.1["request_id"], first.1["request_id"],
        "different logical inputs keep distinct request identities"
    );
    if recovered.0 != first.0
        || recovered.1["reply"] != first.1["reply"]
        || recovered.1["character_id"] != first.1["character_id"]
        || recovered.1["conversation_id"] != first.1["conversation_id"]
    {
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
        app.state::<ChatService>().inner(),
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
        app.state::<ChatService>().inner(),
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
            app.state::<ChatService>().inner(),
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
        require_character: false,
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
        require_character: false,
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
    verify_shared_native_memory_origin(app, provider, tools).await;
    verify_external_failure_memory_origin(app, provider, tools).await;

    // Return this shared fixture to the initial state for the existing scenarios.
    state.set_memory_enabled(false).await;
    state.selection.history.lock().await.clear();
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
    // Characters remain real owners for subsequent shared channel scenarios.
    provider.prompts.lock().unwrap().clear();
    tools.store(0, Ordering::Relaxed);
}

async fn verify_legacy_metadata_and_retired_media(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
    tools: &AtomicUsize,
) {
    use crate::chat::application::StreamChatResponse;
    use crate::chat::persistence::{ExternalReplyImage, ExternalReplyMedia};
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    sqlx::query("INSERT INTO conversations (id,character_id,created_at,updated_at) VALUES ('legacy-replay','test-character','now','now')")
        .execute(&state.db).await.unwrap();
    for role in ["user", "assistant"] {
        for metadata in [None, Some(""), Some("not-json"), Some("{"), Some("null")] {
            sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES ('legacy-replay',?,'legacy text',?,'now')")
                .bind(role).bind(metadata).execute(&state.db).await.unwrap();
        }
    }
    let continued = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "legacy-replay".into(),
    };
    let first = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(778),
        1,
        "fresh legacy turn",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap();
    assert!(first.assistant_message_id.is_some());
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    runtime.forget_completed_for_restart_test();
    let replay = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(778),
        1,
        "fresh legacy turn",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap();
    assert_eq!(replay.text, first.text);
    assert_eq!(replay.assistant_message_id, first.assistant_message_id);
    assert!(!replay.newly_committed);

    // Persist fixture media after the reply entered the journal: seeing it on
    // redelivery proves eviction actually reloads SQLite instead of cached data.
    let media = ExternalReplyMedia {
        images: vec![ExternalReplyImage {
            prompt: "saved picture".into(),
            mime_type: "image/png".into(),
            file_name: "saved.png".into(),
            data_base64: "AQID".into(),
        }],
        output_warnings: vec!["fixture warning".into()],
        ..Default::default()
    };
    sqlx::query("UPDATE conversation_messages SET metadata=json_set(metadata,'$.external_reply_media',json(?)) WHERE id=?")
        .bind(serde_json::to_string(&media).unwrap()).bind(first.assistant_message_id)
        .execute(&state.db).await.unwrap();
    let evict = |round| {
        for index in 0..256 {
            let id = format!("eviction-{round}-{index}");
            runtime
                .begin(&ChatRequest {
                    message: "fixture".into(),
                    client_request_id: Some(id.clone()),
                    ..Default::default()
                })
                .unwrap();
            runtime.complete(
                &id,
                Ok(StreamChatResponse {
                    status: Some("completed".into()),
                    ..Default::default()
                }),
            );
        }
        assert!(runtime
            .turn_scope(first.client_request_id.as_deref().unwrap())
            .is_none());
    };
    evict(0);
    let conflict = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(778),
        1,
        "changed text",
        vec![],
        &continued,
        "test-character",
    )
    .await;
    assert!(
        matches!(conflict, Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
    );
    let replay = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(778),
        1,
        "fresh legacy turn",
        vec![],
        &continued,
        "test-character",
    )
    .await
    .unwrap();
    assert_eq!(replay.text, first.text);
    assert_eq!(replay.external_media, media);
    assert_eq!(replay.output_warnings, media.output_warnings);
    assert_eq!(replay.assistant_message_id, first.assistant_message_id);
    assert!(!replay.newly_committed);

    // Deleting history must not turn a compact tombstone into a fresh request.
    evict(1);
    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id='legacy-replay'")
        .execute(&state.db)
        .await
        .unwrap();
    let missing = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(778),
        1,
        "fresh legacy turn",
        vec![],
        &continued,
        "test-character",
    )
    .await;
    assert!(
        matches!(missing, Err(KokoroError::Chat(error)) if error.starts_with("chat_request_conflict:"))
    );
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id='legacy-replay'"
        )
        .fetch_one(&state.db)
        .await
        .unwrap(),
        0
    );
    sqlx::query("DELETE FROM conversations WHERE id='legacy-replay'")
        .execute(&state.db)
        .await
        .unwrap();
    runtime.forget_completed_for_restart_test();
}

async fn verify_external_failure_memory_origin(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
    tools: &AtomicUsize,
) {
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    state.set_memory_enabled(false).await;
    state.set_memory_enabled(true).await;
    crate::commands::chat::stream_chat(
        app.clone(),
        ChatRequest {
            message: "desktop memory before external failure".into(),
            character_id: Some("test-character".into()),
            conversation_id: Some("desktop".into()),
            client_request_id: Some("failure-memory-desktop".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let contents = |messages: Vec<crate::ai::context::Message>| {
        messages
            .into_iter()
            .map(|message| (message.role, message.content))
            .collect::<Vec<_>>()
    };
    let baseline = contents(state.get_recent_memory_history(20).await);
    assert!(baseline
        .iter()
        .any(|(_, content)| content == "desktop memory before external failure"));
    let mode = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
    };
    let failure = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(779),
        1,
        "[after-tool-stream-failure]",
        vec![],
        &mode,
        "test-character",
    )
    .await;
    assert!(
        matches!(failure, Err(KokoroError::Llm(error)) if error == "fixture post-tool stream failed")
    );
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    let snapshot = runtime
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.message == "[after-tool-stream-failure]")
        .unwrap();
    let (diagnostic_id, diagnostic_content, raw): (i64, String, String) = sqlx::query_as("SELECT id,content,metadata FROM conversation_messages WHERE conversation_id='desktop' AND role='system' AND json_extract(metadata,'$.event.turn_id')=?")
        .bind(snapshot.turn_id.as_deref()).fetch_one(&state.db).await.unwrap();
    let metadata: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(metadata["source"], "telegram");
    assert_eq!(
        metadata["external_user_message_id"],
        snapshot.user_message_id.unwrap()
    );
    assert_eq!(metadata["event"]["retryable"], false);
    assert!(metadata["user_message_id"].is_null());
    assert!(metadata.get("keep_through_message_id").is_none());
    assert_eq!(
        contents(state.get_recent_memory_history(20).await),
        baseline
    );

    // The independent origin marker must not grant diagnostic regeneration.
    let retry = super::retry::FailureRetry {
        diagnostic_message_id: diagnostic_id,
        expected_tail_message_id: diagnostic_id,
        expected_history_revision: None,
    };
    assert!(matches!(
        super::retry::RegenerationPlan::prepare(
            &state,
            &runtime,
            super::retry::Regeneration::Failure(&retry),
            "test-character",
            "desktop",
            "[after-tool-stream-failure]"
        )
        .await,
        Err(KokoroError::Validation(_))
    ));
    assert!(submit_message(
        app.state::<ChatService>().inner(),
        ChatId(779),
        1,
        "[after-tool-stream-failure]",
        vec![],
        &mode,
        "test-character"
    )
    .await
    .is_err());
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );

    // More diagnostics than the shared window must still be filtered in SQL
    // before LIMIT, so the earlier desktop input remains available to memory.
    sqlx::query("WITH RECURSIVE copies(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM copies WHERE n<300) INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) SELECT 'desktop','system',?,?,'now' FROM copies")
        .bind(&diagnostic_content).bind(&raw).execute(&state.db).await.unwrap();
    assert_eq!(
        contents(state.get_recent_memory_history(20).await),
        baseline
    );
    // A stale UI projection must not replace the committed conversation memory window.
    state
        .sync_history_from_rows(vec![
            ("user".to_string(), "fallback desktop".to_string(), None),
            ("system".to_string(), diagnostic_content, Some(raw)),
        ])
        .await;
    state.set_memory_history_boundary(0).await;
    assert_eq!(
        contents(state.get_recent_memory_history(20).await),
        baseline
    );
}

async fn verify_shared_native_memory_origin(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
    tools: &AtomicUsize,
) {
    let state = app.state::<AIOrchestrator>();
    state.set_memory_history_boundary(0).await;
    let desktop = crate::commands::chat::stream_chat(
        app.clone(),
        ChatRequest {
            message: "[memory-native] desktop".into(),
            character_id: Some("test-character".into()),
            conversation_id: Some("desktop".into()),
            client_request_id: Some("memory-desktop".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mode = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
    };
    sqlx::query("CREATE TRIGGER reject_memory_batch BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' AND json_extract(NEW.metadata,'$.type') IS NULL BEGIN SELECT RAISE(ABORT,'memory batch failure'); END").execute(&state.db).await.unwrap();
    assert!(matches!(
        submit_message(
            app.state::<ChatService>().inner(),
            ChatId(777),
            2,
            "[memory-native] external",
            vec![],
            &mode,
            "test-character"
        )
        .await,
        Err(KokoroError::ChatPersistence(_))
    ));
    let before = (
        provider.prompts.lock().unwrap().len(),
        tools.load(Ordering::Relaxed),
    );
    sqlx::query("DROP TRIGGER reject_memory_batch")
        .execute(&state.db)
        .await
        .unwrap();
    let external = submit_message(
        app.state::<ChatService>().inner(),
        ChatId(777),
        2,
        "[memory-native] external",
        vec![],
        &mode,
        "test-character",
    )
    .await
    .unwrap();
    assert_eq!(
        (
            provider.prompts.lock().unwrap().len(),
            tools.load(Ordering::Relaxed)
        ),
        before
    );
    let desktop_user = desktop.user_message_id.unwrap();
    let external_user = external.user_message_id.unwrap();
    let target = crate::chat::persistence::ConversationTarget {
        require_character: false,
        character_id: "test-character".into(),
        conversation_id: "desktop".into(),
        create: false,
    };
    let linked = |message: &crate::ai::context::Message, id| {
        message
            .metadata
            .as_ref()
            .and_then(|meta| meta["user_message_id"].as_i64())
            == Some(id)
    };
    let technical = |message: &crate::ai::context::Message| {
        matches!(
            message
                .metadata
                .as_ref()
                .and_then(|meta| meta["type"].as_str()),
            Some("assistant_tool_calls" | "tool_result")
        )
    };
    let mut defects = Vec::new();
    let rows: Vec<(String,String,Option<String>)> = sqlx::query_as("SELECT role,content,metadata FROM conversation_messages WHERE conversation_id='desktop' AND json_extract(metadata,'$.user_message_id')=?").bind(external_user).fetch_all(&state.db).await.unwrap();
    assert_eq!(rows.len(), 2); // actual native call/result; final uses external_user_message_id
    for (_, _, metadata) in &rows {
        let metadata: Value = serde_json::from_str(metadata.as_ref().unwrap()).unwrap();
        if metadata["external_user_message_id"] != external_user {
            defects.push(format!("External batch lost origin: {metadata}"));
        }
    }
    for legacy in [false, true] {
        if legacy {
            // Real old batches used user_message_id and builtin/mcp tool source.
            sqlx::query("UPDATE conversation_messages SET metadata=json_remove(metadata,'$.external_user_message_id') WHERE conversation_id='desktop' AND json_extract(metadata,'$.user_message_id')=? AND json_extract(metadata,'$.type') IN ('assistant_tool_calls','tool_result')").bind(external_user).execute(&state.db).await.unwrap();
            for message in state
                .selection
                .history
                .lock()
                .await
                .iter_mut()
                .filter(|message| linked(message, external_user) && technical(message))
            {
                message
                    .metadata
                    .as_mut()
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("external_user_message_id");
            }
        }
        let history = state.get_recent_memory_history(20).await;
        if history.iter().any(|message| linked(message, external_user)) {
            defects.push(format!(
                "Desktop SQL includes external native batch (legacy={legacy})"
            ));
        }
        assert_eq!(
            history
                .iter()
                .filter(|message| linked(message, desktop_user) && technical(message))
                .count(),
            2
        );
    }
    // A long legacy external batch must not consume either extraction window.
    let tool = rows.iter().find(|(role, _, _)| role == "tool").unwrap();
    for index in 0..25 {
        sqlx::query("INSERT INTO conversation_messages (conversation_id,role,content,metadata,created_at) VALUES ('desktop','tool',?,json_remove(?,'$.external_user_message_id','$.chat_save_id'),'now')")
            .bind(format!("external native result {index}")).bind(&tool.2).execute(&state.db).await.unwrap();
    }
    let history = state.get_recent_memory_history(20).await;
    if !history
        .iter()
        .any(|message| message.content == "[memory-native] desktop")
    {
        defects.push("External native rows consumed the desktop SQL window".into());
    }
    let history = state
        .saved_external_memory_history(&target, external_user, i64::MAX)
        .await
        .unwrap();
    if !history
        .iter()
        .any(|message| message.role == "user" && message.content == "[memory-native] external")
        || history.iter().any(|message| technical(message))
    {
        defects.push("Native technical rows consumed the external extraction window".into());
    }
    // The user anchor can be outside the rolling in-memory window.
    let recent: Vec<_> = state
        .selection
        .history
        .lock()
        .await
        .iter()
        .filter(|message| technical(message))
        .cloned()
        .collect();
    state.sync_history_from_rows(recent).await;
    state.set_memory_history_boundary(0).await;
    let history = state.get_recent_memory_history(20).await;
    if history.iter().any(|message| linked(message, external_user)) {
        defects.push("Desktop in-memory fallback includes legacy external native rows".into());
    }
    assert_eq!(
        history
            .iter()
            .filter(|message| linked(message, desktop_user) && technical(message))
            .count(),
        2
    );
    assert!(
        defects.is_empty(),
        "Native memory origin regressions: {defects:#?}"
    );
}

async fn verify_hidden_turn_scope(app: &tauri::AppHandle, executions: &AtomicUsize) {
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    let settings = app.state::<Arc<RwLock<ToolSettings>>>();
    for (approve, fail_save) in [(false, false), (true, false), (true, true)] {
        settings.write().await.max_permission_level = ActionPermissionLevel::Safe;
        let generation = {
            let _selection = state.selection.conversation_switch_lock.lock().await;
            *state.selection.current_conversation_id.lock().await = None;
            state.selection.history.lock().await.clear();
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
        // turn owns a reserved ID, while its admission generation projects into the empty view.
        assert!(view.conversation_id.is_none());
        assert_eq!(view.version.generation, generation);
        assert_eq!(
            turn.character_id.as_deref(),
            Some(view.character_id.as_str())
        );
        assert!(turn
            .conversation_id
            .as_ref()
            .is_some_and(|id| !id.is_empty()));
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
                app.clone(),
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
            assert!(state
                .selection
                .current_conversation_id
                .lock()
                .await
                .is_none());
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
            let recovered = crate::commands::chat::retry_chat_persistence(
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
            assert!(state
                .selection
                .current_conversation_id
                .lock()
                .await
                .is_none());
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
                app.state::<ChatService>().inner(),
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
    let stt_config = tempfile::tempdir().unwrap();
    app.state::<crate::stt::SttService>()
        .save_and_reload(&stt_config.path().join("stt_config.json"), config)
        .await
        .unwrap();
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
