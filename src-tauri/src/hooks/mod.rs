// pattern: Functional Core
pub mod handlers;
pub mod runtime;
pub mod types;

pub use handlers::AuditLogHookHandler;
pub use runtime::{HookHandler, HookRuntime};
pub use types::{
    ActionHookPayload, BeforeActionArgsPayload, BeforeLlmRequestMessage, BeforeLlmRequestPayload,
    ChatHookPayload, HookEvent, HookOutcome, HookPayload, ModHookPayload, TtsHookPayload,
};

#[cfg(test)]
mod tests;
