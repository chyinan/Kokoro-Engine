use serde_json::Value as JsonValue;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

pub(super) const SCRIPT_EVENT_CAPACITY: usize = 32;

pub(super) struct ScriptModule {
    pub(super) id: String,
    pub(super) scripts: Vec<(String, String)>,
}

pub(super) enum ScriptCommand {
    DispatchEvent {
        event: String,
        payload: JsonValue,
        slot: Option<OwnedSemaphorePermit>,
    },
    ReplaceListeners {
        module: Option<ScriptModule>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// All commands enter the same FIFO so a listener replacement cannot overtake
/// previously accepted events. Only completions and serialized lifecycle
/// changes bypass the limit; chat fragments and cues may be shed under load.
#[derive(Clone)]
pub(crate) struct ModScriptQueue {
    tx: mpsc::UnboundedSender<ScriptCommand>,
    event_slots: Arc<Semaphore>,
}

impl ModScriptQueue {
    pub(super) fn channel() -> (Self, mpsc::UnboundedReceiver<ScriptCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                tx,
                event_slots: Arc::new(Semaphore::new(SCRIPT_EVENT_CAPACITY)),
            },
            rx,
        )
    }

    pub(crate) fn enqueue_chat_event(&self, name: &str, payload: JsonValue) {
        let event = match name {
            "chat-turn-delta" => "chat",
            "chat-turn-finish" => "chat-done",
            "chat-cue" => "cue",
            _ => return,
        };
        let slot = if event == "chat-done" {
            None
        } else {
            let Ok(slot) = self.event_slots.clone().try_acquire_owned() else {
                return;
            };
            Some(slot)
        };
        // The receiver can only disconnect if the runtime has stopped.
        let _ = self.send_event(event, payload, slot);
    }

    fn send_event(
        &self,
        event: &str,
        payload: JsonValue,
        slot: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String> {
        self.tx
            .send(ScriptCommand::DispatchEvent {
                event: event.into(),
                payload,
                slot,
            })
            .map_err(|_| "Script runtime disconnected".to_string())
    }

    #[cfg(test)]
    pub(super) fn available_event_slots(&self) -> usize {
        self.event_slots.available_permits()
    }

    pub(super) async fn dispatch_event(
        &self,
        event: &str,
        payload: JsonValue,
    ) -> Result<(), String> {
        let slot = self
            .event_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;
        self.send_event(event, payload, Some(slot))
    }

    pub(super) async fn replace_listeners(
        &self,
        module: Option<ScriptModule>,
    ) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(ScriptCommand::ReplaceListeners { module, reply })
            .map_err(|_| "Script runtime disconnected".to_string())?;
        result
            .await
            .map_err(|_| "Script runtime disconnected".to_string())?
    }
}
