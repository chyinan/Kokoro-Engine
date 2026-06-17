# ADR 0001: Frontend Runtime Settings Store

## Status

Accepted

## Context

Runtime settings are currently read and written directly through `localStorage` from `App.tsx`, `SettingsPanel.tsx`, `ChatPanel.tsx`, hooks, and services. Some settings also require custom browser events such as `kokoro-stt-settings-changed` and `kokoro-vision-settings-changed`.

## Decision

Introduce a small frontend settings module that owns localStorage keys, defaults, parsing, writing, and synchronization events. Migrate call sites incrementally.

## Consequences

Callers stop depending on raw storage key names and event names. Tests can verify persistence and event behavior through one interface.
