# Generated desktop IPC

The migration starts from `14473c46edfffe71f2ee3779ffc365d6a0308230`. The generated builder preserves the previous 187 application commands, adds one JSON-input adapter for the MOD LLM connection test, and registers 55 application events for generation. [Shared Rust registry](../src-tauri/src/ipc.rs)

| Command module | Count | Command responsibilities |
| --- | ---: | --- |
| `system`, `character`, `database` | 9 | Runtime/release status, window size, character state, cue and message, database setup and vector store diagnostics |
| `chat`, `context`, `conversation` | 43 | Turn admission, recovery, playback, settings, history, approvals, conversation and message edits |
| `characters`, `registry` | 19 | Instance CRUD, activation, template reconciliation, package discovery, install and removal |
| `tts`, `stt`, `stt::stream` | 25 | Configuration, provider/model discovery, synthesis, capture, wake word, audio streaming |
| `mods`, `live2d`, `imagegen`, `vision` | 32 | MOD lifecycle and scripting, Live2D models and profiles, image generation, vision watcher and capture |
| `memory` | 12 | Memory CRUD, dreaming, proposal handling, embedding model status and download |
| `llm`, `actions`, `tool_settings`, `mcp` | 20 | Model/runtime configuration and discovery, actions, tool policy, MCP lifecycle |
| `bot`, `telegram` | 11 | Bot configuration, authorization, lifecycle and status |
| `backup`, `auto_backup`, `pet` | 17 | Backup export/import/scheduling, pet and bubble windows and configuration |

The event builder covers chat lifecycle and projection, cue and typing, image generation, memory notifications and model download, MOD layout/theme/components/scripts, character activation and Live2D, QQ authorization, desktop visibility and pet/bubble updates, STT, TTS, and vision. Chat producers construct the same Rust payload types used for export. The desktop relay serializes those payloads for the existing journal and MOD delivery. A few legacy channels still have frontend subscribers, including `chat-error`, `imagegen:error`, `camera-observation`, `telegram:chat-sync`, and `toggle-chat-input`. Cross-window signals share the registry with backend events. TTS delivery keeps the target window label and uses a typed event subscription in that window.

## Dependency compatibility

The Cargo manifest pins `tauri-specta` `2.0.0-rc.29`, `specta` `2.0.0-rc.28`, and `specta-typescript` `0.0.15`. Tauri Specta's command collection needs the command-name macro added in released `tauri-macros` `2.6.0`. The lock uses `tauri-macros` and `tauri-codegen` `2.6.0`, `tauri-utils` `2.9.0`, Tauri `2.10.3`, and `tauri-build` `2.5.6`. The intermediate Tauri Specta `2.0.0-rc.25` pinned with the original macro version treated some Serde defaults as optional output fields and used one TypeScript shape for both listening and emission. The selected release generates distinct command input and output shapes where required and distinct event listen and emit shapes. [Tauri Specta command macro](https://docs.rs/crate/tauri-specta/2.0.0-rc.29/source/src/macros.rs) · [Tauri macro source](https://docs.rs/crate/tauri-macros/2.6.0/source/src/command/wrapper.rs) · [Specta Serde phases](https://docs.rs/crate/specta-serde/0.0.15/source/src/lib.rs)

## Wire conventions

- Tauri deserializes command inputs with Serde and still rejects commands that return an error. The generated `KokoroError` records the structured `code` and `message` payload shape. Legacy commands also return string errors and framework failures may reject independently. The frontend error adapter handles the rejection as `unknown`.
- Existing Serde defaults apply on omitted input fields. Before the migration Serde decoded both an omitted and a null `avatar_path` as no update. An omitted value still retains the current avatar, and an explicit `null` now clears it, matching the Remove Avatar action. [Rust fixture](../src-tauri/src/ipc/tests.rs) · [Character Manager](../src/ui/widgets/CharacterManager.tsx)
- Integral JSON numbers must be within `-(2^53 - 1)` through `2^53 - 1`. The Rust invoke handler validates nested numeric inputs before dispatch, including large integral exponent values. Commands with wide response fields serialize through an IPC-only guard, and guarded event emissions check the same range. A larger integer from an imported or externally edited database fails when returned to the WebView. SQLite and persisted configuration retain their full native precision. Telegram Bot API chat IDs use at most 52 significant bits. Use a decimal string DTO for a new full-range IPC identifier. [Telegram Bot API Chat schema](https://core.telegram.org/bots/api#chat)
- Dynamic provider options and MOD payloads use recursive JSON values. The MOD LLM test adapter accepts one JSON configuration value and delegates parsing and defaults to `LlmConfig` before entering the shared test command. Binary payloads currently sent as `Vec<u8>` remain JSON arrays of bytes and are range-checked by Rust deserialization. Bindings contain schema information, including names of existing configuration fields, independently of user credentials.

Run `npm run generate:ipc` to regenerate, and `npm run check:ipc` to verify the committed file against a fresh export. Both use the `export-ipc` Cargo example, which instantiates the shared builder without starting the desktop application. The Windows example embeds the Common Controls manifest required by its linked native libraries. `cargo run --locked --manifest-path src-tauri/Cargo.toml --example export-ipc -- --check <bindings-file>` compares an alternate file for drift testing. [Drift checking and adapters](adr/0003-ipc-contract-check.md)

## Migration verification

- The generated artifact contains 188 distinct commands and 55 distinct events. The 187 baseline command names match the previous handler in order after excluding the new MOD adapter.
- Consecutive Windows exports matched SHA-256 `4a1073301372613e347f39e0453c3f7c9b07bcb69805d47a280ef4d7a70f9857`. The exporter accepted the current file and rejected isolated stale and missing files with exit code 1. `npm run check:ipc` passed both the generated file comparison and the frontend transport usage guard.
- A hidden Windows Wry WebView exercised the registered invoke handler. It returned engine metadata, rejected an empty shortcut with a structured business error, rejected malformed arguments and a missing command through Tauri, and rejected an integer above the safe range before dispatch. Rust fixtures also covered recursive JSON, the range in both directions, and the distinction between absent and null avatar updates. [IPC fixtures](../src-tauri/src/ipc/tests.rs)
- The frontend build verified seven intentional TypeScript errors for invalid command, argument, response, and event usage. Vitest passed 992 tests in 92 suites with the local launcher test file `scripts/tauri.test.mjs` excluded. [TypeScript fixtures](../src/lib/ipc.contract-test.ts)
