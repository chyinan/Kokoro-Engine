# Progress: audit and fix character catalog

> Created: 2026-08-31 | Status: complete pending user verification

## Goal
Audit whether the character catalog is fully implemented and identify the `[object Object]` and other reproducible bugs.

## Success criteria
Inspect card display, description clamping, scrollbar, select, import, edit, duplicate, delete, restore defaults, conflict resolution, and error feedback. Each conclusion must have code or test evidence and be classified as implemented, defective, or uncovered.

## Files read
- `src/ui/widgets/CharacterCatalog.tsx` — cards, selection, and five action buttons share `runAction`; failures use `String(reason)`.
- `src/App.tsx` — selection, template instantiation, duplicate, restore, conflict resolution, and import dependencies.
- `src/lib/kokoro-bridge.ts` — structured IPC errors remain objects; `getKokoroErrorMessage` already exists.
- `src/ui/widgets/CharacterManager.tsx` — Settings has a separate CRUD/import state and error path.
- `src/features/characters/character-runtime-overrides.ts` — edit entry validates the requested character ID.
- `src-tauri/src/commands/characters.rs` — backend CRUD, template instantiation, restore, and reconciliation commands.
- `src-tauri/src/commands/character_instance_core.rs` — template defaults and reconciliation pure logic.
- `src/ui/widgets/CharacterCatalog.test.ts` — covers display class, dependency routing, and plain `Error` failures, but not structured errors or App-level dependencies.
- `src/lib/kokoro-bridge.error.test.ts` — structured errors are normalized, but the catalog does not use the helper.
- `characters/*/character.json` — built-in Kokoro, Pico, and Seren templates are all version 1.0.0.

## Current progress
Audit findings were fixed, including the follow-up persisted-language-code display bug.

## Next step
Restart the app and verify the unified character list, settings display, and custom-avatar replacement/removal behavior; Rust test execution remains limited by the documented ONNX DLL runtime issue.

## Key findings
- Structured IPC errors become `{ code, message, ... }` objects; `CharacterCatalog.runAction` converts them to `[object Object]`.
- `importCharacter` only dispatches an event and returns immediately; App-side file selection, parsing, persistence, and activation failures are logged but not returned to the catalog error area.
- `resolveTemplateConflict` selects the available template using `localeCompare`, not SemVer ordering; versions such as 1.10.0 and 1.2.0 would be ordered incorrectly.
- `templates.map` renders every discovered template version; when one instance is matched by multiple versions, entries reuse the same action ID/key and duplicate cards can appear.
- Settings `CharacterManager` owns a separate local character list; its create/edit/import/delete mutations do not update App's catalog list.
- The phase-3 design calls for a preview surface, but the catalog currently exposes only card text and action buttons.
- Rust tests compile with `cargo test --no-run`; test execution is blocked by the documented Windows `onnxruntime.dll` `STATUS_ENTRYPOINT_NOT_FOUND` error.
- Audit report: `summary/character-catalog-audit.md`.
- Fixed structured error rendering in the catalog and recommendation dialog.
- Made catalog import await the file operation, surface failures, and support cancellation.
- Added catalog retry/error UI and synchronized Settings character mutations in both directions.
- Deduplicated template versions, selected newest versions with SemVer ordering, and hid reconciliation when no update exists.
- Added full character preview with localized labels, Escape/Tab handling, initial focus, and focus restoration.
- Normalized persisted language codes case-insensitively so first-open settings show preset labels instead of a custom input.
- Built-in/installed templates are provisioned once into ordinary character instances and the catalog renders instances only, so Settings and the main catalog share edit/delete state.
- The compatibility marker uses `kokoro_provisioned_character_template_ids_v2` so existing installations with an older marker receive the three shipped bundled templates.
- `characters/template` remains a creator documentation scaffold but is excluded from Tauri resources, bundled installation, and the user-facing template list.
- Verified the current local Roaming catalog contains four package manifests only because the old scaffold is still installed from an earlier build; the new discovery filter hides it without deleting user data.
- Character summaries use the primary text color.
- Custom avatars now have Settings preview/select/remove controls, preserve the existing default icon fallback, use cache-busted/no-store URLs after replacement, and report cleanup/rollback failures.
- Managed avatar roots and files reject symlink/reparse redirects before access or cleanup.
- Final verification after the avatar hardening: frontend 45 test files / 253 tests passed; build, IPC check, Rust no-run/check/Clippy, and diff check passed. Targeted Rust execution remains blocked before test startup by the existing Windows ONNX DLL `STATUS_ENTRYPOINT_NOT_FOUND` error.
- Custom avatar audit: backend persistence, Settings controls, and main-catalog rendering are implemented. Details: `summary/custom-avatar-audit.md`.
