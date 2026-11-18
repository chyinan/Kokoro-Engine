pub mod bridge;
mod client;
pub mod manager;
mod transport;

pub use manager::{McpManager, McpServerConfig};

#[cfg(test)]
mod tests;
