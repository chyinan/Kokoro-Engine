# Release feedback and metrics review for Kokoro Engine `vX.Y.Z`

> Copy this file to `docs/release-reviews/vX.Y.Z-feedback.md` after the
> release is published. Keep every value tied to a source and a collection
> window. Use `Not collected`, `Unavailable`, or `Pending external` when the
> evidence does not exist; never estimate a value from impressions or logs.

## Review gate

| Field | Value |
| --- | --- |
| Release | `vX.Y.Z` |
| Release URL | `https://github.com/chyinan/Kokoro-Engine/releases/tag/vX.Y.Z` |
| Review window | `YYYY-MM-DD` to `YYYY-MM-DD` |
| Review owner | `Maintainer name or team` |
| Review date | `YYYY-MM-DD` |
| Decision before the next product phase | `Hold` / `Proceed` |
| Next phase | `Phase N: name` |

The review owner must make a `Hold` or `Proceed` decision before work on the
next product phase starts. `Proceed` requires the release-specific evidence
to be attached or linked, unresolved support failures to have an owner, and
every unavailable metric to be labeled with a reason. A missing metric is not
a reason to invent a proxy value.

## Privacy boundary

Kokoro Engine does not collect automatic remote analytics. The application
does not send release downloads, setup progress, support events, registry
downloads, submissions, repeat participation, or AstrBot usage to a Kokoro
analytics endpoint. Do not add a tracking request, identifier, cookie, or
background reporting job to fill this table.

Allowed evidence is limited to public platform counters, opt-in usability
sessions, and manually maintained support or submission records. Remove API
keys, bearer tokens, personal contact details, and private conversation data
from evidence. If a user shares a log, ask for a redacted copy and record only
the failure category needed for the review.

## Metrics

Use one row for every required metric. Values must include a unit and the
review window. Release or registry counters are aggregate asset/download
events, not unique people, unless the source explicitly says otherwise.

| Metric | Definition and counting rule | Value | Window | Source / evidence | Owner | Status |
| --- | --- | ---: | --- | --- | --- | --- |
| Release downloads | Sum of public installer asset download counters for this release. Report per asset as well as the sum; do not call it unique users. | `TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | GitHub Release asset page or API snapshot | `TBD` | `Pending` |
| Support failures | Count of support requests in which the documented recovery path did not resolve the user's release-blocking problem. Record category and linked issue/discussion; exclude general feature requests. | `TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | Redacted support issue/discussion triage | `TBD` | `Pending` |
| Usability completion | Clean-machine sessions that reach a successful first reply within ten minutes divided by sessions started. Report numerator, denominator, provider route, and recovery notes. | `TBD / TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | [`first-reply-usability.md`](first-reply-usability.md) | `TBD` | `Pending external` |
| Registry downloads | Public download count for each official character/MOD archive, plus the total. Keep archive downloads separate from installs and unique users. | `TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | [`registry-publication.md`](registry-publication.md) and public registry counters | `TBD` | `Pending external` |
| Content submissions | Number of character or MOD submissions received, accepted, rejected, and still pending during the window. Link each decision record without exposing private data. | `TBD / TBD / TBD / TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | Submission issue/discussion index | `TBD` | `Pending` |
| Repeat participation | Number of people or maintainers with at least two recorded feedback, testing, or contribution actions in the window. Use a manually deduplicated opt-in list; do not infer identity from analytics. | `TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | Redacted participation ledger | `TBD` | `Pending` |
| AstrBot adoption | Number of verified plugin installations or active channels recorded by maintainers. State which denominator is used; local mocked tests are not adoption evidence. | `TBD` | `YYYY-MM-DD` to `YYYY-MM-DD` | [`astrbot-publication.md`](astrbot-publication.md) and maintainer smoke-test records | `TBD` | `Pending external` |

### Evidence rules

For each row:

1. Record the source URL, file, issue, or session identifier and the exact
   date the value was observed.
2. Preserve the raw public counter or a redacted export so another maintainer
   can reproduce the calculation.
3. Mark the value `Unavailable` when the source is inaccessible, and `Not
   collected` when the release intentionally had no collection path. Explain
   the difference in the source/evidence cell.
4. Do not convert asset downloads into unique users, issue counts into failed
   sessions, or local test passes into external adoption.
5. Keep the review immutable after the decision. Add a dated correction note
   if a public counter changes or an evidence link is replaced.

## Feedback triage

Follow [`feedback-workflow.md`](../community/feedback-workflow.md) for intake,
redaction, categorization, owner assignment, and review cadence. Summarize
patterns here without copying private user content.

| Theme | Evidence link | Impact | Owner | Next action | Status |
| --- | --- | --- | --- | --- | --- |
| `TBD` | `TBD` | `TBD` | `TBD` | `TBD` | `Open` |

## Phase decision

### Blocking issues

- `TBD` — owner: `TBD`; evidence: `TBD`; due: `YYYY-MM-DD`.

### Decision rationale

`TBD`: explain which evidence supports the decision, which metrics remain
unavailable, and why the next phase is safe to start or must wait.

### Corrections

Append dated corrections here. Never overwrite the original value without
recording the old value, the new source, and the person who reviewed it.
