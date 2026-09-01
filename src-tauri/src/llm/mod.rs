// pattern: Mixed (unavoidable)
// Reason: 该文件只负责集中声明 LLM 子模块；协议转换、进程 shell 与 provider 实现分层维护。
pub mod anthropic;
pub mod codex_runtime;
pub mod codex_runtime_protocol;
pub mod context;
pub mod llama_cpp;
pub mod llm_config;
pub mod messages;
pub mod ollama;
pub mod provider;
pub mod responses;
mod responses_protocol;
pub mod service;
