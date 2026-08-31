# Repository Guidelines

Last verified: 2026-08-31

## Project Structure & Module Organization
Kokoro Engine is a Tauri v2 app with a React/TypeScript frontend and Rust backend. Frontend code lives in `src/`: `main.tsx` and `App.tsx` are entry points, `src/components/ui` holds primitives, `src/features` holds feature modules, `src/lib` holds bridge/services/tests, `src/ui` holds product UI/locales, and `src/windows` holds extra Tauri windows. Backend code lives in `src-tauri/src`: IPC is in `commands`, with domain modules such as `llm`, `tts`, `stt`, `vision`, `mods`, `mcp`, `registry`, `characters`, `bot`, and `ai`. Content packages are built from `characters/` and `mods/`, registry artifacts live in `registry/`, the AstrBot adapter is in `integrations/astrbot-kokoro/`, and release/community evidence lives in `docs/` and `release-notes/`.

## Build, Test, and Development Commands
- `npm install`: install JavaScript dependencies.
- `npm run tauri dev`: run the full desktop app with Vite.
- `npm run dev`: run the Vite frontend only.
- `npm run build`: typecheck with `tsc` and build frontend assets.
- `npm run tauri build`: build a distributable Tauri app.
- `npm test`: run Vitest unit tests.
- `npm run check:ipc`: verify the typed IPC command registry.
- `cargo test --manifest-path src-tauri/Cargo.toml`: run Rust tests.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`: check Rust warnings before review.
- `node scripts/build-content-registry.mjs`: rebuild deterministic registry archives and index.
- `cd integrations/astrbot-kokoro && pytest -q`: run AstrBot adapter tests.

## Coding Style & Naming Conventions
Use strict TypeScript, React function components, and `@/*` imports when they improve clarity. Match existing two-space TypeScript indentation and Rust `rustfmt` defaults. Name React components and TSX files with `PascalCase`, hooks as `useSomething`, utility files in local style such as `audio-player.ts`, and Rust modules/files with `snake_case`. Keep user-facing strings in `src/ui/locales/*.json`.

## Cross-domain contracts

- `src/lib/kokoro-bridge.ts` is the typed frontend/backend boundary; update Rust command registration and bridge types together.
- Character runtime changes go through the serialized activation owner. Persist per-character overrides through prepare/apply/commit; keep provider secrets and other app-wide credentials separate.
- Registry trust is source-bound: only the canonical official endpoint plus official identity may produce an official label. URL/custom registry installs are community/untrusted and require explicit confirmation.
- Registry and backup writes use staging, canonical containment, size limits, and reparse/symlink checks before promotion. User instances, conversations, memories, and settings survive package removal.
- Webhook and AstrBot payloads use character-scoped conversations, Bearer authentication, bounded bodies, JSON errors, and explicit text/image/audio media contracts. AstrBot audio converted through `Record.convert_to_base64()` is sent as WAV.
- Memory embedding is optional and non-blocking: unavailable semantic retrieval yields a typed unavailable result/BM25 fallback while basic LLM chat remains usable.

## Testing Guidelines
Vitest tests live beside frontend code as `*.test.ts` or `*.test.tsx`. Rust tests are inline `#[cfg(test)]` modules or files such as `src-tauri/src/vision/tests/*.rs`. Add focused tests for changed behavior, especially IPC bridges, providers, memory, chat, audio, and vision. Run the targeted suite for your area plus broader checks when risk is shared, for example `npm test` and `cargo test --manifest-path src-tauri/Cargo.toml llm`.

## Commit & Pull Request Guidelines
Recent history uses short imperative subjects, sometimes with prefixes such as `docs:` or `chore(app):`. Follow that style: `Fix stale vision context` or `docs: update setup notes`. PRs should describe behavioral impact, list test commands run, link related issues, and include screenshots or GIFs for visible UI changes. Call out config, model, or network requirements.

## Security & Configuration Tips
Do not commit secrets, local databases, model files, generated `dist`, or `target*` directories. Keep provider tokens in local config or environment variables. Review changes to permissions, file access, command execution, and remote provider calls carefully.
