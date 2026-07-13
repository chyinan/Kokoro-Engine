# Execution contract

**Architecture:** Tauri v2 commands and SQLite are the persistence boundary; React consumes typed wrappers from `src/lib/kokoro-bridge.ts`. Pure validation, normalization, merge, state-machine, and precedence rules stay in Functional Core files. Filesystem, database, HTTP, localStorage, and Tauri orchestration stay in Imperative Shell files.

**Tech stack:** Rust 2021, Tauri 2, sqlx 0.8/SQLite, zip 2, reqwest 0.12, sha2 0.10, React 19, TypeScript 5.8, Vitest 4.

**Codebase verified:** 2026-07-13 against branch `main`. Frontend baseline: 84 tests pass and `npm run build` succeeds. Rust tests compile but the local runner exits with `STATUS_ENTRYPOINT_NOT_FOUND` because the existing `onnxruntime.dll` is incompatible; use `cargo test --no-run`, `cargo check`, and `clippy` as mandatory local gates until that environment issue is corrected.

**Per-task TDD:** Add the named test first; run the exact targeted command and confirm failure for the missing behavior; implement the minimum production change; rerun the target and broader phase tests. Configuration/data/document-only tasks use operational validation instead of a fabricated unit test.

**Phase gate:** Run `rtk npm test`, `rtk npm run build`, `rtk cargo test --manifest-path src-tauri/Cargo.toml --no-run`, `rtk cargo check --manifest-path src-tauri/Cargo.toml`, `rtk cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`, and `rtk git diff --check` when the phase touches the relevant surface. Do not proceed with a failing gate except for the documented ONNX runtime execution blocker.

**External operations:** Repository administration, Discussions configuration, usability sessions, separate-repository publication, marketplace submission, screenshots, demos, and outreach require maintainer credentials or human participants. Each external item must record owner, date, evidence URL/result, and pass/fail in the evidence file named by its phase; local source changes are not allowed to claim those actions completed.
