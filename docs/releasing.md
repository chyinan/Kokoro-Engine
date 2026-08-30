# Releasing Kokoro Engine

This document describes the repeatable release path for the desktop app,
official character/MOD registry, and AstrBot adapter. The three surfaces are
released in stages so a missing external publication does not get presented as
an available feature.

## Release contract

1. Read the applicable phase evidence first: [activation README usability](release-reviews/activation-readme-usability.md), [first-reply usability](release-reviews/first-reply-usability.md), [registry publication](release-reviews/registry-publication.md), and [AstrBot publication](release-reviews/astrbot-publication.md).
2. Read the current version from `package.json` and
   `src-tauri/tauri.conf.json`. Choose the next version deliberately; do not
   create a tag before the release-specific notes exist.
3. Copy [`release-notes/TEMPLATE.md`](../release-notes/TEMPLATE.md) to
   `release-notes/vX.Y.Z.md`. Fill every field and keep five separate evidence
   rows: character selection, first-reply onboarding, SillyTavern import,
   registry installation, and AstrBot integration.
4. Each row needs a stable evidence asset, a direct CTA, compatibility notes,
   and exact test or manual evidence. A local test cannot stand in for a
   published registry archive, marketplace listing, or real AstrBot channel.
5. Run the local checks below. A maintainer with the required credentials and
   external participants must complete the rows marked external.

The release notes are deliberately evidence-driven. If a required asset or
external result is missing, record `Blocked` with the owner and next action;
never replace it with a guessed URL, screenshot, or metric.

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
SillyTavern import only after the activation and first-reply evidence is ready.
The release notes must link to a character-selection asset and a first-reply
clean-machine record. Include the SillyTavern JSON/PNG smoke test when import is
advertised. A missing provider, memory-model fallback, or optional Live2D asset
must have a user-facing recovery path in the compatibility notes.

### Stage B — registry release (after Phase 5)

Publish the static registry index and versioned archives only after local
checksum/compatibility tests pass and the [registry publication review](release-reviews/registry-publication.md)
contains the real JSON/archive URLs, byte counts, digests, and browse/install
smoke-test result. Keep character installation and executable MOD installation
on their separate trust paths. Do not call a local generated index a published
registry.

### Stage C — AstrBot distribution release (after Phase 6)

Publish the adapter package as a separate integration release after the
[AstrBot publication review](release-reviews/astrbot-publication.md) has a real
repository or marketplace result, tested AstrBot version, supported channel,
and screenshot/demo. The plugin must use Kokoro's authenticated webhook and
must not be described as a bidirectional real-time embodiment protocol. Local
mocked HTTP tests prove the adapter contract only; they do not prove a live
channel integration.

Stages may share a desktop version, but each advertised surface keeps its own
evidence row and status. If a later stage is not ready, publish the earlier
stage with that capability clearly marked as upcoming rather than implying that
it is installable.

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
| Candidate release notes | Blocked | Maintainer: create `release-notes/v0.3.2.md` from the template and fill all placeholders | The release-specific file is intentionally not created by this documentation task |
| Character selection asset | Blocked | Maintainer: capture and publish a stable screenshot/GIF | No release asset URL is available in the repository |
| First-reply onboarding asset and clean-machine record | Blocked | Maintainer: run five clean-machine sessions and attach the real result | `docs/release-reviews/first-reply-usability.md` remains pending external acceptance |
| SillyTavern import asset | Blocked | Maintainer: capture JSON and PNG import evidence | No release asset URL is available in the repository |
| Registry installation asset and publication | Blocked | Maintainer: publish index/archives and run browse/install/update/remove smoke test | `docs/release-reviews/registry-publication.md` keeps external URLs and smoke tests pending |
| AstrBot integration asset and publication | Blocked | Maintainer: publish plugin, test a real channel, and attach a stable demo | `docs/release-reviews/astrbot-publication.md` keeps repository, marketplace, and channel rows pending |
| Platform workflow release-notes wiring | Pending local validation | Maintainer/CI: run YAML and file-check validation on the candidate tag | The three workflows now require `release-notes/${GITHUB_REF_NAME}.md` and pass it as `body_path` |
| External credentials/participants | Blocked | Maintainer: provide GitHub/AstrBot credentials and human testers | Required external operations are not available in a local CLI dry-run |

The candidate remains blocked until the release-specific notes and all required
evidence are supplied. The record intentionally does not claim that any
external operation happened.

## After publishing

Record the release URL, exact installer asset names, and any failed platform
job in the release review. Keep the five evidence rows immutable for that
release. Before starting the next product phase, complete
`docs/release-reviews/TEMPLATE.md` (the Phase 7 Task 2 template) and record
unavailable metrics as `Unknown` or `Pending`; Kokoro does not add automatic
remote analytics to fill those gaps.
