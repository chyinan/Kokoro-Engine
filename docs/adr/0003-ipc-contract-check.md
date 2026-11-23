# ADR 0003 — Rust-owned Tauri IPC bindings

## Status

Accepted

## Context

The command-name check could catch a removed command. It could not check argument keys, DTO fields, replies, or event payloads. Several window and service entry points also called the Tauri APIs directly.

## Decision

The Tauri application and the exporter use the same Rust IPC builder in `src-tauri/src/ipc.rs`. The generated TypeScript consumer is committed as `src/lib/generated/ipc.ts`. The application imports its command functions and event bindings. Adapters in `src/lib/kokoro-bridge.ts` retain application defaults, legacy input handling, and structured command error normalization.

### Application callers

- `kokoro-bridge.ts` covers engine, chat, character, conversation, provider configuration, memory, tools, MCP, registry, backup, MOD, Live2D, speech, and bot commands. It also adapts chat lifecycle, failure, vision, image generation, model-download, MOD, TTS, and bot events.
- `App.tsx`, `PetWindow.tsx`, `PetTab.tsx`, `BubbleWindow.tsx`, and `usePetChat.ts` cover pet and bubble commands and the auxiliary windows' model selection, committed runtime, configuration, visibility, input toggle, and bubble text events.
- `tts-service.ts`, `useVoiceInput.ts`, `useWakeWord.ts`, and `ChatPanel.tsx` cover targeted TTS playback, playback errors, the TTS-to-STT voice interruption signal, STT microphone and wake-word events, and audio stream commands. Playback listeners match the server's target window label.
- `Live2DViewer.tsx`, `useTypingReveal.ts`, `MemoryPanel.tsx`, and `SettingsPanel.tsx` consume Live2D, idle behavior, chat cue and typing, memory updates, and QQ authorization events. The application also consumes QQ authorization requests and expiration in `App.tsx`.
- `IframeSandbox.tsx` checks MOD permissions before dispatching an untrusted iframe request. `mod-ipc.ts` validates arguments to supported commands and calls the generated functions. The legacy `test_llm_connection` name maps to a generated MOD admission command that deserializes its JSON using the existing Rust configuration type and defaults before testing the connection. The typed configuration command remains available to the trusted UI. Historical unregistered command names return a missing-command rejection. Dialog and other Tauri plugin APIs retain their own contracts.

### Generation and CI

When a Rust IPC signature or event changes, from the repository root run:

```sh
npm run generate:ipc
npm run check:ipc
npx tsc --noEmit
```

The generator runs as a Cargo example, outside the desktop binary targets Tauri builds and packages. It uses the project's Rust and native build dependencies without starting Tauri or loading providers, credentials, user data, or models. `check:ipc` compares a fresh export to the committed file and checks that application callers use the generated transport. CI checks regeneration in the Rust job and checks callers and compile-time rejection cases in the frontend job. The regression tests exercise existing command and event handling through the application call sites with the Tauri transport mocked.

The native lifecycle scripts deliberately speak to the wire directly to check desktop integration. Browser DOM events, MOD iframe messages and plugin IPC keep their own contracts.
