// pattern: Imperative Shell
use super::{ChatHookPayload, HookEvent, HookHandler, HookOutcome, HookPayload, HookRuntime};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

struct RecordingHandler {
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    outcome: Result<HookOutcome, &'static str>,
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

        match &self.outcome {
            Ok(outcome) => Ok(outcome.clone()),
            Err(error) => Err(format!("{} failed: {}", self.id, error)),
        }
    }
}

fn continue_handler(
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
) -> RecordingHandler {
    RecordingHandler {
        id,
        events,
        calls,
        outcome: Ok(HookOutcome::Continue),
    }
}

fn deny_handler(
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    reason: &'static str,
) -> RecordingHandler {
    RecordingHandler {
        id,
        events,
        calls,
        outcome: Ok(HookOutcome::Deny {
            reason: reason.to_string(),
        }),
    }
}

fn error_handler(
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    error: &'static str,
) -> RecordingHandler {
    RecordingHandler {
        id,
        events,
        calls,
        outcome: Err(error),
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
    runtime.register(Arc::new(continue_handler(
        "first",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
    )));

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

    runtime.register(Arc::new(continue_handler(
        "first",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
    )));
    runtime.register(Arc::new(continue_handler(
        "second",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
    )));

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

    runtime.register(Arc::new(error_handler(
        "failing",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
        "boom",
    )));
    runtime.register(Arc::new(continue_handler(
        "next",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
    )));

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
async fn emit_best_effort_ignores_deny_and_continues() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(deny_handler(
        "deny",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
        "blocked",
    )));
    runtime.register(Arc::new(continue_handler(
        "next",
        &[HookEvent::BeforeUserMessage],
        calls.clone(),
    )));

    let outcome = runtime
        .emit_best_effort(&HookEvent::BeforeUserMessage, &sample_payload())
        .await;

    assert_eq!(outcome, HookOutcome::Continue);
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["deny:BeforeUserMessage", "next:BeforeUserMessage"]
    );
}

#[tokio::test]
async fn emit_skips_handlers_without_matching_event() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(continue_handler(
        "other",
        &[HookEvent::AfterLlmResponse],
        calls.clone(),
    )));

    runtime
        .emit_best_effort(&HookEvent::BeforeUserMessage, &sample_payload())
        .await;

    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn action_gate_returns_deny_and_stops_later_handlers() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(deny_handler(
        "deny",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        "blocked",
    )));
    runtime.register(Arc::new(continue_handler(
        "later",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
    )));

    let outcome = runtime
        .emit_action_gate(&HookEvent::BeforeActionInvoke, &sample_payload())
        .await;

    assert_eq!(
        outcome,
        HookOutcome::Deny {
            reason: "blocked".to_string(),
        }
    );
    assert_eq!(calls.lock().unwrap().as_slice(), ["deny:BeforeActionInvoke"]);
}

#[tokio::test]
async fn action_gate_continues_after_handler_error() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(error_handler(
        "error",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        "boom",
    )));
    runtime.register(Arc::new(continue_handler(
        "next",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
    )));

    let outcome = runtime
        .emit_action_gate(&HookEvent::BeforeActionInvoke, &sample_payload())
        .await;

    assert_eq!(outcome, HookOutcome::Continue);
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["error:BeforeActionInvoke", "next:BeforeActionInvoke"]
    );
}
