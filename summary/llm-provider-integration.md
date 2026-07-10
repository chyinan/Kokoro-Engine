# LLM Provider Integration Summary

## Scope

Kokoro Engine's main chat path is provider-based. The frontend persists an `LlmConfig` through Tauri IPC, the Rust `LlmService` builds providers, and `stream_chat` consumes provider-neutral stream events.

## Current Boundaries

- `src/lib/kokoro-bridge.ts` mirrors `LlmConfig` and calls the existing `get_llm_config`, `save_llm_config`, `test_llm_connection`, and `stream_chat` commands.
- `src/ui/widgets/settings/ApiTab.tsx` creates and edits provider records. Provider-specific settings can already be stored in `providers[].extra`.
- `src-tauri/src/llm/llm_config.rs` persists provider records. Existing records have no protocol discriminator and default to `provider_type: "openai"`.
- `src-tauri/src/llm/service.rs` selects an implementation from `provider_type`. The connection test uses the same `LlmProvider::chat` method as production.
- `src-tauri/src/llm/provider.rs` defines the provider-neutral interface and `LlmStreamEvent::{Text, ReasoningContent, ToolCall}`. Its `OpenAIProvider` posts to `/chat/completions` and parses Chat Completions SSE.
- `src-tauri/src/commands/chat.rs` owns conversation history, streaming UI events, tool execution, persistence, and multi-round continuation. It does not expose provider protocol details to the frontend.
- `src-tauri/src/llm/messages.rs` builds Chat Completions-shaped messages, including assistant tool calls and tool results. A Responses adapter must translate these into Responses input items.

## Compatibility Implications

- Do not change the meaning of `provider_type: "openai"`; many OpenAI-compatible servers only implement Chat Completions.
- Add a separate opt-in `openai_responses` provider type and leave defaults unchanged.
- Keep the existing Tauri commands and frontend chat event names unchanged.
- Implement Responses request/message/tool/SSE translation inside a new provider module.
- Start with stateless requests (`store: false`) using Kokoro's existing database-backed history as the source of truth. Do not introduce `previous_response_id` in the first release.
- Preserve opaque reasoning/output items during an in-memory native-tool loop when required by Responses reasoning models; do not expose those items through public IPC.
- Independent OpenAI STT, TTS, image generation, and direct VLM endpoints are separate protocols and remain unchanged. Vision configured to reuse the active LLM can benefit indirectly.

## Existing Test Patterns

- Rust provider HTTP behavior uses `wiremock` in existing provider modules.
- `LlmService` already tests provider selection, normalization, connection tests, and atomic configuration updates.
- Frontend configuration behavior is concentrated in `ApiTab.tsx` and bridge types.
