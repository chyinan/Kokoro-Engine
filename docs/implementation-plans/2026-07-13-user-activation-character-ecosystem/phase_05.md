# Phase 5: Minimal character and MOD registry

**Goal:** Browse and install official content from a static, versioned registry without building a marketplace backend.

**Execution:** Follow `execution-contract.md`. Dependencies: Phase 2 package contract and Phase 3 activation/catalog. Default official endpoint: `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json`, with a user-overridable HTTPS endpoint for development.

<!-- START_TASK_1 -->
### Task 1: Define and validate the static registry contract

**Files:** `registry/v1/index.json`, `registry/schema/registry-v1.schema.json`, `registry/packages/.gitkeep`, `scripts/build-content-registry.mjs`, `scripts/build-content-registry.test.mjs`, `src-tauri/src/registry/manifest.rs`, `src-tauri/src/registry/mod.rs`

1. Write tests for content type, compatibility, HTTPS URL, checksum, size, trust label, permissions/recommendations, duplicate IDs, and trust-source normalization.
2. Build versioned character/MOD ZIPs deterministically into `registry/packages/`, calculate SHA-256 and byte sizes, and generate `registry/v1/index.json` from those artifacts.
3. Bind the official label to the exact built-in canonical endpoint plus an official-registry identity constant; any user-overridden registry endpoint is non-official regardless of entry metadata.
4. Verify every index URL basename, checksum, size, manifest version, and engine range against the generated archive before publication.
5. Document upload of the JSON and archives to the official GitHub release/raw locations as an external release action.
6. RED/GREEN commands: `rtk npm test -- scripts/build-content-registry.test.mjs` and `rtk cargo test --manifest-path src-tauri/Cargo.toml registry::manifest -- --nocapture`.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add registry fetch and character installation commands

**Files:** `src-tauri/src/registry/client.rs`, `src-tauri/src/commands/registry.rs`, `src-tauri/src/commands/registry_tests.rs`, `src-tauri/src/characters/catalog.rs`, `src-tauri/src/lib.rs`, `src/lib/kokoro-bridge.ts`

1. Write tests for checksum mismatch, incompatible/corrupt/truncated archives, traversal, size limits, and script/HTML/executable rejection.
2. Download to a temporary file, validate checksum and manifest, extract to staging, then atomically install/update.
3. Remove package resources without deleting user instances, conversations, memories, or settings; switch an active removed package to the built-in presentation fallback first.
4. Test that data-only restore can fetch the exact official package version and that unavailable versions retain usable fallback presentation.
5. Add install-character-from-URL using the same declarative validation path; force URL installs and all custom-registry installs to non-official, and prove package/registry metadata cannot self-assert the official label.
6. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::registry_tests -- --nocapture`.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Harden registry-backed MOD updates

**Files:** `src-tauri/src/commands/mods.rs`, `src-tauri/src/commands/mods_registry_tests.rs`, `src-tauri/src/mods/manifest.rs`

1. Enforce engine compatibility and explicit rejection of invalid entries.
2. Preserve the previous MOD until staging extraction and permission review succeed.
3. Add update/remove commands and an explicit untrusted-code warning for URL installs.
4. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::mods_registry_tests -- --nocapture`.
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: Add the content library UI

**Files:** `src/ui/widgets/ContentLibrary.tsx`, `src/ui/widgets/ContentLibrary.test.tsx`, `src/ui/widgets/content-library-state.ts`, `src/ui/widgets/content-library-state.test.ts`, `src/ui/widgets/SettingsPanel.tsx`, `src/App.tsx`, `src/ui/locales/en.json`, `src/ui/locales/zh.json`, `src/ui/locales/zh-TW.json`, `src/ui/locales/ja.json`, `src/ui/locales/ko.json`, `src/ui/locales/ru.json`

1. Add separate Character and MOD tabs with preview, compatibility, trust, permissions/recommendations, install/update/remove, and URL install.
2. Keep official labels exclusive to official registry entries.
3. Show actionable download, validation, and compatibility failures.
4. Test Character/MOD separation, trust labels, install/update/remove states, URL warning confirmation, and error recovery.
5. RED/GREEN command: `rtk npm test -- src/ui/widgets/ContentLibrary.test.tsx src/ui/widgets/content-library-state.test.ts`.
<!-- END_TASK_4 -->

<!-- START_TASK_5 -->
### Task 5: Publish creator templates and validation guidance

**Files:** `docs/content-registry.md`, `docs/creating-character-packages.md`, `docs/creating-mod-packages.md`, `characters/template/character.json`, `characters/template/LICENSE.md`, `characters/template/cues.json`

1. Document allowed files, manifests, licenses, checksums, compatibility, validation, and PR submission.
2. Provide a non-executable character package template and command-line validation examples.
3. External evidence: record official JSON/archive URLs, SHA-256 values, and browse/install smoke-test results in `docs/release-reviews/registry-publication.md`.
4. Phase gate: full frontend gate, Rust no-run/check/clippy gate, schema validation, and diff check.
<!-- END_TASK_5 -->
