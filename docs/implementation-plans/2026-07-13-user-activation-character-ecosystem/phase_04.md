# Phase 4: First-reply onboarding

**Goal:** Reach one successful response through language, character, provider, test, and chat.

**Execution:** Follow `execution-contract.md`. Dependencies: Phase 3 committed activation service and main catalog.

<!-- START_TASK_1 -->
### Task 1: Create a resumable onboarding state machine

**Files:** `src/features/onboarding/onboarding-flow.ts`, `src/features/onboarding/onboarding-flow.test.ts`, `src/App.tsx`

1. Write failing tests for continuation, retry, dismissal, resume, and completion only after a successful reply.
2. Persist a serializable draft for `language -> character -> provider -> connection-test -> chat`.
3. Preserve configured values on retry and dismissal.
4. RED/GREEN command: `rtk npm test -- src/features/onboarding/onboarding-flow.test.ts`.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add focused provider setup

**Files:** `src/features/onboarding/provider-setup.ts`, `src/features/onboarding/provider-setup.test.ts`, `src/ui/widgets/onboarding/ProviderSetupStep.tsx`, `src/ui/widgets/settings/ApiTab.tsx`

1. Extract and test OpenAI-compatible presets, Ollama discovery, normalization, save, and connection test from the advanced API tab.
2. Present only endpoint/preset, key where required, model, discovery, and test result in onboarding.
3. Keep advanced generation/context options in Settings.
4. RED/GREEN command: `rtk npm test -- src/features/onboarding/provider-setup.test.ts`.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Rebuild onboarding as an outcome-led workflow

**Files:** `src/ui/widgets/OnboardingOverlay.tsx`, `src/ui/widgets/OnboardingOverlay.test.tsx`, `src/ui/widgets/CharacterCatalog.tsx`, `src/App.tsx`, `package.json`, `src/ui/locales/en.json`, `src/ui/locales/zh.json`, `src/ui/locales/zh-TW.json`, `src/ui/locales/ja.json`, `src/ui/locales/ko.json`, `src/ui/locales/ru.json`

1. Render actual language, character, provider, test, and chat steps instead of a settings spotlight tour.
2. Provide localized actionable errors and retry controls.
3. Complete only from the first successful chat turn callback.
4. Add the existing project-compatible DOM test environment and test language selection, character selection, connection retry, dismissal/resume, and first-reply completion.
5. RED/GREEN command: `rtk npm test -- src/ui/widgets/OnboardingOverlay.test.tsx`.
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: Make memory initialization non-blocking and group settings

**Files:** `src/lib/memory-model-gate.ts`, `src/lib/memory-model-gate.test.ts`, `src/ui/widgets/ChatPanel.tsx`, `src/ui/widgets/MemoryModelDownloadDialog.tsx`, `src/ui/widgets/SettingsPanel.tsx`, `src/ui/widgets/settings/settings-groups.ts`, `src/ui/widgets/settings/settings-groups.test.ts`, `src-tauri/src/ai/memory_embedding_model.rs`, `src-tauri/src/ai/memory.rs`, `src-tauri/src/ai/memory_nonblocking_tests.rs`

1. Write tests proving missing/downloading/error memory states still allow basic chat.
2. Start memory download in the background, show status and retry, and bypass semantic retrieval until ready.
3. On the backend, make embedding initialization/retrieval return a typed unavailable state that chat treats as an empty semantic result; never synchronously download or fail the base LLM turn.
4. Add basic/advanced settings grouping without removing any tab.
5. RED/GREEN commands: `rtk npm test -- src/lib/memory-model-gate.test.ts src/ui/widgets/settings/settings-groups.test.ts` and `rtk cargo test --manifest-path src-tauri/Cargo.toml ai::memory_nonblocking_tests -- --nocapture`.
6. Create `docs/release-reviews/first-reply-usability.md` with five clean-machine sessions, start/end timestamps, provider route, recovery notes, and pass/fail; external acceptance requires at least 4/5 successful replies within ten minutes.
7. Phase gate: full frontend tests/build and Rust check/no-run for backend memory changes.
<!-- END_TASK_4 -->
