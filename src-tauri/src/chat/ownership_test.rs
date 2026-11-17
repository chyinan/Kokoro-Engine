use super::channels_test::{until_request_started, ControlledProvider};
use crate::ai::context::AIOrchestrator;
use crate::characters::activation::ActivationCoordinator;
use crate::chat::{application::ChatRequest, runtime::ChatRuntimeState};
use crate::telegram::bot::{submit_message, SessionMode};
use std::{sync::Arc, time::Duration};
use tauri::{Listener, Manager};
use teloxide::types::ChatId;

pub(super) async fn verify_state_ownership(
    app: &tauri::AppHandle,
    provider: &Arc<ControlledProvider>,
    directory: &std::path::Path,
) {
    use crate::chat::application::{submit_chat, ChatPolicy};
    use crate::chat::persistence::ConversationTarget;
    use crate::commands::conversation::{
        load_conversation, DeleteConversationRequest, LoadConversationRequest,
    };
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    for id in ["ownership-a", "ownership-b", "ownership-deleted"] {
        sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES (?,'test-character','now','now')").bind(id).execute(&state.db).await.unwrap();
    }
    state.set_memory_enabled(true).await;
    state
        .set_system_prompt("ORIGINAL OWNED PERSONA".into())
        .await;
    load_conversation(
        LoadConversationRequest {
            id: "ownership-a".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
    let request = |id: &str, conversation: &str, text: &str| ChatRequest {
        message: text.into(),
        client_request_id: Some(id.into()),
        character_id: Some("test-character".into()),
        conversation_id: Some(conversation.into()),
        ..Default::default()
    };
    let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listeners: Vec<_> = [
        "chat-turn-delta",
        "chat-turn-tool",
        "chat-turn-finish",
        "chat-typing",
    ]
    .into_iter()
    .map(|name| {
        let emitted = emitted.clone();
        app.listen(name, move |event| {
            let value: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            emitted.lock().unwrap().push((name, value));
        })
    })
    .collect();
    let first = tokio::spawn(submit_chat(
        app.clone(),
        request(
            "ownership-first",
            "ownership-a",
            "[ownership-tools] original owner",
        ),
        ChatPolicy::Desktop,
    ));
    until_request_started(provider).await;
    // A desktop selection transition is a real command and cannot wait for the model.
    tokio::time::timeout(
        Duration::from_secs(3),
        load_conversation(
            LoadConversationRequest {
                id: "ownership-b".into(),
            },
            app.state(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let same_session = SessionMode::Desktop {
        character_id: "test-character".into(),
        conversation_id: "ownership-a".into(),
    };
    let busy = submit_message(
        app,
        ChatId(42),
        100,
        "same session from Telegram",
        vec![],
        &same_session,
        "test-character",
    )
    .await
    .unwrap_err();
    assert!(busy.to_string().contains("chat_turn_busy"));
    // The production activation backend exercises prepare/apply/commit, with
    // isolated files. No synthetic assignment stands in for activation.
    let backend = crate::commands::characters::OrchestratorActivationBackend {
        orchestrator: &state,
        app_data: directory.join("activation"),
    };
    let coordinator = app.state::<ActivationCoordinator>();
    let token = coordinator
        .prepare(
            &state.db,
            "other-character",
            &Default::default(),
            &[],
            &backend,
        )
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        coordinator.commit(&state.db, token, &backend),
    )
    .await
    .unwrap()
    .unwrap();
    sqlx::query("UPDATE characters SET persona='CHANGED OWNED PERSONA',updated_at=updated_at+1 WHERE id='test-character'").execute(&state.db).await.unwrap();
    let token = coordinator
        .prepare(
            &state.db,
            "test-character",
            &Default::default(),
            &[],
            &backend,
        )
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        coordinator.commit(&state.db, token, &backend),
    )
    .await
    .unwrap()
    .unwrap();
    let replacement = Arc::new(ControlledProvider::default());
    app.state::<crate::llm::service::LlmService>()
        .replace_test_provider(replacement.clone())
        .await;
    provider.finish_pending.notify_one();
    let result = first.await.unwrap().unwrap();
    assert_eq!(result.conversation_id, "ownership-a");
    let saved_owner: String =
        sqlx::query_scalar("SELECT conversation_id FROM conversation_messages WHERE id=?")
            .bind(result.assistant_message_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(saved_owner, "ownership-a");
    let snapshot = runtime.turn_scope("ownership-first").unwrap();
    assert_eq!(snapshot.character_id.as_deref(), Some("test-character"));
    assert_eq!(snapshot.conversation_id.as_deref(), Some("ownership-a"));
    let first_prompt = provider
        .prompts
        .lock()
        .unwrap()
        .iter()
        .find(|prompt| {
            prompt
                .to_string()
                .contains("[ownership-tools] original owner")
        })
        .unwrap()
        .to_string();
    assert!(first_prompt.contains("ORIGINAL OWNED PERSONA"));
    assert!(!first_prompt.contains("CHANGED OWNED PERSONA"));
    let continuation = provider
        .prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|prompt| {
            prompt
                .to_string()
                .contains("[ownership-tools] original owner")
        })
        .last()
        .unwrap()
        .to_string();
    assert!(continuation.contains("ORIGINAL OWNED PERSONA"));
    assert!(!continuation.contains("CHANGED OWNED PERSONA"));
    assert!(continuation.contains("owner recorded"));
    assert!(replacement.prompts.lock().unwrap().is_empty());
    let scopes = app.state::<Arc<super::channels_test::ToolScopes>>();
    let scope = scopes.0.lock().unwrap().last().unwrap().clone();
    assert_eq!(scope.character_id, "test-character");
    assert_eq!(scope.conversation_id.as_deref(), Some("ownership-a"));
    assert_eq!(scope.request_id.as_deref(), Some("ownership-first"));
    assert_eq!(scope.turn_id.as_deref(), snapshot.turn_id.as_deref());
    app.state::<crate::llm::service::LlmService>()
        .replace_test_provider(provider.clone())
        .await;
    for name in [
        "chat-turn-delta",
        "chat-turn-tool",
        "chat-turn-finish",
        "chat-typing",
    ] {
        let events = emitted.lock().unwrap();
        let payload = &events
            .iter()
            .find(|(kind, event)| *kind == name && event["client_request_id"] == "ownership-first")
            .unwrap_or_else(|| panic!("missing scoped {name}: {events:?}"))
            .1;
        assert_eq!(payload["character_id"], "test-character");
        assert_eq!(payload["conversation_id"], "ownership-a");
        assert_eq!(payload["turn_id"].as_str(), snapshot.turn_id.as_deref());
    }
    for listener in listeners {
        app.unlisten(listener);
    }
    let a = ConversationTarget {
        require_character: false,
        character_id: "test-character".into(),
        conversation_id: "ownership-a".into(),
        create: false,
    };
    let b = ConversationTarget {
        conversation_id: "ownership-b".into(),
        ..a.clone()
    };
    assert_eq!(state.memory_progress_for(&a).await.0, 1);
    assert_eq!(state.memory_progress_for(&b).await.0, 0);
    load_conversation(
        LoadConversationRequest {
            id: "ownership-a".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
    submit_chat(
        app.clone(),
        request("ownership-second", "ownership-a", "second owned turn"),
        ChatPolicy::Desktop,
    )
    .await
    .unwrap();
    assert_eq!(state.memory_progress_for(&a).await.0, 2);
    submit_message(
        app,
        ChatId(42),
        101,
        "shared after release",
        vec![],
        &same_session,
        "test-character",
    )
    .await
    .unwrap();
    assert_eq!(
        state.memory_progress_for(&a).await.0,
        2,
        "external cadence stays separate within the shared session"
    );
    for (recreate, request_id, input) in [
        (false, "ownership-late", "[wait] delete while generating"),
        (
            true,
            "ownership-late-recreated",
            "[wait] delete while generating",
        ),
        (
            true,
            "ownership-late-failure",
            "[wait] [fail-stream] late failure",
        ),
        (
            true,
            "ownership-late-tool",
            "[ownership-tools] late tool result",
        ),
    ] {
        sqlx::query("INSERT OR IGNORE INTO conversations(id,character_id,created_at,updated_at) VALUES ('ownership-deleted','test-character','now','now')").execute(&state.db).await.unwrap();
        load_conversation(
            LoadConversationRequest {
                id: "ownership-deleted".into(),
            },
            app.state(),
        )
        .await
        .unwrap();
        let late = tokio::spawn(submit_chat(
            app.clone(),
            request(request_id, "ownership-deleted", input),
            ChatPolicy::Desktop,
        ));
        until_request_started(provider).await;
        crate::commands::conversation::delete_conversation(
            DeleteConversationRequest {
                id: "ownership-deleted".into(),
            },
            app.state(),
        )
        .await
        .unwrap();
        if recreate {
            sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('ownership-deleted','test-character','new','new')").execute(&state.db).await.unwrap();
        }
        provider.finish_pending.notify_one();
        assert!(late.await.unwrap().is_err());
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id='ownership-deleted'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        let orphaned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id='ownership-deleted'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!((remaining, orphaned), (i64::from(recreate), 0));
    }
    verify_deleted_approval(app).await;
    state.set_memory_enabled(false).await;
    // Two platform chats interleave at the shared admission gate. A cancellation
    // addressed to B cannot terminate A, and B can retry after A completes.
    let app_a = app.clone();
    let bot_a = tokio::spawn(async move {
        submit_message(
            &app_a,
            ChatId(701),
            1,
            "[wait] bot A isolation",
            vec![],
            &SessionMode::Private,
            "test-character",
        )
        .await
    });
    until_request_started(provider).await;
    let bot_b = submit_message(
        app,
        ChatId(702),
        1,
        "bot B isolation",
        vec![],
        &SessionMode::Private,
        "test-character",
    )
    .await
    .unwrap_err();
    assert!(bot_b.to_string().contains("chat_turn_busy"));
    crate::commands::chat::cancel_chat_turn(
        "unrelated-request".into(),
        None,
        app.state(),
        app.state(),
        app.state(),
    )
    .await
    .unwrap();
    assert!(!bot_a.is_finished());
    provider.finish_pending.notify_one();
    let a_reply = bot_a.await.unwrap().unwrap();
    let b_reply = submit_message(
        app,
        ChatId(702),
        2,
        "bot B after A",
        vec![],
        &SessionMode::Private,
        "test-character",
    )
    .await
    .unwrap();
    assert_ne!(a_reply.conversation_id, b_reply.conversation_id);
    let b_history: Vec<String> =
        sqlx::query_scalar("SELECT content FROM conversation_messages WHERE conversation_id=?")
            .bind(&b_reply.conversation_id)
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert!(!b_history
        .iter()
        .any(|text| text.contains("bot A isolation")));
    // Deleting an executing character uses the same production deletion routine.
    sqlx::query("INSERT INTO characters(id,name) VALUES ('doomed-owner','Doomed')")
        .execute(&state.db)
        .await
        .unwrap();
    let doomed = crate::chat::application::ChatRequest {
        message: "[wait] deleted role".into(),
        character_id: Some("doomed-owner".into()),
        conversation_id: Some("doomed-session".into()),
        client_request_id: Some("doomed-request".into()),
        ..Default::default()
    };
    let late_role = tokio::spawn(submit_chat(
        app.clone(),
        doomed,
        ChatPolicy::External {
            create_conversation: true,
            image_output: false,
            source: "telegram",
        },
    ));
    until_request_started(provider).await;
    {
        let _guard = state.owners.memory_commit.write().await;
        crate::commands::characters::delete_character_with_resources_in_pool(
            &state.db,
            directory,
            "doomed-owner",
        )
        .await
        .unwrap();
        state.forget_character("doomed-owner").await;
    }
    provider.finish_pending.notify_one();
    assert!(late_role.await.unwrap().is_err());
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id='doomed-session'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(rows, 0);
    crate::telegram::bot::verify_bound_session_submission(app).await;
    scopes.0.lock().unwrap().clear();
    verify_pending_reply_snapshots(app).await;
    verify_cue_snapshot(app, directory).await;
}

#[derive(Default)]
struct SnapshotLlm {
    streams: std::sync::atomic::AtomicUsize,
    memory: std::sync::Mutex<Vec<serde_json::Value>>,
    cues: std::sync::Mutex<Vec<serde_json::Value>>,
    memory_entered: tokio::sync::Notify,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    pause: bool,
}
#[async_trait::async_trait]
impl crate::llm::provider::LlmProvider for SnapshotLlm {
    fn id(&self) -> &str {
        "snapshot-llm"
    }
    async fn chat(
        &self,
        messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
        _: Option<crate::llm::provider::LlmParams>,
    ) -> Result<String, String> {
        let value = serde_json::to_value(messages).unwrap();
        if value.to_string().contains("memory extraction assistant") {
            self.memory.lock().unwrap().push(value);
            self.memory_entered.notify_one();
            Ok("[]".into())
        } else {
            self.cues.lock().unwrap().push(value);
            Ok(r#"{"cue":"old-cue"}"#.into())
        }
    }
    async fn chat_stream(
        &self,
        _: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
        _: Option<crate::llm::provider::LlmParams>,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
    {
        self.streams
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.pause {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(Box::pin(futures::stream::iter([Ok(
            "saved snapshot reply".into(),
        )])))
    }
}
struct SnapshotVoice {
    value: u8,
    calls: Arc<std::sync::Mutex<Vec<crate::tts::interface::TtsParams>>>,
}
#[async_trait::async_trait]
impl crate::tts::interface::TtsProvider for SnapshotVoice {
    fn id(&self) -> String {
        "browser".into()
    }
    fn capabilities(&self) -> crate::tts::interface::ProviderCapabilities {
        Default::default()
    }
    fn voices(&self) -> Vec<crate::tts::interface::VoiceProfile> {
        vec![]
    }
    async fn is_available(&self) -> bool {
        true
    }
    async fn synthesize(
        &self,
        _: &str,
        params: crate::tts::interface::TtsParams,
    ) -> Result<Vec<u8>, crate::tts::interface::TtsError> {
        self.calls.lock().unwrap().push(params);
        Ok(vec![self.value])
    }
}
async fn verify_pending_reply_snapshots(app: &tauri::AppHandle) {
    use crate::chat::application::{submit_chat, ChatPolicy};
    let state = app.state::<AIOrchestrator>();
    state.set_memory_enabled(true).await;
    let profile = serde_json::json!({"response_language":"Japanese","tts":{"provider_id":"browser","voice":"old-voice","speed":1.25}});
    sqlx::query("INSERT INTO characters(id,name,runtime_profile_json) VALUES ('snapshot-retry','Snapshot',?)").bind(profile.to_string()).execute(&state.db).await.unwrap();
    let old = Arc::new(SnapshotLlm::default());
    app.state::<crate::llm::service::LlmService>()
        .replace_test_provider(old.clone())
        .await;
    let old_voice = Arc::new(std::sync::Mutex::new(Vec::new()));
    app.state::<crate::tts::TtsService>()
        .register_provider(Arc::new(SnapshotVoice {
            value: 1,
            calls: old_voice.clone(),
        }))
        .await;
    let request = |number: u8| ChatRequest {
        message: format!("neutral snapshot step {number}"),
        character_id: Some("snapshot-retry".into()),
        conversation_id: Some("snapshot-retry-session".into()),
        client_request_id: Some(format!("snapshot-retry-{number}")),
        ..Default::default()
    };
    let policy = ChatPolicy::External {
        create_conversation: true,
        image_output: false,
        source: "telegram",
    };
    for number in 1..5 {
        submit_chat(app.clone(), request(number), policy)
            .await
            .unwrap();
    }
    sqlx::query("CREATE TRIGGER reject_snapshot_reply BEFORE INSERT ON conversation_messages WHEN NEW.conversation_id='snapshot-retry-session' AND NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'snapshot final save fault'); END").execute(&state.db).await.unwrap();
    assert!(matches!(
        submit_chat(app.clone(), request(5), policy).await,
        Err(crate::error::KokoroError::ChatPersistence(_))
    ));
    sqlx::query("DROP TRIGGER reject_snapshot_reply")
        .execute(&state.db)
        .await
        .unwrap();
    let new = Arc::new(SnapshotLlm::default());
    app.state::<crate::llm::service::LlmService>()
        .replace_test_provider(new.clone())
        .await;
    let new_voice = Arc::new(std::sync::Mutex::new(Vec::new()));
    app.state::<crate::tts::TtsService>()
        .register_provider(Arc::new(SnapshotVoice {
            value: 2,
            calls: new_voice.clone(),
        }))
        .await;
    let changed = serde_json::json!({"response_language":"German","tts":{"provider_id":"browser","voice":"new-voice","speed":0.75}});
    sqlx::query("UPDATE characters SET runtime_profile_json=? WHERE id='snapshot-retry'")
        .bind(changed.to_string())
        .execute(&state.db)
        .await
        .unwrap();
    let recovered = submit_chat(app.clone(), request(5), policy).await.unwrap();
    assert_eq!(
        recovered
            .speech_snapshot
            .as_ref()
            .unwrap()
            .synthesize("delivered recovered reply")
            .await
            .unwrap(),
        vec![1]
    );
    tokio::time::timeout(Duration::from_secs(3), old.memory_entered.notified())
        .await
        .unwrap();
    let extraction = old.memory.lock().unwrap()[0].to_string();
    assert!(extraction.contains("Japanese"), "{extraction}");
    assert!(extraction.contains("neutral snapshot step 5"));
    assert!(new.memory.lock().unwrap().is_empty());
    assert_eq!(new.streams.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(old.streams.load(std::sync::atomic::Ordering::SeqCst), 5);
    assert_eq!(
        old_voice.lock().unwrap()[0].voice.as_deref(),
        Some("old-voice")
    );
    assert_eq!(old_voice.lock().unwrap()[0].speed, Some(1.25));
    let duplicate = submit_chat(app.clone(), request(5), policy).await.unwrap();
    assert_eq!(
        duplicate.assistant_message_id,
        recovered.assistant_message_id
    );
    let next = submit_chat(app.clone(), request(6), policy).await.unwrap();
    assert_eq!(
        next.speech_snapshot
            .as_ref()
            .unwrap()
            .synthesize("next reply")
            .await
            .unwrap(),
        vec![2]
    );
    assert_eq!(
        new_voice.lock().unwrap()[0].voice.as_deref(),
        Some("new-voice")
    );
    let owner:(String,String)=sqlx::query_as("SELECT c.character_id,m.conversation_id FROM conversation_messages m JOIN conversations c ON c.id=m.conversation_id WHERE m.id=?").bind(recovered.assistant_message_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(
        owner,
        ("snapshot-retry".into(), "snapshot-retry-session".into())
    );
}

async fn verify_cue_snapshot(app: &tauri::AppHandle, directory: &std::path::Path) {
    use crate::actions::registry::ActionHandler;
    use crate::chat::application::{submit_chat, ChatPolicy};
    let state = app.state::<AIOrchestrator>();
    state.set_memory_enabled(false).await;
    let package_root = directory.join("cue-packages");
    let package = package_root.join("cue-template").join("1.0.0");
    std::fs::create_dir_all(&package).unwrap();
    let manifest = serde_json::json!({"schema_version":1,"engine_version":">=0.3.0","id":"cue-template","version":"1.0.0","name":"Cue","description":"fixture","author":"Test","license":"CC0-1.0","persona":"Cue persona","greeting":"Hello","assets":{"cue_profile":"cues.json"}});
    std::fs::write(package.join("character.json"), manifest.to_string()).unwrap();
    std::fs::write(
        package.join("cues.json"),
        r#"{"schema_version":1,"cues":{"old-cue":{}}}"#,
    )
    .unwrap();
    sqlx::query("INSERT INTO characters(id,name,source_format,template_id,template_version,template_snapshot_json) VALUES ('cue-owner','Cue','template','cue-template','1.0.0',?)").bind(manifest.to_string()).execute(&state.db).await.unwrap();
    let backend = crate::commands::characters::OrchestratorActivationBackend {
        orchestrator: &state,
        app_data: directory.join("cue-activation"),
    };
    let coordinator = app.state::<ActivationCoordinator>();
    let token = coordinator
        .prepare_with_package_root(
            &state.db,
            &package_root,
            "cue-owner",
            &Default::default(),
            &[],
            &backend,
        )
        .await
        .unwrap();
    let committed = coordinator
        .commit(&state.db, token, &backend)
        .await
        .unwrap();
    let conversation = committed.target_conversation_id;
    let provider = Arc::new(SnapshotLlm {
        pause: true,
        ..Default::default()
    });
    app.state::<crate::llm::service::LlmService>()
        .replace_test_provider(provider.clone())
        .await;
    let events = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let captured = events.clone();
    let listener = app.listen("chat-cue", move |event| {
        captured
            .lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap())
    });
    let task = tokio::spawn(submit_chat(
        app.clone(),
        ChatRequest {
            message: "bound cue reply".into(),
            character_id: Some("cue-owner".into()),
            conversation_id: Some(conversation.clone()),
            client_request_id: Some("cue-snapshot-turn".into()),
            ..Default::default()
        },
        ChatPolicy::Desktop,
    ));
    tokio::time::timeout(Duration::from_secs(3), provider.entered.notified())
        .await
        .unwrap();
    std::fs::write(
        package.join("cues.json"),
        r#"{"schema_version":1,"cues":{"new-cue":{}}}"#,
    )
    .unwrap();
    let token = coordinator
        .prepare(
            &state.db,
            "other-character",
            &Default::default(),
            &[],
            &backend,
        )
        .await
        .unwrap();
    coordinator
        .commit(&state.db, token, &backend)
        .await
        .unwrap();
    provider.release.notify_one();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.conversation_id, conversation);
    let analysis = provider.cues.lock().unwrap()[0].to_string();
    assert!(analysis.contains("old-cue"));
    assert!(!analysis.contains("new-cue"));
    let fallback = events.lock().unwrap()[0].clone();
    assert_eq!(fallback["cue"], "old-cue");
    assert_eq!(fallback["character_id"], "cue-owner");
    assert_eq!(fallback["conversation_id"], conversation);
    assert_eq!(fallback["client_request_id"], "cue-snapshot-turn");
    let context = crate::actions::registry::ActionContext {
        memory_scope: None,
        cue_names: Some(Arc::new(std::collections::HashSet::from(
            ["old-cue".into()],
        ))),
        request_id: Some("cue-snapshot-turn".into()),
        turn_id: fallback["turn_id"].as_str().map(str::to_owned),
        app: app.clone(),
        character_id: "cue-owner".into(),
        conversation_id: Some(conversation),
        source: Some("chat".into()),
    };
    crate::actions::builtin::PlayCueAction
        .execute(
            std::collections::HashMap::from([("cue".into(), "old-cue".into())]),
            context,
        )
        .await
        .unwrap();
    assert_eq!(events.lock().unwrap()[1]["character_id"], "cue-owner");
    assert_eq!(events.lock().unwrap()[1]["source"], "builtin-play-cue");
    app.unlisten(listener);
}

struct OwnerWrite(Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl crate::actions::registry::ActionHandler for OwnerWrite {
    fn name(&self) -> &str {
        "owner_write"
    }
    fn description(&self) -> &str {
        "Count a non-memory write tool"
    }
    fn parameters(&self) -> Vec<crate::actions::registry::ActionParam> {
        vec![]
    }
    fn permission_level(&self) -> crate::actions::registry::ActionPermissionLevel {
        crate::actions::registry::ActionPermissionLevel::Elevated
    }
    async fn execute(
        &self,
        _: std::collections::HashMap<String, String>,
        _: crate::actions::registry::ActionContext,
    ) -> Result<crate::actions::registry::ActionResult, crate::actions::registry::ActionError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(crate::actions::registry::ActionResult::ok(
            "owner write completed",
        ))
    }
}
async fn verify_deleted_approval(app: &tauri::AppHandle) {
    use crate::actions::{
        registry::{ActionPermissionLevel, ActionRegistry},
        tool_settings::ToolSettings,
    };
    use crate::chat::application::{submit_chat, ChatPolicy};
    use crate::commands::conversation::{
        load_conversation, DeleteConversationRequest, LoadConversationRequest,
    };
    let state = app.state::<AIOrchestrator>();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    app.state::<Arc<tokio::sync::RwLock<ActionRegistry>>>()
        .write()
        .await
        .register(OwnerWrite(calls.clone()));
    let settings = app.state::<Arc<tokio::sync::RwLock<ToolSettings>>>();
    settings.write().await.max_permission_level = ActionPermissionLevel::Safe;
    sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('approval-owner','test-character','now','now')").execute(&state.db).await.unwrap();
    load_conversation(
        LoadConversationRequest {
            id: "approval-owner".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
    let approval = Arc::new(std::sync::Mutex::new(None));
    let ready = Arc::new(tokio::sync::Notify::new());
    let event_approval = approval.clone();
    let event_ready = ready.clone();
    let listener = app.listen("chat-turn-tool", move |event| {
        let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        if payload["client_request_id"] == "deleted-approval"
            && payload["approval_status"] == "requested"
        {
            *event_approval.lock().unwrap() =
                payload["approval_request_id"].as_str().map(str::to_owned);
            event_ready.notify_one();
        }
    });
    let request = |id: &str| ChatRequest {
        message: "[owner-approval]".into(),
        character_id: Some("test-character".into()),
        conversation_id: Some("approval-owner".into()),
        client_request_id: Some(id.into()),
        ..Default::default()
    };
    let worker = tokio::spawn(submit_chat(
        app.clone(),
        request("deleted-approval"),
        ChatPolicy::Desktop,
    ));
    tokio::time::timeout(Duration::from_secs(5), ready.notified())
        .await
        .unwrap();
    app.unlisten(listener);
    crate::commands::conversation::delete_conversation(
        DeleteConversationRequest {
            id: "approval-owner".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES ('approval-owner','test-character','new','new')").execute(&state.db).await.unwrap();
    settings.write().await.max_permission_level = ActionPermissionLevel::Elevated;
    let approval_id = approval.lock().unwrap().clone().unwrap();
    crate::commands::chat::approve_tool_approval(approval_id, app.state())
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id='approval-owner'"
        )
        .fetch_one(&state.db)
        .await
        .unwrap(),
        0
    );
    // Revocation must not accidentally turn the memory toggle into tool permission.
    state.set_memory_enabled(false).await;
    load_conversation(
        LoadConversationRequest {
            id: "approval-owner".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
    submit_chat(
        app.clone(),
        request("fresh-owner-tool"),
        ChatPolicy::Desktop,
    )
    .await
    .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// The prompt mutex pauses a real admitted worker before its execution snapshot;
/// the existing provider Notify pauses it again after bound/start publication.
pub(super) async fn verify_admission_selection_generation(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
) {
    use crate::chat::application::{retry_chat_persistence, submit_chat, ChatPolicy};
    use crate::commands::conversation::{
        create_conversation, load_conversation, LoadConversationRequest,
    };
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    let events = Arc::new(std::sync::Mutex::new(
        Vec::<(String, serde_json::Value)>::new(),
    ));
    let listeners: Vec<_> = ["chat-turn-bound", "chat-turn-start", "chat-turn-finish"]
        .into_iter()
        .map(|name| {
            let events = events.clone();
            app.listen(name, move |event| {
                let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
                events.lock().unwrap().push((name.into(), payload));
            })
        })
        .collect();
    for (index, hidden, switch_at, save_retry) in [
        (0, true, "snapshot", false),
        (1, false, "snapshot", false),
        (2, true, "model", false),
        (3, true, "retry", true),
        (4, true, "never", false),
        (5, false, "never", false),
        (6, true, "never", true),
    ] {
        create_conversation(app.state()).await.unwrap();
        let admitted_generation = state.current_conversation_generation();
        let request_id = format!("selection-generation-{index}");
        let prompt_guard = if switch_at == "snapshot" {
            Some(state.system_prompt.lock().await)
        } else {
            None
        };
        let worker = tokio::spawn(submit_chat(
            app.clone(),
            ChatRequest {
                message: "[wait] preserve admission selection".into(),
                hidden,
                character_id: Some("test-character".into()),
                conversation_generation: Some(admitted_generation),
                client_request_id: Some(request_id.clone()),
                ..Default::default()
            },
            ChatPolicy::Desktop,
        ));
        if prompt_guard.is_some() {
            // Observation only: the held mutex is the actual pause, no timing sleeps.
            tokio::time::timeout(Duration::from_secs(5), async {
                while runtime.turn_scope(&request_id).is_none() || !state.is_chat_busy() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            create_conversation(app.state()).await.unwrap();
        }
        drop(prompt_guard);
        until_request_started(provider).await;
        let bound = runtime.turn_scope(&request_id).unwrap();
        let conversation = bound.conversation_id.clone().unwrap();
        let prepared_generation = if !hidden && switch_at != "snapshot" {
            admitted_generation + 1
        } else {
            admitted_generation
        };
        let observed_before = bound.conversation_generation;
        let bound_events: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, p)| {
                p["client_request_id"] == request_id
                    && (name == "chat-turn-bound" || name == "chat-turn-start")
            })
            .map(|(_, p)| p["conversation_generation"].as_u64())
            .collect();
        assert_eq!(bound_events.len(), 2);
        if switch_at == "model" {
            create_conversation(app.state()).await.unwrap();
        }
        if save_retry {
            sqlx::query("CREATE TRIGGER reject_generation_reply BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'generation retry'); END").execute(&state.db).await.unwrap();
        }
        provider.finish_pending.notify_one();
        let result = worker.await.unwrap();
        let response = if save_retry {
            assert!(matches!(
                result,
                Err(crate::error::KokoroError::ChatPersistence(_))
            ));
            sqlx::query("DROP TRIGGER reject_generation_reply")
                .execute(&state.db)
                .await
                .unwrap();
            if switch_at == "retry" {
                create_conversation(app.state()).await.unwrap();
            }
            let before = provider.prompts.lock().unwrap().len();
            let response = retry_chat_persistence(app.clone(), request_id.clone())
                .await
                .unwrap();
            assert_eq!(provider.prompts.lock().unwrap().len(), before);
            response
        } else {
            result.unwrap()
        };
        let final_turn = runtime.turn_scope(&request_id).unwrap();
        let final_generation = if switch_at == "never" {
            admitted_generation + 1
        } else {
            admitted_generation
        };
        let selected = state.selection.current_conversation_id.lock().await.clone();
        assert_eq!(response.conversation_id, conversation);
        assert_eq!(
            selected,
            if switch_at == "never" {
                Some(conversation.clone())
            } else {
                None
            }
        );
        let owner: String = sqlx::query_scalar("SELECT character_id FROM conversations WHERE id=?")
            .bind(&conversation)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(owner, "test-character");
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT role FROM conversation_messages WHERE conversation_id=? ORDER BY id",
        )
        .bind(&conversation)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(
            roles,
            if hidden {
                vec!["assistant"]
            } else {
                vec!["user", "assistant"]
            }
        );
        assert_eq!(
            observed_before,
            Some(prepared_generation),
            "bound journal adopted a later empty selection: {index}"
        );
        assert!(
            bound_events
                .iter()
                .all(|generation| *generation == Some(prepared_generation)),
            "bound/start event changed selection owner: {index}: {bound_events:?}"
        );
        assert_eq!(
            final_turn.conversation_generation,
            Some(final_generation),
            "completion/retry adopted a later empty selection: {index}"
        );
        let finish = events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(name, p)| name == "chat-turn-finish" && p["client_request_id"] == request_id)
            .unwrap()
            .1
            .clone();
        assert_eq!(
            finish["conversation_generation"].as_u64(),
            Some(final_generation)
        );
        if switch_at != "never" {
            let view = crate::chat::runtime::get_chat_view(&state, &runtime, None)
                .await
                .unwrap()
                .unwrap();
            assert!(view.conversation_id.is_none() && view.messages.is_empty());
            assert_ne!(
                final_turn.conversation_generation,
                Some(view.version.generation)
            );
        }
    }
    for listener in listeners {
        app.unlisten(listener);
    }
    load_conversation(
        LoadConversationRequest {
            id: "desktop".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
}

pub(super) async fn verify_reserved_memory_tools(
    app: &tauri::AppHandle,
    provider: &ControlledProvider,
) {
    use crate::chat::application::{submit_chat, ChatPolicy};
    use crate::commands::conversation::{
        create_conversation, delete_conversation, load_conversation, DeleteConversationRequest,
        LoadConversationRequest,
    };
    let state = app.state::<AIOrchestrator>();
    let runtime = app.state::<Arc<ChatRuntimeState>>();
    for mutation in [
        "none",
        "selection",
        "memory-toggle",
        "conversation-deleted",
        "conversation-recreated",
        "character-recreated",
        "foreign-conversation",
    ] {
        let owner = format!("reserved-memory-{mutation}");
        sqlx::query("INSERT INTO characters(id,name) VALUES (?,?)")
            .bind(&owner)
            .bind(&owner)
            .execute(&state.db)
            .await
            .unwrap();
        state.set_character_id(owner.clone()).await;
        state.set_memory_enabled(true).await;
        create_conversation(app.state()).await.unwrap();
        let (embedding_entered, embedding_release) =
            state.memory_manager.pause_embedding_for("model trains");
        let request_id = format!("reserved-memory-request-{mutation}");
        let worker = tokio::spawn(submit_chat(
            app.clone(),
            ChatRequest {
                message: "[reserved-memory] remember a fact".into(),
                hidden: true,
                character_id: Some(owner.clone()),
                client_request_id: Some(request_id.clone()),
                ..Default::default()
            },
            ChatPolicy::Desktop,
        ));
        until_request_started(provider).await;
        let scope = runtime.turn_scope(&request_id).unwrap();
        let conversation = scope.conversation_id.clone().unwrap();
        let background_events = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        if mutation == "none" {
            use crate::actions::registry::ActionHandler;
            let captured = background_events.clone();
            let listener = app.listen("chat-imagegen", move |event| {
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(event.payload()).unwrap());
            });
            let context = crate::actions::ActionContext {
                memory_scope: None,
                cue_names: None,
                request_id: Some(request_id.clone()),
                turn_id: scope.turn_id.clone(),
                app: app.clone(),
                character_id: owner.clone(),
                conversation_id: Some(conversation.clone()),
                source: Some("chat".into()),
            };
            crate::actions::builtin::SetBackgroundAction
                .execute(
                    std::collections::HashMap::from([("prompt".into(), "garden".into())]),
                    context.clone(),
                )
                .await
                .unwrap();
            crate::actions::builtin::SetBackgroundAction
                .execute(
                    std::collections::HashMap::from([("prompt".into(), "manual garden".into())]),
                    crate::actions::ActionContext {
                        request_id: None,
                        turn_id: None,
                        conversation_id: None,
                        ..context
                    },
                )
                .await
                .unwrap();
            app.unlisten(listener);
        }
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id=?")
            .bind(&conversation)
            .fetch_one(&state.db)
            .await
            .unwrap();
        provider.finish_pending.notify_one();
        tokio::time::timeout(Duration::from_secs(5), embedding_entered.notified())
            .await
            .unwrap();
        match mutation {
            "selection" => {
                state.set_character_id("other-character".into()).await;
                create_conversation(app.state()).await.unwrap();
            }
            "memory-toggle" => {
                state.set_memory_enabled(false).await;
                state.set_memory_enabled(true).await;
            }
            "conversation-deleted" | "conversation-recreated" => {
                delete_conversation(
                    DeleteConversationRequest {
                        id: conversation.clone(),
                    },
                    app.state(),
                )
                .await
                .unwrap();
                if mutation == "conversation-recreated" {
                    sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES (?,?,'now','now')").bind(&conversation).bind(&owner).execute(&state.db).await.unwrap();
                }
            }
            "character-recreated" => {
                crate::commands::characters::delete_character(
                    owner.clone(),
                    app.state(),
                    app.clone(),
                )
                .await
                .unwrap();
                sqlx::query("INSERT INTO characters(id,name) VALUES (?,?)")
                    .bind(&owner)
                    .bind(&owner)
                    .execute(&state.db)
                    .await
                    .unwrap();
            }
            "foreign-conversation" => {
                sqlx::query("INSERT INTO conversations(id,character_id,created_at,updated_at) VALUES (?,'other-character','now','now')").bind(&conversation).execute(&state.db).await.unwrap();
            }
            _ => {}
        }
        embedding_release.notify_one();
        let result = worker.await.unwrap();
        if mutation == "none" {
            let events = background_events.lock().unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0]["conversation_id"], conversation);
            assert_eq!(events[0]["client_request_id"], request_id);
            assert_eq!(
                events[0]["conversation_generation"].as_u64(),
                scope.conversation_generation
            );
            assert!(events[1]["client_request_id"].is_null());
            assert!(events[1]["conversation_generation"].is_null());
        }
        assert_eq!(
            before, 0,
            "hidden admission must not materialize an empty conversation"
        );
        let memories: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE character_id=?")
                .bind(&owner)
                .fetch_one(&state.db)
                .await
                .unwrap();
        let candidates: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memory_candidates WHERE character_id=?")
                .bind(&owner)
                .fetch_one(&state.db)
                .await
                .unwrap();
        if matches!(mutation, "none" | "selection") {
            assert!(result.is_ok(), "{mutation}: {result:?}");
            assert_eq!(memories, 1, "reserved role memory failed: {mutation}");
            assert_eq!(candidates, 1);
            let (saved_owner, content): (String, String) =
                sqlx::query_as("SELECT character_id,content FROM memories WHERE character_id=?")
                    .bind(&owner)
                    .fetch_one(&state.db)
                    .await
                    .unwrap();
            assert_eq!(saved_owner, owner);
            assert_eq!(content, "User collects model trains.");
            let tool_result: String = sqlx::query_scalar(
                "SELECT content FROM conversation_messages WHERE conversation_id=? AND role='tool'",
            )
            .bind(&conversation)
            .fetch_one(&state.db)
            .await
            .unwrap();
            assert!(!tool_result.contains("revoked"), "{tool_result}");
        } else {
            assert_eq!(memories, 0, "revoked scope wrote memory: {mutation}");
            assert_eq!(candidates, 0, "revoked scope wrote candidate: {mutation}");
        }
    }
    state.set_memory_enabled(false).await;
    state.set_character_id("test-character".into()).await;
    load_conversation(
        LoadConversationRequest {
            id: "desktop".into(),
        },
        app.state(),
    )
    .await
    .unwrap();
}
