// pattern: Imperative Shell

pub mod actions;
pub mod auto_backup;
pub mod backup;
#[cfg(test)]
mod backup_tests;
pub mod bot;
pub mod character;
mod character_instance_core;
pub mod characters;
pub mod chat;
pub mod context;
pub mod conversation;
pub mod database;
pub mod imagegen;
pub mod live2d;
pub mod live2d_protocol;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod mods;
pub mod pet;
pub mod registry;
#[cfg(test)]
mod registry_tests;
pub mod stt;
pub mod system;
pub mod telegram;
pub mod tool_settings;
pub mod tts;
pub mod vision;
