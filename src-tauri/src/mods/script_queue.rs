use serde_json::Value as JsonValue;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

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
        work: Option<WorkLease>,
    },
    ReplaceListeners {
        module: Option<ScriptModule>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stop,
}

/// All commands enter the same FIFO so a listener replacement cannot overtake
/// previously accepted events. Only completions and serialized lifecycle
/// changes bypass the limit; chat fragments and cues may be shed under load.
#[derive(Clone)]
pub(crate) struct ModScriptQueue {
    tx: mpsc::UnboundedSender<ScriptCommand>,
    event_slots: Arc<Semaphore>,
    work: TaskScope,
    stop: CancellationToken,
}

impl ModScriptQueue {
    #[cfg(test)]
    pub(super) fn channel() -> (Self, mpsc::UnboundedReceiver<ScriptCommand>) {
        let app = crate::lifecycle::AppLifecycle::new();
        Self::channel_with_scope(app.scope(crate::lifecycle::TaskStage::Work, "mods"))
    }

    pub(super) fn channel_with_scope(
        work: TaskScope,
    ) -> (Self, mpsc::UnboundedReceiver<ScriptCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                tx,
                event_slots: Arc::new(Semaphore::new(SCRIPT_EVENT_CAPACITY)),
                work,
                stop: CancellationToken::new(),
            },
            rx,
        )
    }

    pub(super) fn token(&self) -> CancellationToken {
        self.stop.clone()
    }

    pub(super) fn work_token(&self) -> CancellationToken {
        self.work.token()
    }

    pub(super) fn stop(&self) {
        self.stop.cancel();
        self.event_slots.close();
        let _ = self.tx.send(ScriptCommand::Stop);
    }

    pub(super) fn admit(&self, name: &'static str) -> Result<WorkLease, String> {
        self.work.lease(name).map_err(|error| error.to_string())
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
        let _ = self.send_event(event, payload, slot, None);
    }

    fn send_event(
        &self,
        event: &str,
        payload: JsonValue,
        slot: Option<OwnedSemaphorePermit>,
        work: Option<WorkLease>,
    ) -> Result<(), String> {
        if self.stop.is_cancelled() {
            return Err("Script runtime disconnected".to_string());
        }
        self.tx
            .send(ScriptCommand::DispatchEvent {
                event: event.into(),
                payload,
                slot,
                work,
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
        let work = self.admit("script event")?;
        let shutdown = self.work.token();
        let slot = shutdown
            .run_until_cancelled(self.event_slots.clone().acquire_owned())
            .await
            .ok_or("The application is shutting down")?
            .map_err(|err| err.to_string())?;
        self.send_event(event, payload, Some(slot), Some(work))
    }

    pub(super) async fn replace_listeners(
        &self,
        module: Option<ScriptModule>,
    ) -> Result<(), String> {
        if self.stop.is_cancelled() {
            return Err("Script runtime disconnected".into());
        }
        let (reply, result) = oneshot::channel();
        self.tx
            .send(ScriptCommand::ReplaceListeners { module, reply })
            .map_err(|_| "Script runtime disconnected".to_string())?;
        result
            .await
            .map_err(|_| "Script runtime disconnected".to_string())?
    }
}
use crate::lifecycle::{TaskScope, WorkLease};
