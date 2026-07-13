# Phase 1: Positioning and community entry points

**Goal:** Make Kokoro understandable and testable before product changes ship.

**Execution:** Follow `execution-contract.md`. Dependencies: none. Phase evidence: `docs/release-reviews/activation-readme-usability.md`.

<!-- START_TASK_1 -->
### Task 1: Rewrite user-facing entry documentation

**Files:** `README.md`, `README_EN.md`, `README_ZH-TW.md`, `README_JA.md`, `README_KO.md`, `README_RU.md`, `docs/quick-start.md`, `docs/troubleshooting.md`, `docs/PRD.md`

1. Lead with the desktop character experience, direct release download, and three observable scenarios.
2. Separate end-user setup from source-build instructions and link troubleshooting from quick start.
3. Update the PRD to distinguish immutable templates, editable instances, and the single-application scope.
4. Preserve the user's existing QQ link text edits.
5. Verify with `rtk git diff --check` and link inspection.
6. Do not advertise three built-in characters, the registry, or the new onboarding as available in the current release until their release gate passes; Phase 1 examples must use capabilities already present or be clearly labeled as upcoming.
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Add contribution and community entry forms

**Files:** `.github/ISSUE_TEMPLATE/support.yml`, `.github/ISSUE_TEMPLATE/character_submission.yml`, `.github/ISSUE_TEMPLATE/mod_submission.yml`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/PULL_REQUEST_TEMPLATE.md`

1. Add support, character submission, and MOD submission forms with license and reproduction fields.
2. Add direct links for Discussions and security-sensitive reports.
3. Verify YAML syntax and `rtk git diff --check`.
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Record built-in character briefs and licenses

**Files:** `docs/characters/*.md`, `characters/*/LICENSE.md`

1. Define three original characters with distinct user outcomes, voice, greeting, dialogue style, cues, and asset policy.
2. Record authorship, redistribution status, and the absence or source of binary assets.
3. Document Hiyori and Haru Live2D notices without extending upstream rights.
4. Create `docs/release-reviews/activation-readme-usability.md` with maintainer owner, repository-admin prerequisite, five anonymous tester rows, the four identification questions, asset-license audit, and evidence links.
5. External acceptance: update GitHub description/homepage/topics, enable Discussions/categories, and record five-person first-viewport results in that file.
6. Phase gate: `rtk git diff --check` plus manual verification of every new local Markdown link.
<!-- END_TASK_3 -->
