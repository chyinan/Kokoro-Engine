# ADR 0002: Chat Turn State Machine

## Status

Accepted

## Context

`ChatPanel.tsx` currently understands backend event ordering, streaming reveal, cancellation, tool traces, approval states, vision context, Telegram sync, proactive triggers, and TTS auto-play.

## Decision

Extract pure Chat Turn state transitions into a frontend module. React should wire events and render snapshots, not encode the state machine inline.

## Consequences

Most streaming and tool trace regressions can be tested without mounting React or running Tauri.
