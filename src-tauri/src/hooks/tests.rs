// pattern: Imperative Shell
use super::{
    BeforeActionArgsPayload, BeforeLlmRequestMessage, BeforeLlmRequestPayload, ChatHookPayload,
    HookEvent, HookHandler, HookOutcome, HookPayload, HookRuntime,
};
use crate::hooks::types::HookModifyPolicy;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct RecordingHandler {
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    outcome: Result<HookOutcome, &'static str>,
    before_llm_request_modifier: Option<BeforeLlmRequestModifier>,
    before_action_args_modifier: Option<BeforeActionArgsModifier>,
}

type BeforeLlmRequestModifier =
    Arc<dyn Fn(&mut BeforeLlmRequestPayload) -> Result<(), &'static str> + Send + Sync>;

type BeforeActionArgsModifier =
    Arc<dyn Fn(&mut BeforeActionArgsPayload) -> Result<(), &'static str> + Send + Sync>;

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

    async fn modify_before_llm_request(
        &self,
        payload: &mut BeforeLlmRequestPayload,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:BeforeLlmRequestModify", self.id));

        match self.before_llm_request_modifier.as_ref() {
            Some(modifier) => {
                modifier(payload).map_err(|error| format!("{} failed: {}", self.id, error))
            }
            None => Ok(()),
        }
    }

    async fn modify_before_action_args(
        &self,
        payload: &mut BeforeActionArgsPayload,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:BeforeActionArgsModify", self.id));

        match self.before_action_args_modifier.as_ref() {
            Some(modifier) => {
                modifier(payload).map_err(|error| format!("{} failed: {}", self.id, error))
            }
            None => Ok(()),
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
        before_llm_request_modifier: None,
        before_action_args_modifier: None,
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
        before_llm_request_modifier: None,
        before_action_args_modifier: None,
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
        before_llm_request_modifier: None,
        before_action_args_modifier: None,
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

fn sample_before_llm_request_payload() -> BeforeLlmRequestPayload {
    BeforeLlmRequestPayload {
        conversation_id: Some("conv-1".to_string()),
        character_id: "default".to_string(),
        turn_id: Some("turn-1".to_string()),
        hidden: false,
        request_message: "hello".to_string(),
        messages: vec![
            BeforeLlmRequestMessage {
                role: "system".to_string(),
                content: "system prompt".to_string(),
            },
            BeforeLlmRequestMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            },
        ],
    }
}

fn sample_before_action_args_payload() -> BeforeActionArgsPayload {
    BeforeActionArgsPayload {
        conversation_id: Some("conv-1".to_string()),
        character_id: "default".to_string(),
        tool_call_id: Some("tool-call-1".to_string()),
        action_id: "builtin__search_memory".to_string(),
        action_name: "search_memory".to_string(),
        args: HashMap::from([("query".to_string(), "kokoro".to_string())]),
        source: Some("chat".to_string()),
    }
}

fn append_modifier(suffix: &'static str) -> BeforeLlmRequestModifier {
    Arc::new(move |payload| {
        payload.request_message.push_str(suffix);
        payload.messages.push(BeforeLlmRequestMessage {
            role: "assistant".to_string(),
            content: suffix.trim().to_string(),
        });
        Ok(())
    })
}

fn error_modifier(error: &'static str) -> BeforeLlmRequestModifier {
    Arc::new(move |_payload| Err(error))
}

fn modify_handler(
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    modifier: BeforeLlmRequestModifier,
) -> RecordingHandler {
    RecordingHandler {
        id,
        events,
        calls,
        outcome: Ok(HookOutcome::Continue),
        before_llm_request_modifier: Some(modifier),
        before_action_args_modifier: None,
    }
}

fn append_action_arg_modifier(key: &'static str, value: &'static str) -> BeforeActionArgsModifier {
    Arc::new(move |payload| {
        payload.args.insert(key.to_string(), value.to_string());
        Ok(())
    })
}

fn action_arg_error_modifier(error: &'static str) -> BeforeActionArgsModifier {
    Arc::new(move |_payload| Err(error))
}

fn action_args_modify_handler(
    id: &'static str,
    events: &'static [HookEvent],
    calls: Arc<Mutex<Vec<String>>>,
    modifier: BeforeActionArgsModifier,
) -> RecordingHandler {
    RecordingHandler {
        id,
        events,
        calls,
        outcome: Ok(HookOutcome::Continue),
        before_llm_request_modifier: None,
        before_action_args_modifier: Some(modifier),
    }
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
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["first:BeforeUserMessage"]
    );
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
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["deny:BeforeActionInvoke"]
    );
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

#[tokio::test]
async fn before_llm_request_modify_preserves_request_message_and_messages() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_llm_request_payload();

    runtime.register(Arc::new(modify_handler(
        "first",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_modifier(" +first"),
    )));
    runtime.register(Arc::new(modify_handler(
        "second",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_modifier(" +second"),
    )));

    runtime
        .emit_before_llm_request_modify(&mut payload, HookModifyPolicy::Permissive)
        .await
        .unwrap();

    assert_eq!(payload.request_message, "hello +first +second");
    assert_eq!(payload.messages.len(), 4);
    assert_eq!(payload.messages[0].role, "system");
    assert_eq!(payload.messages[1].content, "hello");
    assert_eq!(payload.messages[2].content, "+first");
    assert_eq!(payload.messages[3].content, "+second");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "first:BeforeLlmRequestModify",
            "second:BeforeLlmRequestModify"
        ]
    );
}

#[tokio::test]
async fn before_llm_modify_strict_mode_returns_err_when_handler_fails() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_llm_request_payload();

    runtime.register(Arc::new(modify_handler(
        "failing",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        error_modifier("boom"),
    )));
    runtime.register(Arc::new(modify_handler(
        "next",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_modifier(" +next"),
    )));

    let result = runtime
        .emit_before_llm_request_modify(&mut payload, HookModifyPolicy::Strict)
        .await;

    assert!(result.is_err());
    let error = result.err().unwrap();
    assert!(error.contains("failing failed: boom"));
    assert_eq!(payload.request_message, "hello");
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(calls.lock().unwrap().as_slice(), ["failing:BeforeLlmRequestModify"]);
}

#[tokio::test]
async fn before_llm_modify_permissive_mode_keeps_current_best_effort_behavior() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_llm_request_payload();

    runtime.register(Arc::new(modify_handler(
        "failing",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        error_modifier("boom"),
    )));
    runtime.register(Arc::new(modify_handler(
        "next",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_modifier(" +next"),
    )));

    let result = runtime
        .emit_before_llm_request_modify(&mut payload, HookModifyPolicy::Permissive)
        .await;

    assert!(result.is_ok());
    assert_eq!(payload.request_message, "hello +next");
    assert_eq!(payload.messages.len(), 3);
    assert_eq!(payload.messages[2].content, "+next");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "failing:BeforeLlmRequestModify",
            "next:BeforeLlmRequestModify"
        ]
    );
}

#[tokio::test]
async fn before_llm_request_modify_continues_after_handler_error() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_llm_request_payload();

    runtime.register(Arc::new(modify_handler(
        "failing",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        error_modifier("boom"),
    )));
    runtime.register(Arc::new(modify_handler(
        "next",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_modifier(" +next"),
    )));

    runtime
        .emit_before_llm_request_modify(&mut payload, HookModifyPolicy::Permissive)
        .await
        .unwrap();

    assert_eq!(payload.request_message, "hello +next");
    assert_eq!(payload.messages.len(), 3);
    assert_eq!(payload.messages[2].content, "+next");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "failing:BeforeLlmRequestModify",
            "next:BeforeLlmRequestModify"
        ]
    );
}

#[tokio::test]
async fn before_action_args_payload_carries_canonical_action_id() {
    let payload = sample_before_action_args_payload();

    assert_eq!(payload.action_id, "builtin__search_memory");
    assert_eq!(payload.action_name, "search_memory");
    assert_eq!(payload.tool_call_id.as_deref(), Some("tool-call-1"));
    assert_eq!(payload.source.as_deref(), Some("chat"));
    assert_eq!(payload.args.get("query"), Some(&"kokoro".to_string()));
}

#[tokio::test]
async fn before_action_args_modify_applies_in_registration_order() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_action_args_payload();

    runtime.register(Arc::new(action_args_modify_handler(
        "first",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        append_action_arg_modifier("query", "kokoro refined"),
    )));
    runtime.register(Arc::new(action_args_modify_handler(
        "second",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        append_action_arg_modifier("limit", "5"),
    )));

    runtime
        .emit_before_action_args_modify(&mut payload, HookModifyPolicy::Permissive)
        .await
        .unwrap();

    assert_eq!(payload.action_id, "builtin__search_memory");
    assert_eq!(
        payload.args.get("query"),
        Some(&"kokoro refined".to_string())
    );
    assert_eq!(payload.args.get("limit"), Some(&"5".to_string()));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "first:BeforeActionArgsModify",
            "second:BeforeActionArgsModify"
        ]
    );
}

#[tokio::test]
async fn before_action_args_modify_continues_after_handler_error() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_action_args_payload();

    runtime.register(Arc::new(action_args_modify_handler(
        "failing",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        action_arg_error_modifier("boom"),
    )));
    runtime.register(Arc::new(action_args_modify_handler(
        "next",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        append_action_arg_modifier("limit", "3"),
    )));

    runtime
        .emit_before_action_args_modify(&mut payload, HookModifyPolicy::Permissive)
        .await
        .unwrap();

    assert_eq!(payload.args.get("query"), Some(&"kokoro".to_string()));
    assert_eq!(payload.args.get("limit"), Some(&"3".to_string()));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "failing:BeforeActionArgsModify",
            "next:BeforeActionArgsModify"
        ]
    );
}

#[tokio::test]
async fn before_action_args_modify_skips_handlers_without_matching_event() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut payload = sample_before_action_args_payload();

    runtime.register(Arc::new(action_args_modify_handler(
        "other",
        &[HookEvent::BeforeLlmRequest],
        calls.clone(),
        append_action_arg_modifier("query", "changed"),
    )));

    runtime
        .emit_before_action_args_modify(&mut payload, HookModifyPolicy::Permissive)
        .await
        .unwrap();

    assert_eq!(payload.args.get("query"), Some(&"kokoro".to_string()));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn action_gate_deny_still_short_circuits_with_action_args_modify_available() {
    let runtime = HookRuntime::new();
    let calls = Arc::new(Mutex::new(Vec::new()));

    runtime.register(Arc::new(deny_handler(
        "deny",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        "blocked",
    )));
    runtime.register(Arc::new(action_args_modify_handler(
        "modifier",
        &[HookEvent::BeforeActionInvoke],
        calls.clone(),
        append_action_arg_modifier("query", "changed"),
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
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["deny:BeforeActionInvoke"]
    );
}
