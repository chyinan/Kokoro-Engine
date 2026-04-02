use super::{ChatHookPayload, HookEvent, HookHandler, HookOutcome, HookPayload, HookRuntime};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

struct RecordingHandler {
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    fail: bool,
}

#[async_trait]
impl HookHandler for RecordingHandler {
    fn id(&self) -> &str {
        self.id
    }

    fn events(&self) -> &'static [HookEvent] {
        self.events
    }

    async fn handle(
        &self,
        event: &HookEvent,
        _payload: &HookPayload,
    ) -> Result<HookOutcome, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{:?}", self.id, event));

        if self.fail {
            Err(format!("{} failed", self.id))
        } else {
            Ok(HookOutcome::Continue)
        }
    }
}

fn sample_payload() -> HookPayload {
    HookPayload::Chat(ChatHookPayload {
        conversation_id: Some("conv-1".to_string()),
        character_id: "default".to_string(),
        turn_id: Some("turn-1".to_string()),
        message: Some("hello".to_string()),
        response: None,
        tool_round: None,
        hidden: false,
    })
}

#[tokio::test]
async fn emit_calls_registered_handler_once() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    runtime.register(Arc::new(RecordingHandler {
        id: "first",
        events: &[HookEvent::BeforeUserMessage],
        calls: calls.clone(),
        fail: false,
    }));

    let outcome = runtime
        .emit(&HookEvent::BeforeUserMessage, &sample_payload())
        .await
        .unwrap();

    assert_eq!(outcome, HookOutcome::Continue);
    assert_eq!(calls.lock().unwrap().as_slice(), ["first:BeforeUserMessage"]);
}

#[tokio::test]
async fn emit_preserves_registration_order() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(RecordingHandler {
        id: "first",
        events: &[HookEvent::BeforeUserMessage],
        calls: calls.clone(),
        fail: false,
    }));
    runtime.register(Arc::new(RecordingHandler {
        id: "second",
        events: &[HookEvent::BeforeUserMessage],
        calls: calls.clone(),
        fail: false,
    }));

    runtime
        .emit(&HookEvent::BeforeUserMessage, &sample_payload())
        .await
        .unwrap();

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["first:BeforeUserMessage", "second:BeforeUserMessage"]
    );
}

#[tokio::test]
async fn emit_best_effort_continues_after_handler_error() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(RecordingHandler {
        id: "failing",
        events: &[HookEvent::BeforeUserMessage],
        calls: calls.clone(),
        fail: true,
    }));
    runtime.register(Arc::new(RecordingHandler {
        id: "next",
        events: &[HookEvent::BeforeUserMessage],
        calls: calls.clone(),
        fail: false,
    }));

    let outcome = runtime
        .emit_best_effort(&HookEvent::BeforeUserMessage, &sample_payload())
        .await;

    assert_eq!(outcome, HookOutcome::Continue);
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["failing:BeforeUserMessage", "next:BeforeUserMessage"]
    );
}

#[tokio::test]
async fn emit_skips_handlers_without_matching_event() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(RecordingHandler {
        id: "other",
        events: &[HookEvent::AfterLlmResponse],
        calls: calls.clone(),
        fail: false,
    }));

    runtime
        .emit_best_effort(&HookEvent::BeforeUserMessage, &sample_payload())
        .await;

    assert!(calls.lock().unwrap().is_empty());
}
