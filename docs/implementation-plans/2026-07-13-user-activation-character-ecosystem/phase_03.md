# Phase 3: Character activation and main-surface catalog

**Goal:** Let users switch complete characters from the main UI through one coordinated activation path.

**Execution:** Follow `execution-contract.md`. Dependencies: Phase 2 catalog, instance schema, and typed IPC. The activation owner is the protocol below; no caller may directly write a subset of active-character runtime state.

<!-- START_TASK_1 -->
### Task 1: Implement activation resolution and greeting transaction

**Files:** `src-tauri/src/characters/activation.rs`, `src-tauri/src/characters/activation_tests.rs`, `src-tauri/src/commands/characters.rs`, `src-tauri/src/lib.rs`, `src/lib/kokoro-bridge.ts`

1. Write tests for override precedence, TTS provider-ID/type/local-preset resolution, optional fallbacks, conversation ownership, empty greeting, one-time greeting, deleted greeting history, and concurrent activation.
2. Add `prepare_character_activation(character_id)` as a read-only IPC returning an activation token with a monotonically increasing activation revision, previous committed snapshot, resolved runtime profile, prompt payload, target conversation, greeting action, and capability recommendations. Serialize prepare/commit through one backend activation mutex and reject stale tokens.
3. Resolve TTS in this order: matching configured `provider_id`; configured default of matching type; allowlisted local preset with conventional loopback endpoint probe and explicit save confirmation; browser/text-only fallback.
4. Add `commit_character_activation(token)` to revalidate the token/revision and open a transaction without committing the greeting. Select/create the target conversation and stage greeting consumption, apply all required backend prompt/name/id/language/proactive state through fallible persistence APIs, persist the complete resolved runtime profile as the backend source of truth, then commit the SQLite transaction last. If backend apply or transaction commit fails, restore the previous backend snapshot and leave greeting unconsumed.
5. Add `get_committed_character_runtime()` so startup, window recreation, and crash recovery reapply the backend committed profile rather than trusting localStorage.
6. Never save a provider secret, custom endpoint, or local model path from character content.
7. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml characters::activation_tests -- --nocapture`.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add a single frontend activation shell

**Files:** `src/features/characters/character-runtime-profile.ts`, `src/features/characters/character-runtime-profile.test.ts`, `src/features/characters/character-activation.ts`, `src/features/characters/character-activation.test.ts`, `src/lib/app-settings.ts`

1. Write failing tests for snapshot, prepare, frontend apply, backend commit, rollback, fallback, stale token, crash recovery, concurrent activation ordering, and exactly one runtime event.
2. The shell must call `prepare_character_activation`, snapshot frontend local/runtime state, apply Live2D/background/TTS/cue-profile settings, then call `commit_character_activation`. If frontend apply or backend commit fails, restore the complete frontend snapshot and emit no event.
3. An allowlisted local TTS preset is probed at its conventional loopback endpoint; the endpoint and result are shown to the user, and config is saved only after explicit confirmation.
4. Treat backend committed runtime as authoritative. Local persistence is a best-effort cache only; on startup or window recreation call `get_committed_character_runtime()` and reapply it before rendering character-dependent UI.
5. Serialize frontend activation requests. Roll back a failed request only when its revision is still current; an older request must never overwrite a newer successful runtime.
6. After success, dispatch `kokoro-character-runtime-changed` once. A cache-write failure does not change the committed result and is repaired from backend state on the next startup.
7. Route App, settings, import, MOD actions, and selectors through this service.
8. RED/GREEN command: `rtk npm test -- src/features/characters/character-activation.test.ts src/features/characters/character-runtime-profile.test.ts`.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Ship three built-in packages and the main catalog

**Files:** `characters/kokoro/character.json`, `characters/kokoro/LICENSE.md`, `characters/kokoro/cues.json`, `characters/pico/character.json`, `characters/pico/LICENSE.md`, `characters/pico/cues.json`, `characters/seren/character.json`, `characters/seren/LICENSE.md`, `characters/seren/cues.json`, `src/ui/widgets/CharacterCatalog.tsx`, `src/ui/widgets/CharacterCatalog.test.ts`, `src/ui/widgets/CharacterRecommendationDialog.tsx`, `src/ui/widgets/CharacterRecommendationDialog.test.ts`, `src/App.tsx`, `src/ui/layout/LayoutRenderer.tsx`, `src/ui/locales/en.json`, `src/ui/locales/zh.json`, `src/ui/locales/zh-TW.json`, `src/ui/locales/ja.json`, `src/ui/locales/ko.json`, `src/ui/locales/ru.json`

1. Add three original manifests, greetings, examples, cue profiles, and license records; use the built-in Live2D fallback where no licensed model exists.
2. Add a compact main-surface selector with name thumbnail/avatar, description, active state, import, edit, duplicate, restore-default, and template conflict resolution actions.
3. Present vision, memory, MCP, and bot recommendations only after successful activation and require explicit consent before enabling sensitive capabilities.
4. Keep the primary Live2D/chat surface visible and avoid opening advanced settings for switching.
5. Test select/import/edit/duplicate/restore-default/conflict actions and prove recommendations render without changing capability settings until explicit confirmation.
6. RED/GREEN command: `rtk npm test -- src/ui/widgets/CharacterCatalog.test.ts src/ui/widgets/CharacterRecommendationDialog.test.ts`.
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: Synchronize chat and harden Live2D deletion

**Files:** `src/ui/widgets/ChatPanel.tsx`, `src/ui/widgets/ConversationSidebar.tsx`, `src/ui/widgets/chat-character-sync.ts`, `src/ui/widgets/chat-character-sync.test.ts`, `src-tauri/src/commands/live2d.rs`

1. Reload the selected character's conversation after activation and never display the prior character's messages.
2. Replace lexical Live2D deletion containment with canonical/strict model ownership checks.
3. Test character conversation sync, fallback on missing model, and traversal attempts.
4. RED/GREEN commands: `rtk npm test -- src/ui/widgets/chat-character-sync.test.ts` and `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::live2d::tests -- --nocapture`.
5. Phase gate: run the full frontend gate and Rust no-run/check/clippy gate from `execution-contract.md`.
<!-- END_TASK_4 -->
