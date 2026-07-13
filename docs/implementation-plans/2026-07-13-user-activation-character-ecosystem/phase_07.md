# Phase 7: Release and feedback loop

**Goal:** Make activation, registry, and adapter releases repeatable and evidence-driven without telemetry.

**Execution:** Follow `execution-contract.md`. Dependencies: publish after Phase 4, then repeat after Phases 5 and 6.

<!-- START_TASK_1 -->
### Task 1: Add release templates and workflow checks

**Files:** `release-notes/TEMPLATE.md`, `docs/releasing.md`, `.github/workflows/build-windows.yml`, `.github/workflows/build-linux.yml`, `.github/workflows/build-macos.yml`

1. Require separate evidence rows/assets for character selection, first-reply onboarding, SillyTavern import, registry installation, and AstrBot integration, with a direct CTA, compatibility notes, and test evidence for each shipped surface.
2. Wire release workflows to the matching release-notes file where the GitHub action supports it.
3. Document staged activation, registry, and AstrBot releases.
4. Operational validation: dry-run the release checklist against the next version and record missing assets as blockers.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add feedback and metrics review templates

**Files:** `docs/release-reviews/TEMPLATE.md`, `docs/community/feedback-workflow.md`

1. Record release downloads, support failures, usability completion, registry downloads, submissions, repeat participation, and AstrBot adoption.
2. State that no automatic remote analytics are collected.
3. Require review before the next product phase.
4. Operational validation: create one sample review from existing v0.3.1 public data and mark unavailable metrics explicitly rather than inventing values.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Add creator outreach and external operations checklist

**Files:** `docs/community/creator-program.md`, `docs/community/external-operations-checklist.md`

1. Define accepted original character, Live2D, voice, background, and MOD contributions with explicit rights.
2. List manual GitHub Discussions, demo publishing, outreach, marketplace, and usability-test actions.
3. Verify all documentation links and `rtk git diff --check`.
4. External evidence file: `docs/community/external-operations-checklist.md` must name the maintainer/creator owner, required permissions, completion date, evidence URL/result, and pass/fail for each action.
5. Phase gate: documentation link check, workflow syntax validation, and diff check.
<!-- END_TASK_3 -->
