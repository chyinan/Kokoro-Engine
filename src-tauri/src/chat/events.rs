use super::runtime::{relay_chat_event, ChatRuntimeState, ChatTurnSnapshot};
use crate::chat::event_payloads::{
    ChatFailure, ChatImageGenError, ChatImageGenEvent, ChatTurnAcknowledgedEvent,
    ChatTurnBoundEvent, ChatTurnDeltaEvent, ChatTurnFinishEvent, ChatTurnStartEvent,
    ChatTurnTextCompleteEvent, ChatTurnToolEvent, ChatTurnTranslationEvent, ChatTypingEvent,
    ChatWarningEvent, CueEvent, ImageGenDone, MemoryUpdated,
};
use crate::mods::script_queue::ModScriptQueue;
use crate::tts::TtsPlayback;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri_specta::Event;

pub(crate) enum ChatEvent {
    Acknowledged(ChatTurnAcknowledgedEvent),
    Bound(ChatTurnBoundEvent),
    Cue(CueEvent),
    Delta(ChatTurnDeltaEvent),
    Failure(ChatFailure),
    Finished(ChatTurnFinishEvent),
    ImageGenerated(ImageGenDone),
    ImageGenerationError(ChatImageGenError),
    ImageGenerationRequested(ChatImageGenEvent),
    MemoryUpdated(MemoryUpdated),
    RuntimeChanged(ChatTurnSnapshot),
    Started(ChatTurnStartEvent),
    TextComplete(ChatTurnTextCompleteEvent),
    Tool(ChatTurnToolEvent),
    Translation(ChatTurnTranslationEvent),
    Typing(ChatTypingEvent),
    Warning(ChatWarningEvent),
}

impl ChatEvent {
    fn into_wire(self) -> (&'static str, serde_json::Result<Value>) {
        fn encode<T: Event + Serialize>(payload: T) -> (&'static str, serde_json::Result<Value>) {
            (T::NAME, serde_json::to_value(payload))
        }

        match self {
            Self::Acknowledged(payload) => encode(payload),
            Self::Bound(payload) => encode(payload),
            Self::Cue(payload) => encode(payload),
            Self::Delta(payload) => encode(payload),
            Self::Failure(payload) => encode(payload),
            Self::Finished(payload) => encode(payload),
            Self::ImageGenerated(payload) => encode(payload),
            Self::ImageGenerationError(payload) => encode(payload),
            Self::ImageGenerationRequested(payload) => encode(payload),
            Self::MemoryUpdated(payload) => encode(payload),
            Self::RuntimeChanged(payload) => encode(payload),
            Self::Started(payload) => encode(payload),
            Self::TextComplete(payload) => encode(payload),
            Self::Tool(payload) => encode(payload),
            Self::Translation(payload) => encode(payload),
            Self::Typing(payload) => encode(payload),
            Self::Warning(payload) => encode(payload),
        }
    }
}

pub(crate) trait ChatDesktop: Send + Sync {
    fn emit(&self, name: &'static str, payload: Value) -> Result<(), String>;
    fn window_is_open(&self, label: &str) -> bool;
    fn playback(&self, runtime: Arc<ChatRuntimeState>) -> Option<Box<dyn TtsPlayback>>;
}

#[derive(Clone)]
pub(crate) struct ChatEvents {
    pub(crate) runtime: Arc<ChatRuntimeState>,
    pub(crate) mods: Option<ModScriptQueue>,
    pub(crate) desktop: Option<Arc<dyn ChatDesktop>>,
}

impl ChatEvents {
    pub(crate) fn finish_playback(&self, request_id: &str, label: &str, error: Option<String>) {
        if let Some(snapshot) = self.runtime.finish_playback(request_id, label, error) {
            self.notify(ChatEvent::RuntimeChanged(snapshot));
        }
    }

    pub(crate) fn notify(&self, event: ChatEvent) {
        if self.desktop.is_none() {
            return;
        }
        let (name, payload) = event.into_wire();
        self.notify_wire(name, payload);
    }

    fn notify_wire(&self, name: &'static str, payload: serde_json::Result<Value>) {
        let Some(desktop) = &self.desktop else {
            return;
        };
        let result = payload
            .map_err(|error| error.to_string())
            .and_then(|payload| desktop.emit(name, payload));
        if let Err(error) = result {
            tracing::warn!(target: "chat", "event {name} delivery failed: {error}");
        }
    }

    /// Update the journal before notifying desktop subscribers. An absent
    /// subscriber has no effect on a turn's outcome or persisted reply.
    pub(crate) fn publish(&self, desktop: bool, event: ChatEvent) {
        let (name, value) = event.into_wire();
        if !desktop {
            if let Ok(value) = value {
                self.runtime.event(name, &value);
            }
            return;
        }
        let mut emitted = match value {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(target: "chat", "event {name} serialization failed: {error}");
                return;
            }
        };
        if let Some(id) = emitted["client_request_id"]
            .as_str()
            .or_else(|| emitted["turn_id"].as_str())
        {
            if let Some(scope) = self.runtime.turn_scope(id) {
                emitted["character_id"] = scope.character_id.into();
                if emitted.get("conversation_id").is_none_or(Value::is_null) {
                    emitted["conversation_id"] = scope.conversation_id.into();
                }
                emitted["client_request_id"] = scope.client_request_id.into();
                if emitted
                    .get("conversation_generation")
                    .is_none_or(Value::is_null)
                {
                    emitted["conversation_generation"] = scope.conversation_generation.into();
                }
                if emitted.get("turn_id").is_none() {
                    emitted["turn_id"] = scope.turn_id.into();
                }
            }
        }
        if let Some(snapshot) = relay_chat_event(
            Some(&self.runtime),
            self.mods.as_ref(),
            name,
            emitted.clone(),
        ) {
            self.notify(ChatEvent::RuntimeChanged(snapshot));
        }
        self.notify_wire(name, Ok(emitted));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::event_payloads::{ChatEventScope, ChatFinishStatus};
    use std::sync::Mutex;

    #[derive(Default)]
    struct DesktopEvents(Mutex<Vec<(&'static str, Value)>>);

    impl ChatDesktop for DesktopEvents {
        fn emit(&self, name: &'static str, payload: Value) -> Result<(), String> {
            self.0.lock().unwrap().push((name, payload));
            Ok(())
        }

        fn window_is_open(&self, _: &str) -> bool {
            true
        }

        fn playback(&self, _: Arc<ChatRuntimeState>) -> Option<Box<dyn TtsPlayback>> {
            None
        }
    }

    #[test]
    fn typed_finish_preserves_explicit_null_through_the_desktop_relay() {
        let desktop = Arc::new(DesktopEvents::default());
        let events = ChatEvents {
            runtime: Arc::new(ChatRuntimeState::default()),
            desktop: Some(desktop.clone()),
            mods: None,
        };
        events.publish(
            true,
            ChatEvent::Finished(ChatTurnFinishEvent {
                scope: ChatEventScope {
                    conversation_id: Some(None),
                    ..Default::default()
                },
                turn_id: Some("cancelled-turn".into()),
                status: ChatFinishStatus::Cancelled,
                assistant_message_id: Some(None),
                text: None,
                stage: None,
                code: None,
            }),
        );
        assert_eq!(
            *desktop.0.lock().unwrap(),
            [(
                "chat-turn-finish",
                serde_json::json!({
                    "turn_id": "cancelled-turn",
                    "status": "cancelled",
                    "conversation_id": null,
                    "assistant_message_id": null,
                }),
            )]
        );
    }
}
