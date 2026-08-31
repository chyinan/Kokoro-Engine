# Onboarding Flow

Last verified: 2026-08-31

## Purpose

Keep first-reply setup resumable and outcome-led while preserving the single character activation owner.

## Contracts

- **Exposes**: serializable `OnboardingDraft`, reducer events, provider setup core/shell, and onboarding turn correlation helpers.
- **Guarantees**: language → character → provider → connection-test → chat ordering; dismiss/retry preserves configured values; completion requires a successful first reply.
- **Expects**: App persistence at `kokoro_onboarding_draft`, activation/service callbacks, and correlated chat turn IDs.

## Dependencies

- **Uses**: typed Kokoro bridge, character activation service, provider setup shell, localized UI.
- **Used by**: `App.tsx`, `OnboardingOverlay`, `ProviderSetupStep`.
- **Boundary**: provider secrets remain app-wide configuration; onboarding must not write character runtime subsets directly.

## Invariants

- Empty or malformed persisted selections are treated as unconfigured.
- A dismissed or canceled chat request cannot complete onboarding or leak into background ChatPanel state.
- Provider discovery/save/test failures surface localized retryable errors.

## Gotchas

- AstrBot and webhook media contracts are outside this domain; only the provider setup fields needed for first reply belong here.
