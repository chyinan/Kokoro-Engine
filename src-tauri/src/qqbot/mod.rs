// pattern: Imperative Shell

pub mod config;
pub mod protocol;
pub mod runtime;

pub use config::QQBotConfig;
pub use runtime::{run_qq_gateway, QQAuthorizationBroker};
