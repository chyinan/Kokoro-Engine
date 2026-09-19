# Releasing Kokoro Engine

This document describes the repeatable release path for the desktop app,
official character/MOD registry, and AstrBot adapter. The three surfaces are
released in stages so a missing external publication does not get presented as
an available feature.

## Release contract

1. Review the applicable phase notes when they exist: [activation README usability](release-reviews/activation-readme-usability.md), [first-reply usability](release-reviews/first-reply-usability.md), [registry publication](release-reviews/registry-publication.md), and [AstrBot publication](release-reviews/astrbot-publication.md). These records are supplementary and are not a gate for publishing the desktop release.
2. Read the current version from `package.json` and
   `src-tauri/tauri.conf.json`. Choose the next version deliberately; do not
   create a tag before the release-specific notes exist.
3. Copy [`release-notes/TEMPLATE.md`](../release-notes/TEMPLATE.md) to
   `release-notes/vX.Y.Z.md` and fill the release details and shipped-surface
   notes. Add supporting assets or external results when they are available.
4. Record compatibility notes and exact local/manual checks for advertised
   surfaces. Keep local verification distinct from external publication claims;
   missing optional evidence does not block the desktop release.
5. Run the local checks below. External credentials or participants are only
   needed when the release also updates an external surface.

The release notes should remain evidence-aware. If supporting evidence is not
available, record `Pending`, `Not collected`, or a concise next action; never
replace it with a guessed URL, screenshot, or metric.

## What the build workflows do

All three platform workflows run on `v*` tags and upload their installer
artifacts to the matching GitHub Release. Before uploading, each workflow
checks that `release-notes/${GITHUB_REF_NAME}.md` exists. The release action
then uses that same file as `body_path`, so a tag without matching notes fails
early instead of publishing an undocumented build.

The macOS workflow checks out the repository again in its final aggregation job
because that job receives downloaded artifacts in a fresh runner. Windows uses
PowerShell for its file check; Linux and macOS use POSIX shell syntax.

To test the file-selection behavior without publishing anything, run the
workflow's release-notes check locally with the candidate tag name and inspect
the generated action inputs. Do not upload a release from a dry-run.

## Staged release sequence

### Stage A — activation release (after Phase 4)

Ship the main-surface character selector, first-reply onboarding, and
SillyTavern import when the implementation and local checks are ready. Include
available character-selection, first-reply, and JSON/PNG smoke-test evidence
when those surfaces are advertised. A missing provider, memory-model fallback,
or optional Live2D asset must have a user-facing recovery path in the
compatibility notes.

### Stage B — registry release (after Phase 5)

Publish the static registry index and versioned archives after local
checksum/compatibility tests pass. Record the [registry publication review](release-reviews/registry-publication.md)
separately when external URLs, byte counts, digests, and browse/install
smoke-test results become available. Keep character installation and
executable MOD installation on their separate trust paths. Do not call a local
generated index a published registry.

### Stage C — AstrBot distribution release (after Phase 6)

Publish the adapter package as a separate integration release when its local
contract checks are ready. Record the [AstrBot publication review](release-reviews/astrbot-publication.md)
with the repository or marketplace result, tested AstrBot version, supported
channel, and screenshot/demo when available. The plugin must use Kokoro's
authenticated webhook and must not be described as a bidirectional real-time
embodiment protocol. Local mocked HTTP tests prove the adapter contract only;
they do not prove a live channel integration.

Stages may share a desktop version, but each advertised surface should keep its
own notes and status. If a later stage is not ready, publish the earlier stage
with that capability clearly marked as upcoming rather than implying that it is
installable.

## Local validation checklist

Run from the repository root, prefixing commands with `rtk`:

```text
rtk npm test
rtk npm run build
rtk npm run check:ipc
rtk cargo test --manifest-path src-tauri/Cargo.toml --no-run
rtk cargo check --manifest-path src-tauri/Cargo.toml
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings
rtk git diff --check
```

For the registry and AstrBot stages, also run the exact commands named in the
release-notes rows. The Rust tests may compile but fail to execute locally when
the existing `onnxruntime.dll` reports `STATUS_ENTRYPOINT_NOT_FOUND`; record
that environment blocker rather than weakening the gate.

## Dry-run record for the next version

The following dry-run was performed against the current app version `0.3.1`
with the next candidate tag `v0.3.2`. It is a checklist rehearsal, not a
release approval. No tag, GitHub Release, registry upload, marketplace
submission, or external demo was created.

| Check | Result | Owner / next action | Evidence or blocker |
| --- | --- | --- | --- |
| Candidate release notes | Pending | Maintainer: create `release-notes/v0.3.2.md` from the template and fill the release details | Supporting assets and external records are optional and can be added later |
| Advertised-surface evidence | Not collected | Maintainer: add available screenshots, smoke tests, or external records when useful | Missing optional evidence does not block the desktop release |
| Platform workflow release-notes wiring | Pending local validation | Maintainer/CI: run YAML and file-check validation on the candidate tag | The three workflows now require `release-notes/${GITHUB_REF_NAME}.md` and pass it as `body_path` |
| External credentials/participants | Not required for desktop release | Maintainer: provide them only when updating an external surface | Local desktop publication can proceed without external participants |

The candidate is ready once the release-specific notes and local validation are
complete. The record intentionally does not claim that any external operation
happened unless it has been performed.

## After publishing

Record the release URL, exact installer asset names, and any failed platform
job in the release review. Add optional evidence and feedback as it becomes
available. Before starting the next product phase, copy
`docs/release-reviews/TEMPLATE.md` to
`docs/release-reviews/vX.Y.Z-feedback.md` (the Phase 7 Task 2 template), then
record unavailable metrics as `Not collected`, `Unavailable`, or `Pending
external` according to the evidence state. Do not overwrite the template;
Kokoro does not add automatic remote analytics to fill those gaps.
