// pattern: Imperative Shell
use crate::hooks::types::{HookEvent, HookOutcome, HookPayload};
use async_trait::async_trait;
use std::sync::{Arc, RwLock};

#[async_trait]
pub trait HookHandler: Send + Sync {
    fn id(&self) -> &str;
    fn events(&self) -> &'static [HookEvent];
    async fn handle(
        &self,
        event: &HookEvent,
        payload: &HookPayload,
    ) -> Result<HookOutcome, String>;
}

#[derive(Default)]
pub struct HookRuntime {
    handlers: RwLock<Vec<Arc<dyn HookHandler>>>,
}

impl HookRuntime {
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(Vec::new()),
        }
    }

    pub fn register(&self, handler: Arc<dyn HookHandler>) {
        self.handlers.write().unwrap().push(handler);
    }

    pub async fn emit(
        &self,
        event: &HookEvent,
        payload: &HookPayload,
    ) -> Result<HookOutcome, String> {
        let handlers = self.handlers.read().unwrap().clone();
        for handler in handlers {
            if !handler.events().iter().any(|candidate| candidate == event) {
                continue;
            }
            handler.handle(event, payload).await?;
        }
        Ok(HookOutcome::Continue)
    }

    pub async fn emit_best_effort(&self, event: &HookEvent, payload: &HookPayload) -> HookOutcome {
        let handlers = self.handlers.read().unwrap().clone();
        for handler in handlers {
            if !handler.events().iter().any(|candidate| candidate == event) {
                continue;
            }
            if let Err(error) = handler.handle(event, payload).await {
                eprintln!(
                    "[Hook] handler={} event={:?} error={}",
                    handler.id(),
                    event,
                    error
                );
            }
        }
        HookOutcome::Continue
    }

    pub async fn emit_action_gate(&self, event: &HookEvent, payload: &HookPayload) -> HookOutcome {
        let handlers = self.handlers.read().unwrap().clone();
        for handler in handlers {
            if !handler.events().iter().any(|candidate| candidate == event) {
                continue;
            }
            match handler.handle(event, payload).await {
                Ok(HookOutcome::Continue) => {}
                Ok(HookOutcome::Deny { reason }) => {
                    return HookOutcome::Deny { reason };
                }
                Err(error) => {
                    eprintln!(
                        "[Hook] handler={} event={:?} error={}",
                        handler.id(),
                        event,
                        error
                    );
                }
            }
        }
        HookOutcome::Continue
    }
}
