# AstrBot Kokoro Adapter

Last verified: 2026-08-31

## Purpose

Bridge supported AstrBot messages to Kokoro's character-aware webhook without collecting remote telemetry or exposing provider secrets.

## Contracts

- **Exposes**: AstrBot `Star` all-message listener, configurable webhook endpoint/token/character/session strategy, and text/image/audio reply components.
- **Guarantees**: Bearer auth is forwarded, private/group conversations remain character-scoped, AstrBot audio conversion is sent as WAV, image MIME is inferred from available path/URL/data, and malformed responses become actionable errors.
- **Expects**: AstrBot >=4.5 documented APIs, a running Kokoro webhook, and explicit media toggles.

## Dependencies

- **Uses**: `AstrBotConfig`, `AstrMessageEvent`, `Plain`, `Image`, `Record`, `ALL`, and local HTTP client.
- **Used by**: AstrBot runtime and setup/release documentation.
- **Boundary**: no secret logging; marketplace/channel publication and real smoke tests are manual external operations.

## Invariants

- Character selection follows request override → configured webhook default → active character.
- Session strategy determines private/group identity mapping and never mixes character histories.
- External repository, marketplace, screenshot, and channel evidence stays Pending until actually performed.
