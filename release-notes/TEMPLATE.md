# Kokoro Engine `vX.Y.Z`

> Copy this file to `release-notes/vX.Y.Z.md` before creating the tag. Replace
> every placeholder and add supporting assets or external results when
> available. This file is also used as the GitHub Release body by the build
> workflows.

## At a glance

- **Release date:** `YYYY-MM-DD`
- **Previous release:** `vX.Y.Z`
- **Compatibility:** `Kokoro Engine vX.Y.Z`; list provider, OS, registry, and
  AstrBot constraints below.
- **Privacy:** Kokoro does not collect automatic remote analytics. Feedback
  and usability evidence in this release is opt-in or manually recorded.

## What to try

Describe the observable paths shipped in this release. Add an asset, call to
action, compatibility note, and test evidence when available; do not reuse a
single screenshot or test result as proof for unrelated rows.

| Shipped surface | Supporting asset (optional) | Direct CTA | Compatibility notes | Test evidence or manual note | Status |
| --- | --- | --- | --- | --- | --- |
| Character selection | Optional screenshot/GIF | [Choose a character in the main catalog](../docs/quick-start.md#3-choose-or-import-a-character) | Built-in character packages must match the release engine version; missing optional avatar/Live2D assets must show the documented fallback | `rtk npm test -- src/ui/widgets/CharacterCatalog.test.ts src/ui/widgets/CharacterRecommendationDialog.test.ts` | Pending |
| First-reply onboarding | Optional GIF/MP4 | [Reach the first reply](../docs/quick-start.md#6-send-the-first-message) | Requires a configured OpenAI-compatible provider or local Ollama; memory model initialization must not block a basic text reply | `rtk npm test -- src/ui/widgets/OnboardingOverlay.test.tsx src/features/onboarding/onboarding-flow.test.ts` | Pending |
| SillyTavern import | Optional screenshot/GIF | [Import a SillyTavern card](../docs/quick-start.md#3-choose-or-import-a-character) | Supports the documented JSON/PNG card formats; imported cards remain user-owned and provider credentials stay local | `rtk npm test -- src/lib/character-card-parser.test.ts src/ui/widgets/CharacterManager.test.ts` | Pending |
| Registry installation | Optional GIF/MP4 | [Browse and install official content](../docs/content-registry.md) | Registry entries must declare compatible engine versions, archive size, SHA-256, and trust label; third-party MOD permissions remain explicit | `rtk npm test -- scripts/build-content-registry.test.mjs` | Pending |
| AstrBot integration | Optional GIF/MP4 | [Install and configure the AstrBot adapter](../docs/integrations/astrbot.md) | Requires a compatible AstrBot version, a reachable authenticated Kokoro webhook, and a supported channel | `rtk pytest -q` from `integrations/astrbot-kokoro` | Pending |

## Activation and capability notes

- Character activation applies persona, presentation, voice, language, and cue
  settings together and rolls back on a required failure.
- Vision, memory, MCP servers, and bot access are recommendations until the
  user explicitly confirms them. A recommendation is not permission.
- Existing conversations and memories remain isolated by character instance.

## Upgrade and rollback notes

- Provider credentials and custom endpoints remain application-level settings;
  character packages never contain them.
- Data-only backups omit binary character resources by default. Use the
  resource-inclusive option when migrating package-owned assets.
- If the release is rolled back, follow the compatibility instructions in
  `docs/releasing.md` and remove or disable settings unsupported by the older
  version before downgrading.

## Full changes

- [Compare this release with the previous tag](https://github.com/chyinan/Kokoro-Engine/compare/vX.Y.Z...vX.Y.Z)
- [Release procedure](../docs/releasing.md)
