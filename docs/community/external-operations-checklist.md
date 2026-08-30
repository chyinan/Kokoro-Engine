# External operations checklist

This ledger records the manual work needed to turn the creator program into a
public release and feedback loop. Local source changes, mocked tests, and a
passing documentation check do not prove that an external action happened.
Keep each row `Pending` until the named owner supplies the real result. Never
invent a URL, participant, publication, screenshot, or usability outcome.

## Review metadata

| Field | Value |
| --- | --- |
| Target release/window | v0.3.2 creator-program dry run; set the actual release and UTC date range before execution |
| Maintainer owner | Kokoro Engine maintainer (GitHub: [chyinan](https://github.com/chyinan)) |
| Creator-program owner | Kokoro Engine maintainer (chyinan); name the invited creator in the row before outreach |
| Checklist opened | 2026-08-31 |
| Status vocabulary | `Pass`, `Fail`, or `Pending`; `Pending` requires a blocker or missing evidence |
| Privacy boundary | Manual, opt-in records only; no automatic remote analytics |

## External action ledger

| Action | Maintainer/creator owner | Required permissions or participants | Completion date | Evidence URL/result | Pass/fail |
| --- | --- | --- | --- | --- | --- |
| Configure GitHub Discussions categories and publish a creator call | Kokoro Engine maintainer (chyinan) | Repository administration permission; permission to publish under the Kokoro organization; maintainer review of submission and rights links | Pending | Pending — no repository-administration operation was performed locally. Add the category names, published thread URL, commit or settings result, and UTC date. | Pending |
| Publish one character-selection or first-reply demo | Kokoro Engine maintainer (chyinan); contributing creator if creator content is shown | Release/demo account, a tested Kokoro build, provider access, and creator permission for every displayed asset or conversation | Pending | Pending — no screenshot or video was captured. Add a stable public URL, release/build, character ID, redaction check, and publication result. | Pending |
| Invite creators for original character, Live2D, voice, background, and MOD submissions | Kokoro Engine maintainer (chyinan); invited creator owner must be named before each invitation | Maintainer outreach account; creator opt-in; documented source, license, performer consent, and redistribution permission for each asset | Pending | Pending — no outreach was performed. Add the creator's public handle, invitation URL or public thread, date, response, and consent result; keep private contact details out of this file. | Pending |
| Review and publish an approved character or MOD package to the static registry | Kokoro Engine maintainer (chyinan); submitting creator (GitHub handle from the submission issue) | Registry repository write permission; creator's rights packet; package validation, checksum review, compatibility review, and publication approval | Pending | Pending — no package was externally published. Add the submission issue/PR URL, published index/archive URL, checksum, accepted version, and browse/install smoke-test result. | Pending |
| Submit the AstrBot adapter to the AstrBot marketplace | Kokoro Engine maintainer (chyinan) | AstrBot marketplace account or maintainer approval; separate plugin repository; published plugin metadata, license, screenshots, and supported AstrBot version | Pending | Pending — no marketplace submission was performed. Add the marketplace listing URL, submission ID/result, accepted version, and publication date. See [AstrBot publication review](../release-reviews/astrbot-publication.md). | Pending |
| Run a real creator or user usability session for character selection and first reply | Kokoro Engine maintainer (chyinan); volunteer participant (name or anonymous session ID recorded privately) | Consent from each participant; clean machine/profile; provider access; session facilitator; permission to publish only aggregate results | Pending | Pending — no human session was run. Add a redacted session ID, clean-machine status, start/end timestamps, provider route, first-reply result, recovery notes, and review URL. Do not publish conversation text or personal data. | Pending |

`Fail` means the action was attempted and the acceptance condition was not
met. Record the failure reason, owner, recovery action, and follow-up date in
the evidence result before the next release decision. `Pending` means the
action has not been proven; it is not a pass and must remain a release blocker
when the phase requires it.

## Local prerequisites

These checks can be performed in the repository, but they do not change the
external rows above.

| Check | Owner | Completion date | Evidence URL/result | Pass/fail |
| --- | --- | --- | --- | --- |
| Creator rights and package boundary documentation reviewed | Kokoro Engine maintainer (chyinan) | 2026-08-31 | [Creator program](creator-program.md), [Creating character packages](../creating-character-packages.md), and [Content registry](../content-registry.md) describe provenance, license, declarative limits, and explicit consent boundaries. | Pass |
| External checklist contains an owner, permission requirement, date, evidence result, and status for every manual action | Kokoro Engine maintainer (chyinan) | 2026-08-31 | This file; all unperformed external actions remain `Pending` with a blocker. | Pass |
| Documentation links resolve from the repository checkout | Kokoro Engine maintainer (chyinan) | 2026-08-31 | `rtk node -e` link check: 16 relative targets in the two Task 3 documents; all resolve. | Pass |
| `rtk git diff --check` is clean | Kokoro Engine maintainer (chyinan) | 2026-08-31 | `rtk git diff --check`: exit 0 with no whitespace errors. | Pass |

## Completion rule

Before calling the creator-program work complete, the maintainer must replace
each applicable external `Pending` row with a real `Pass` or `Fail`, or record
why the action is intentionally deferred in the release decision. A local
commit can document the operations and their blockers, but it cannot claim
that a Discussion, demo, outreach campaign, registry/marketplace publication,
or human usability session occurred.

Use the [community feedback workflow](feedback-workflow.md) for redacted
support and participation records, the [release review template](../release-reviews/TEMPLATE.md)
for aggregate metrics, and the [AstrBot publication review](../release-reviews/astrbot-publication.md)
for plugin-specific external evidence.
