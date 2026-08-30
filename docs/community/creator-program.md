# Creator program

The creator program helps people contribute character content and MODs that
others can install safely. It is a small, manual review process around the
existing declarative character package and executable MOD boundaries. It does
not create creator accounts, payouts, ratings, a social feed, or automatic
remote analytics.

## What we accept

Every submission must identify its author, source, license, and permission to
redistribute the submitted files in Kokoro Engine. A package can combine
different licenses, but the package must include a complete notice for every
component and must not imply that a third-party asset is covered by the
repository MIT license.

| Contribution | Accepted content | Rights and provenance required | Boundary |
| --- | --- | --- | --- |
| Original character | Persona, greeting, example dialogue, names, and cue definitions written by the contributor or commissioned for them | Contributor's GitHub handle; source or first-publication URL; author or commissioning record; license that grants Kokoro redistribution, modification, and packaging; attribution text when required | Keep private information, undisclosed real-person impersonation, and unlicensed trademarks out of the package. Character packages remain declarative. |
| Live2D model | A model, textures, motions, physics, expressions, poses, and related data that Kokoro can load from the package | Model author and source URL; complete upstream license or permission; proof that redistribution in an application and modification for packaging are allowed; required notices and restrictions | Do not submit a model copied from a sample, game, artist, or marketplace unless the terms explicitly allow this use. No native libraries, scripts, or model paths outside the package. Missing or rejected models use Kokoro's built-in fallback. |
| Voice or audio | Creator-owned recordings, commissioned recordings, or an audio/TTS reference that the stated provider permits the user to select | Performer or rights-holder consent; source URL; license covering redistribution and application use; attribution and any synthetic-voice disclosure; provider terms for a TTS voice reference | Never include API keys, bearer tokens, custom endpoints, local model paths, or a voice imitation of an identifiable person without documented permission. A provider reference must not change the user's provider configuration. |
| Background | Original or commissioned image, or an image whose license permits redistribution with Kokoro | Photographer/artist, source URL, license or written permission, attribution, and any required releases for recognizable people or protected locations | Remove personal data and unlicensed logos. The asset must use a supported format and remain optional presentation content; an invalid image falls back to the default background. |
| MOD | Original executable content that extends Kokoro through the existing MOD interface | Contributor identity; source repository and commit; license for the code and each dependency/asset; requested permissions with a reason; security and compatibility notes; permission to redistribute the packaged build | MODs are executable and are reviewed separately from character packages. No hidden telemetry, credential collection, permission bypass, unrelated network calls, or bundled secrets. URL-installed MODs retain the untrusted-code warning. |

The package format, allowed files, path rules, and local validation commands are
defined in [Creating character packages](../creating-character-packages.md).
MOD authors must also follow [Creating MOD packages](../creating-mod-packages.md).

## Rights packet for a submission

Attach one rights packet to the issue and pull request. Use a public source
link where possible; if a permission letter is private, tell the maintainer
that it exists and provide only the minimum redacted evidence needed for review.
Never put private contact details, access tokens, or private conversation data
in a public issue.

The packet must contain:

- The contributor's GitHub handle and the role they have for each asset: author,
  commissioning party, licensee, or maintainer of the source project.
- A file-by-file inventory covering persona text, cues, avatar, Live2D files,
  voice/audio, background, fonts, and MOD dependencies.
- The original source URL or repository commit for every non-trivial component.
- The exact license identifier or permission statement, required attribution,
  modification terms, and whether redistribution in a desktop application is
  allowed.
- Written permission or a contract excerpt when the contributor is not the
  original author. For voice content, include performer consent and the allowed
  use of a synthetic or transformed voice.
- The package version, supported engine range, generated archive checksum, and
  the result of the schema, archive, and install smoke tests.

The maintainer records unresolved rights as `Pending` and does not merge or
label the entry official until the evidence is complete. A checksum proves
that bytes were not corrupted in transit; it does not prove that content is
licensed or that executable code is safe. See the [content registry trust and
submission rules](../content-registry.md) and the
[built-in asset license audit](../characters/asset-license-audit.md).

## Submission path

1. Copy the character scaffold and replace every placeholder. Run the JSON,
   manifest, archive, and package smoke tests described in
   [Creating character packages](../creating-character-packages.md).
2. Open the [character submission form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=character_submission.yml)
   or [MOD submission form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=mod_submission.yml).
   Include the rights packet, compatibility range, preview references, and
   requested capabilities or permissions.
3. Keep source files, generated archives, and registry metadata in a focused
   pull request. Do not hand-edit generated checksums or archive sizes.
4. The maintainer validates the package boundary, rights, license notices,
   compatibility, permissions, and a disposable-profile install/remove smoke
   test. A local green test is not evidence of external publication.
5. After publication, record the real URL, commit or release, date, and result
   in [the external operations checklist](external-operations-checklist.md).

An accepted character package remains declarative and may recommend memory,
vision, MCP, or bot capabilities, but a recommendation never grants consent.
The application asks the user before enabling a sensitive capability. An
accepted MOD follows the existing permission review and trust warning.

## Manual community actions

These actions require a maintainer account, a creator's consent, or human
participants. They are tracked in the [external operations checklist](external-operations-checklist.md),
not inferred from local files:

- Configure the relevant [GitHub Discussions](https://github.com/chyinan/Kokoro-Engine/discussions)
  categories and publish one contribution prompt with the submission-form and
  rights-packet links.
- Publish a short, stable demo showing character selection, a first reply, or a
  safe MOD installation. Redact provider keys, bearer tokens, local paths, and
  private conversation content before publishing.
- Invite a small set of creators to submit original character cards, Live2D,
  voice, background, and MOD work. Record opt-in, the invited creator handle,
  contact route, date, and response without exposing private contact details.
- Publish an approved package to the static registry or submit the AstrBot
  adapter to its marketplace only after the relevant external owner completes
  the required review. Kokoro does not operate a creator marketplace backend.
- Run consented clean-machine usability sessions. Record only the session
  number, environment, provider route, first-reply result, timestamps, and
  recovery notes in the applicable review; do not add hidden telemetry.

For support and follow-up, use the [community feedback workflow](feedback-workflow.md).
For release sequencing and approval, use [releasing Kokoro](../releasing.md).

## Review and removal

The maintainer can request clearer provenance, limit an asset to local use, or
reject a submission when redistribution rights, safety boundaries, or package
validation are incomplete. A creator can request correction or removal through
the repository's issue or discussion channels. The maintainer records the
decision, affected package version, evidence, and any replacement or removal
commit; removing a package must not delete a user's character instance,
conversation, memory, or global Live2D model.
