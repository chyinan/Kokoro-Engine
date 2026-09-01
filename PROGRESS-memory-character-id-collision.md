# Progress: isolate memories for same-named characters

> Created: 2026-09-01 | Status: complete pending user verification

## Goal

Ensure the Memory page shows memories for the selected character instance even when a user-created character has the same display name as a preset character.

## Success criteria

- Character selection and memory queries use the unique character ID, not the display name.
- Same-named characters remain independently selectable and their memories never mix or disappear.
- Existing memory enable/disable, search, timeline, graph, and dream views keep working.
- A regression test covers two character records with the same name and different IDs.

## Files read

- `src/ui/widgets/MemoryPanel.tsx` — selector options were filtered by display name, hiding same-named instances; memory queries already pass `selectedCharId`.
- `src/lib/kokoro-bridge.ts` — `listMemories` accepts and forwards `characterId` as `character_id`.
- `src-tauri/src/commands/memory.rs` — list/count/dream commands receive character IDs.
- `src-tauri/src/characters/instance_resource.rs` — unrelated avatar protocol parsing was also fixed in the previous task and remains in progress verification.

## Current progress

Added a failing pure regression test for same-named selector options. Implemented ID-preserving option construction and synchronized the selector with the active character ID after character-list loading.

## Next step

Restart the app and verify that both same-named entries can be selected and show their own memories. The targeted Rust protocol test from the previous avatar fix remains blocked before test startup by the existing ONNX DLL runtime issue.
