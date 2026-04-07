use crate::hooks::runtime::HookHandler;
use crate::hooks::types::{HookEvent, HookOutcome, HookPayload};
use async_trait::async_trait;

pub struct AuditLogHookHandler;

#[async_trait]
impl HookHandler for AuditLogHookHandler {
    fn id(&self) -> &str {
        "audit_log"
    }

    fn events(&self) -> &'static [HookEvent] {
        const EVENTS: &[HookEvent] = &[
            HookEvent::BeforeUserMessage,
            HookEvent::AfterUserMessagePersisted,
            HookEvent::BeforeLlmRequest,
            HookEvent::AfterLlmResponse,
            HookEvent::BeforeActionInvoke,
            HookEvent::AfterActionInvoke,
            HookEvent::BeforeTtsPlay,
            HookEvent::AfterTtsPlay,
            HookEvent::OnModLoaded,
            HookEvent::OnModUnloaded,
        ];
        EVENTS
    }

    async fn handle(
        &self,
        event: &HookEvent,
        payload: &HookPayload,
    ) -> Result<HookOutcome, String> {
        tracing::info!(target: "hooks", "[Hook] event={:?} payload={:?}", event, payload);
        Ok(HookOutcome::Continue)
    }
}
