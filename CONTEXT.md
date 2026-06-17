# Kokoro Engine Context

Kokoro Engine is a Tauri v2 desktop virtual character interaction engine.

## Domain Terms

- Character: The active virtual persona, including name, persona prompt, Live2D model, cue mapping, memory, and conversation history.
- Chat Turn: One assistant response lifecycle from `chat-turn-start` through deltas, tool traces, text completion, translation, and `chat-turn-finish`.
- Turn Event: A Tauri event emitted by the backend during a Chat Turn.
- Tool Trace: User-visible metadata for a tool call, approval request, approval resolution, result, or error.
- MOD Action: A host action requested by a MOD UI through the `kokoro:mod-action` browser event.
- Runtime Setting: A frontend setting currently persisted through localStorage and sometimes mirrored to backend config.
- Provider Config: A backend persisted configuration for LLM, TTS, STT, Vision, ImageGen, Bot, Telegram, or tools.
- Memory: Character-scoped facts stored in SQLite with embedding, keyword retrieval, observability, and dreaming support.
- Dreaming: Background memory consolidation and proposal generation.
- Cue: A semantic Live2D action or expression that can be triggered by chat, tools, MODs, or interaction events.

## Architecture Priorities

- Keep frontend UI modules thin by moving event sequencing and persistence rules into deeper modules.
- Keep `kokoro-bridge.ts` as a typed IPC seam, but protect it with contract checks.
- Keep backend command modules as IPC adapters. Long-lived behavior should live in domain modules behind smaller interfaces.
- Prefer pure reducer or parser modules for complex state transitions, because they are cheaper to test than Tauri or React integration.
