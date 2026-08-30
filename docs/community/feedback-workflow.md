# Community feedback workflow

Kokoro Engine uses public, opt-in, and manual feedback. The workflow keeps
support useful without turning the application into a telemetry client.

## Privacy boundary

Kokoro Engine does not collect automatic remote analytics. It does not report
downloads, activation, setup completion, support failures, registry usage,
submissions, repeat participation, or AstrBot usage to a remote Kokoro
endpoint. Maintainers must not add hidden tracking, fingerprinting, cookies,
or background analytics requests to this workflow.

Ask only for information needed to reproduce the issue. Contributors should
remove provider keys, bearer tokens, local paths, contact details, screenshots
with personal data, and private conversation content before posting. A
maintainer who receives sensitive material should ask for a redacted follow-up
and avoid copying it into an issue, release review, or public log.

## Intake routes

Choose the smallest route that fits the report:

| Need | Route | What to include |
| --- | --- | --- |
| Setup or runtime failure | [Support issue form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=support.yml) | Release, OS, provider route, exact symptom, safe recovery attempt, and redacted logs |
| Reproducible defect | [Bug issue form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=bug_report.yml) | Reproduction steps, expected/actual result, release, and environment |
| Product feedback | [Feature request form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=feature_request.yml) | User outcome, current workaround, and affected release/surface |
| Character contribution | [Character submission form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=character_submission.yml) | Originality, rights, license, package version, and preview evidence |
| MOD contribution | [MOD submission form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=mod_submission.yml) | Permissions, supported engine range, rights, package validation, and preview evidence |
| Discussion or tested example | [GitHub Discussions](https://github.com/chyinan/Kokoro-Engine/discussions) | A short outcome, release, and whether others may reproduce it |

Do not ask a reporter to paste a secret into an issue. For a suspected
security problem, follow the repository's private disclosure process instead
of publishing exploit details.

## Triage states

The maintainer who first reads a report records one state and an owner in the
issue or discussion. State changes should explain what evidence caused the
change.

| State | Meaning | Required action |
| --- | --- | --- |
| `New` | Report is received but not classified. | Acknowledge and remove accidental secrets. |
| `Needs information` | The report cannot be reproduced from the supplied details. | Ask one focused question and set a follow-up date. |
| `Reproducible` | The behavior is confirmed on a stated release or commit. | Assign an owner and link the smallest reproduction. |
| `Workaround` | A documented recovery path resolves the immediate problem. | Link the path and count as a support success, not a failure. |
| `Release blocker` | The documented recovery path does not restore the claimed experience. | Record a support failure, owner, affected release, and decision date. |
| `Resolved` | A fix, documentation change, or release closes the report. | Link the commit/release and ask the reporter to verify when possible. |
| `Not planned` | The request is outside the product contract or current phase. | State the scope reason; do not silently discard the report. |

Support failures are counted only for `Release blocker` reports where the
recovery path was attempted and did not resolve the problem. A duplicate keeps
its link to the canonical report and is not counted twice.

## Manual evidence ledger

At the end of each review window, the release owner copies aggregate values
into [`docs/release-reviews/TEMPLATE.md`](../release-reviews/TEMPLATE.md) (or a
versioned review copied from it). The ledger may contain issue/discussion IDs,
dates, categories, and owner names; it must not contain secrets or private
conversation text.

Record these fields for each manual entry:

| Field | Requirement |
| --- | --- |
| Release and window | Exact version and UTC/Asia/Shanghai date range. |
| Evidence ID | Public issue/discussion URL, session ID, or redacted ledger row. |
| Category | Support failure, usability session, submission, repeat participation, or AstrBot adoption. |
| Owner | Maintainer or participant responsible for the record. |
| Outcome | Pass, fail, pending, not collected, or unavailable. |
| Follow-up | Next action, due date, and linked fix or decision. |

For usability sessions, record only the session number, clean-machine status,
start/end timestamps, provider route, first-reply result, and recovery notes.
For AstrBot adoption, record a maintainer-verified installation or channel
smoke test; a mocked HTTP test proves the adapter contract, not adoption.

## Review cadence and gate

The release owner performs a first triage within three working days of a
report and closes or reassigns each open item before the review window ends.
At the end of the window, the owner:

1. Captures public installer and registry counters with observation dates.
2. Reconciles support and submission records, de-duplicates reports, and
   records unresolved failures with owners.
3. Updates all seven required metrics in the release review. Use `Not
   collected`, `Unavailable`, or `Pending external` with an explanation when
   evidence is missing.
4. Links the review from the release record and records a `Hold` or `Proceed`
   decision.

No work on the next product phase starts until a maintainer has reviewed the
release evidence and recorded that decision. The gate can proceed with an
unavailable metric only when the review explains why it was unavailable and
does not present a proxy as a measurement.

## Public summary

When sharing results, publish aggregate counts and recovery guidance, not
identifiable participant histories. Link the relevant versioned review and
state that the figures are manually recorded or public platform counters.
If a later correction changes a value, append a dated correction to the
versioned review instead of rewriting the original evidence.
