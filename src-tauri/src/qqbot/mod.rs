// pattern: Imperative Shell

pub mod config;
pub mod protocol;
pub mod runtime;

pub use config::QQBotConfig;
pub(crate) use runtime::run_qq_gateway;
pub use runtime::QQAuthorizationBroker;
