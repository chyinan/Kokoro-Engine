# Kokoro Engine `vX.Y.Z`

> Copy this file to `release-notes/vX.Y.Z.md` before creating the tag. Replace
> every placeholder, attach the five evidence assets, and keep a row `Pending`
> or `Blocked` until the named owner has supplied the evidence. This file is
> also used as the GitHub Release body by the build workflows.

## At a glance

- **Release date:** `YYYY-MM-DD`
- **Previous release:** `vX.Y.Z`
- **Compatibility:** `Kokoro Engine vX.Y.Z`; list provider, OS, registry, and
  AstrBot constraints below.
- **Privacy:** Kokoro does not collect automatic remote analytics. Feedback
  and usability evidence in this release is opt-in or manually recorded.

## What to try

Complete one observable path before publishing. Each surface has its own row,
asset, call to action, compatibility notes, and test evidence; do not reuse a
single screenshot or test result as proof for multiple rows.

| Shipped surface | Required evidence asset (stable URL or repository path) | Direct CTA | Compatibility notes | Test evidence | Status |
| --- | --- | --- | --- | --- | --- |
| Character selection | `assets/releases/vX.Y.Z/character-selection.png` or `.gif` — replace with the real asset URL | [Choose a character in the main catalog](../docs/quick-start.md#3-choose-or-import-a-character) | Built-in character packages must match the release engine version; missing optional avatar/Live2D assets must show the documented fallback | `rtk npm test -- src/ui/widgets/CharacterCatalog.test.ts src/ui/widgets/CharacterRecommendationDialog.test.ts`; attach CI run or commit | Pending |
| First-reply onboarding | `assets/releases/vX.Y.Z/first-reply-onboarding.gif` or `.mp4` — replace with the real asset URL | [Reach the first reply](../docs/quick-start.md#6-send-the-first-message) | Requires a configured OpenAI-compatible provider or local Ollama; memory model initialization must not block a basic text reply | `rtk npm test -- src/ui/widgets/OnboardingOverlay.test.tsx src/features/onboarding/onboarding-flow.test.ts`; attach clean-machine usability evidence | Pending |
| SillyTavern import | `assets/releases/vX.Y.Z/sillytavern-import.png` or `.gif` — replace with the real asset URL | [Import a SillyTavern card](../docs/quick-start.md#3-choose-or-import-a-character) | Supports the documented JSON/PNG card formats; imported cards remain user-owned and provider credentials stay local | `rtk npm test -- src/lib/character-card-parser.test.ts src/ui/widgets/CharacterManager.test.ts`; attach JSON and PNG smoke-test result | Pending |
| Registry installation | `assets/releases/vX.Y.Z/registry-installation.gif` or `.mp4` — replace with the real asset URL | [Browse and install official content](../docs/content-registry.md) | Registry entries must declare compatible engine versions, archive size, SHA-256, and trust label; third-party MOD permissions remain explicit | `rtk npm test -- scripts/build-content-registry.test.mjs`; attach clean-app browse/install/update/remove result and registry publication review | Pending |
| AstrBot integration | `assets/releases/vX.Y.Z/astrbot-integration.gif` or `.mp4` — replace with the real asset URL | [Install and configure the AstrBot adapter](../docs/integrations/astrbot.md) | Requires a published plugin compatible with the tested AstrBot version, a reachable authenticated Kokoro webhook, and a supported channel | `rtk pytest -q` from `integrations/astrbot-kokoro`; attach real-channel smoke-test result and AstrBot publication review | Pending |

### Evidence rules

For every row above, record all of the following before changing `Status` to
`Pass`:

1. An asset that shows the claimed user outcome. Do not use a mock, placeholder,
   or private URL that the release reader cannot open.
2. A direct CTA that takes the reader to the next action. Keep the CTA usable
   if the reader opens the release from a translated README or a downloaded
   installer.
3. Compatibility notes for the engine version, provider or external adapter,
   supported media, and documented fallback behavior.
4. The exact test command, CI run, manual smoke test, or usability record. A
   local test does not prove external publication, marketplace acceptance, or
   a real AstrBot channel.

If evidence is unavailable, write `Blocked — <owner> must supply <evidence>`
and keep the release out of the publish queue. Do not infer completion from
local source changes.

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
- [Release and evidence procedure](../docs/releasing.md)
