# ADR 0003: IPC Contract Check

## Status

Accepted

## Context

Rust commands are registered in `src-tauri/src/lib.rs`, while TypeScript invokes commands through `src/lib/kokoro-bridge.ts` and occasional direct `invoke()` calls.

## Decision

Add a lightweight script that checks every TypeScript command invocation is registered by the Rust Tauri handler.

## Consequences

Renamed or removed commands fail in CI before becoming runtime errors.
