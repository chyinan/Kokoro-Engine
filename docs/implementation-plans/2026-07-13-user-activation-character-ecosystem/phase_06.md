# Phase 6: AstrBot distribution adapter

**Goal:** Deliver a publishable AstrBot plugin that forwards supported messages to Kokoro's webhook.

**Execution:** Follow `execution-contract.md`. Dependencies: existing Kokoro webhook and Phase 3 activation. Research baseline: AstrBot official repository commit `d0e5e68c55804bb99b40ab946dfb9d2b38fdbe1f` on 2026-07-13; plugin metadata targets AstrBot `>=4.5.0` and uses only documented `Star`, `AstrBotConfig`, `AstrMessageEvent`, and message-component APIs.

<!-- START_TASK_1 -->
### Task 1: Correct and document the Kokoro webhook contract

**Files:** `src-tauri/src/commands/bot.rs`, `docs/API specification.md`, Rust tests

1. Write tests for request character precedence, Bearer authentication, text/media parsing, conversation mapping, and JSON errors.
2. Resolve character as request override, configured webhook default, then active character.
3. Ensure private/group mapping identifies a character-owned conversation before persistence.
4. RED/GREEN command: `rtk cargo test --manifest-path src-tauri/Cargo.toml commands::bot::tests -- --nocapture`.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Build the AstrBot plugin package

**Files:** `integrations/astrbot-kokoro/main.py`, `integrations/astrbot-kokoro/metadata.yaml`, `integrations/astrbot-kokoro/_conf_schema.json`, `integrations/astrbot-kokoro/requirements-dev.txt`, `integrations/astrbot-kokoro/LICENSE`, `integrations/astrbot-kokoro/tests/test_plugin.py`, `integrations/astrbot-kokoro/tests/fakes.py`

1. Implement AstrBot's current `Star`/`AstrBotConfig` plugin API and all-message listener.
2. Map private/group sessions, text, images, and audio to the Kokoro webhook; return text/image/audio replies supported by the contract.
3. Add mocked HTTP tests for success, 401, connection failure, malformed response, and supported media.
4. RED/GREEN command from the integration directory: `pytest -q` using the AstrBot commit/API baseline above and the dev requirements file.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Add setup and release documentation

**Files:** `integrations/astrbot-kokoro/README.md`, `docs/integrations/astrbot.md`

1. Document loopback/container endpoints, Bearer tokens, character selection, conversation strategy, media toggles, and curl smoke tests.
2. External acceptance: create/sync a separate plugin repository, publish to the AstrBot marketplace, add screenshots/demo, and run a real channel smoke test.
3. Record repository URL, marketplace URL, AstrBot version, tested channel, screenshots/demo, and smoke-test result in `docs/release-reviews/astrbot-publication.md`.
4. Phase gate: Rust contract tests/checks, Python tests, docs link check, and diff check.
<!-- END_TASK_3 -->
