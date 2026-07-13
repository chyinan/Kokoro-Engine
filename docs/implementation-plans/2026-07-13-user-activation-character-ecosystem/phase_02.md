# Phase 2: Character package and persistence foundation

**Goal:** Add a safe package contract and user-owned character instances without changing chat behavior.

**Execution:** Follow `execution-contract.md`. Dependencies: approved Phase 1 character briefs/licenses. Phase gate includes Rust no-run/check/clippy, frontend tests/build, and diff check.

<!-- START_TASK_1 -->
### Task 1: Add manifest, path, compatibility, and merge cores

**Files:** `src-tauri/src/characters/manifest.rs`, `src-tauri/src/characters/merge.rs`, `src-tauri/src/characters/mod.rs`, `src-tauri/Cargo.toml`

1. Write failing Rust tests for required fields, semantic engine ranges, safe relative assets, secret/custom-endpoint rejection, unsupported files, and three-way merge preservation/conflicts.
2. Implement versioned manifest validation and pure field-by-field/semantic runtime merge logic.
3. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml characters::manifest -- --nocapture`; if runtime loading blocks execution, preserve the failure evidence and require `rtk cargo test --manifest-path src-tauri/Cargo.toml --no-run` to succeed.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add catalog discovery and atomic package installation

**Files:** `src-tauri/src/characters/catalog.rs`, `src-tauri/src/characters/catalog_tests.rs`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`

1. Write tests for catalog discovery, ZIP traversal, size limits, unsupported content, and failed-update preservation.
2. Install bundled resources into app data, discover versioned packages, and atomically replace validated versions.
3. Register `../characters` as a Tauri resource and initialize the catalog before character commands are used.
4. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml characters::catalog -- --nocapture`.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Extend SQLite character instances

**Files:** `src-tauri/migrations/0010_character_ecosystem.sql`, `src-tauri/src/ai/database_migrations.rs`

1. Add template origin/version/snapshot, description, avatar, greeting state, example dialogue, runtime profile, and user edit tracking.
2. Mark existing rows as greeting-consumed while leaving future rows unconsumed.
3. Add migration tests for legacy preservation and new defaults.
4. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml database_migrations::tests -- --nocapture`.
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: Extend character IPC and template instantiation

**Files:** `src-tauri/src/commands/characters.rs`, `src-tauri/src/commands/characters_tests.rs`, `src-tauri/src/lib.rs`, `src/lib/kokoro-bridge.ts`, `src/lib/character-template-update.ts`, `src/lib/character-template-update.test.ts`

1. Write failing CRUD and IPC contract tests for all new fields and validation errors.
2. Add list/create/update/delete, template catalog, instantiate, reconcile, duplicate, and restore-default commands.
3. Return old/user/new values for conflicts so the UI can keep current values, accept selected template values, or create a new instance.
4. Keep existing callers compatible through explicit defaults; do not overwrite conversations, memories, or consumed greetings.
5. RED/GREEN commands: `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::characters_tests -- --nocapture` and `rtk npm test -- src/lib/character-template-update.test.ts`.
<!-- END_TASK_4 -->

<!-- START_TASK_5 -->
### Task 5: Preserve SillyTavern greeting, examples, and PNG avatar

**Files:** `src/lib/character-card-parser.ts`, `src/lib/character-card-parser.test.ts`, `src/ui/widgets/CharacterManager.tsx`

1. Write failing tests for v1/v2/v3 `first_mes`, example dialogue, description, and invalid input.
2. Parse unknown JSON through type guards, keep greeting/example fields separate, and retain PNG imports as avatar resources.
3. Remove persona flattening and pass complete instance data to IPC.
4. RED/GREEN command: `rtk npm test -- src/lib/character-card-parser.test.ts`.
<!-- END_TASK_5 -->

<!-- START_TASK_6 -->
### Task 6: Add backup resource mode and character restore

**Files:** `src-tauri/src/commands/backup.rs`, `src-tauri/src/commands/backup_tests.rs`, `src-tauri/src/commands/auto_backup.rs`, `src/ui/widgets/settings/BackupTab.tsx`, `src/ui/widgets/settings/backup-resource-options.ts`, `src/ui/widgets/settings/backup-resource-options.test.ts`, `src/lib/kokoro-bridge.ts`, `src/ui/locales/en.json`, `src/ui/locales/zh.json`, `src/ui/locales/zh-TW.json`, `src/ui/locales/ja.json`, `src/ui/locales/ko.json`, `src/ui/locales/ru.json`

1. Write tests for data-only default, resource-inclusive export, old backup compatibility, resource validation, and character-table restore.
2. Add `include_character_resources` only to manual export; keep auto backup data-only and warn separately about provider credential configuration.
3. Restore character instances from SQLite and validated resources without depending on stale frontend `characters.json`.
4. On data-only restore, call an injected package resolver. Phase 2 implements and tests local-catalog exact-version resolution plus fallback-only instances; Phase 5 supplies and verifies the remote official-registry resolver.
5. For resource-inclusive restore, validate/stage all resources and remap typed asset references before committing restored rows; rollback staged files on any failure.
6. RED/GREEN commands: `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::backup_tests -- --nocapture` and `rtk npm test -- src/ui/widgets/settings/backup-resource-options.test.ts`.
<!-- END_TASK_6 -->
