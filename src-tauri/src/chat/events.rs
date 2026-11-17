use super::runtime::{relay_chat_event, ChatRuntimeState};
use crate::mods::script_queue::ModScriptQueue;
use crate::tts::TtsPlayback;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) enum ChatEvent {
    Acknowledged,
    Bound,
    Cue,
    Delta,
    Failure,
    Finished,
    ImageGenerated,
    ImageGenerationError,
    ImageGenerationRequested,
    MemoryUpdated,
    RuntimeChanged,
    Started,
    TextComplete,
    Tool,
    Translation,
    Typing,
    Warning,
}

impl ChatEvent {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Acknowledged => "chat-turn-acknowledged",
            Self::Bound => "chat-turn-bound",
            Self::Cue => "chat-cue",
            Self::Delta => "chat-turn-delta",
            Self::Failure => "chat-failure",
            Self::Finished => "chat-turn-finish",
            Self::ImageGenerated => "imagegen:done",
            Self::ImageGenerationError => "chat-imagegen-error",
            Self::ImageGenerationRequested => "chat-imagegen",
            Self::MemoryUpdated => "memory:updated",
            Self::RuntimeChanged => "chat-runtime-changed",
            Self::Started => "chat-turn-start",
            Self::TextComplete => "chat-turn-text-complete",
            Self::Tool => "chat-turn-tool",
            Self::Translation => "chat-turn-translation",
            Self::Typing => "chat-typing",
            Self::Warning => "chat-warning",
        }
    }
}

pub(crate) trait ChatDesktop: Send + Sync {
    fn emit(&self, event: ChatEvent, payload: Value) -> Result<(), String>;
    fn window_is_open(&self, label: &str) -> bool;
    fn playback(
        &self,
        runtime: Arc<ChatRuntimeState>,
    ) -> Option<Box<dyn TtsPlayback>>;
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
            self.notify(ChatEvent::RuntimeChanged, snapshot);
        }
    }

    pub(crate) fn notify(&self, event: ChatEvent, payload: impl Serialize) {
        if let Some(desktop) = &self.desktop {
            if let Err(error) =
                desktop.emit(event, serde_json::to_value(payload).unwrap_or(Value::Null))
            {
                tracing::warn!(target: "chat", "event {} delivery failed: {error}", event.name());
            }
        }
    }

    /// Update the journal before notifying desktop subscribers. An absent
    /// subscriber has no effect on a turn's outcome or persisted reply.
    pub(crate) fn publish(&self, desktop: bool, event: ChatEvent, payload: impl Serialize) {
        let value = serde_json::to_value(payload);
        if !desktop {
            if let Ok(value) = value {
                self.runtime.event(event.name(), &value);
            }
            return;
        }
        let mut emitted = value.unwrap_or(Value::Null);
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
            event.name(),
            emitted.clone(),
        ) {
            self.notify(ChatEvent::RuntimeChanged, snapshot);
        }
        self.notify(event, emitted);
    }
}
