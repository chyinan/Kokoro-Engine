# Same-named character memory isolation

> Audit date: 2026-09-01

## Root cause

`src/ui/widgets/MemoryPanel.tsx` filtered selector options with a display-name comparison. When a user-created character and a preset shared the name `Kokoro`, the non-current entry was removed even though `listMemories` already queried by the selected character ID.

## Fix

- Added `src/ui/widgets/memory/character-memory-selection.ts` as a pure selector-option builder keyed by character ID.
- MemoryPanel now renders every character record and marks only the active ID.
- Character-list loading synchronizes the selected memory character to the active character ID, while preserving user selection until the active character actually changes.

## Regression coverage

`src/ui/widgets/memory/character-memory-selection.test.ts` verifies two `Kokoro` records with different IDs remain separate options and retain the correct active label.

## Verification

- Targeted regression test: passed.
- Full frontend suite: 46 test files / 254 tests passed.
- Frontend build, IPC contract check, Rust `test --no-run`, `cargo check`, Clippy, format check, and `git diff --check`: passed.
- The targeted Rust avatar protocol test remains blocked before test execution by the existing Windows `onnxruntime.dll` `STATUS_ENTRYPOINT_NOT_FOUND` error.
