# AstrBot publication review

This file separates repository-local validation from the external acceptance
needed to publish the AstrBot adapter. Local files and mocked HTTP tests do not
prove that a separate plugin repository, the AstrBot marketplace, or a real
channel is available. Keep external rows `Pending` until a maintainer attaches
the stated evidence.

## Integration identity

| Item | Value |
| --- | --- |
| Source repository URL | `https://github.com/chyinan/Kokoro-Engine` |
| In-tree plugin URL | `https://github.com/chyinan/Kokoro-Engine/tree/main/integrations/astrbot-kokoro` |
| Package version | `0.1.0` (from `integrations/astrbot-kokoro/metadata.yaml`) |
| Target AstrBot version | `>=4.5.0` |
| Review date | 2026-08-30 |
| External publication owner | Kokoro Engine maintainer; credentials and a running AstrBot instance required |

The in-tree URL is the source package, not a claim that a separate plugin
repository has been created or synchronized.

## External acceptance evidence

| Check | Status | Owner | Date | URL/result and blocker |
| --- | --- | --- | --- | --- |
| Separate plugin repository URL | Pending | Kokoro Engine maintainer | Pending | No separate repository operation was performed in this task. Record the repository URL, synced commit, and package version after publication. |
| AstrBot marketplace URL | Pending | Kokoro Engine maintainer | Pending | No marketplace submission was performed. Record the marketplace listing URL, accepted version, and submission result. |
| AstrBot version tested | Pending | Kokoro Engine maintainer | Pending | Run the published plugin against the target AstrBot release and record the exact version. |
| Tested channel | Pending | Kokoro Engine maintainer | Pending | Select one supported AstrBot channel and record its adapter/channel name and date after a maintainer-run smoke test. |
| Screenshots and demo | Pending | Kokoro Engine maintainer | Pending | Add stable screenshot/demo URLs showing setup, character selection, and one channel reply. No media was captured in this local task. |
| Real channel smoke test | Pending | Kokoro Engine maintainer | Pending | Requires published plugin, a running AstrBot instance, valid Kokoro LLM/TTS/STT settings as applicable, and channel credentials. Record request time, character ID, conversation strategy, response result, and evidence URL. |

These statuses are intentionally not inferred from local code or pytest. The
external owner must add evidence before changing a row to `Pass` or `Fail`.

## Local validation evidence

| Check | Status | Owner | Date | Evidence |
| --- | --- | --- | --- | --- |
| Plugin metadata/config review | Pass | Phase 6 Task 3 | 2026-08-30 | `metadata.yaml` targets AstrBot `>=4.5.0`; `_conf_schema.json` exposes endpoint, token, character, conversation, and text/image/audio toggles. |
| Webhook contract review | Pass | Phase 6 Task 3 | 2026-08-30 | [`docs/API specification.md`](../API%20specification.md#authenticated-generic-webhook) and the plugin docs match the Rust handler's request fields, precedence, conversation keys, replies, and `400`/`401`/`404`/`500` outcomes. |
| Plugin mocked HTTP tests | Pass | Phase 6 Task 3 | 2026-08-30 | `rtk pytest -q` from `integrations/astrbot-kokoro`: 6 tests passed. |
| Documentation link check | Pass | Phase 6 Task 3 | 2026-08-30 | Local Markdown target check covered the three Task 3 documents; all relative targets exist. |
| Real Kokoro endpoint smoke test | Pending | Kokoro Engine maintainer | Pending | Local docs provide curl commands, but no running LLM/STT/TTS service was exercised in this task. |

## Acceptance checklist for the maintainer

1. Sync `integrations/astrbot-kokoro` to a separate plugin repository. Record
   its URL, commit, metadata version, and license.
2. Install that commit in a clean AstrBot `>=4.5.0` instance. Record the exact
   AstrBot version and installation result.
3. Configure Kokoro's Webhook runtime with a Bearer token. Keep the token out
   of screenshots, logs, and this file.
4. Run the text and authentication curl checks in the [AstrBot integration
   guide](../integrations/astrbot.md), then test the published plugin through
   one real channel.
5. Capture screenshots or a short demo covering plugin configuration,
   character selection, and a successful reply. Link them above.
6. Submit the plugin to the AstrBot marketplace. Record the listing URL and
   accepted version only after the marketplace confirms publication.
7. Record the real channel result, date, character ID, conversation strategy,
   media toggles, and any failure/recovery notes. A local mocked test is not a
   substitute for this row.

## Related documentation

- [AstrBot integration guide](../integrations/astrbot.md)
- [Plugin README](../../integrations/astrbot-kokoro/README.md)
- [Kokoro webhook API](../API%20specification.md#authenticated-generic-webhook)
- [General troubleshooting](../troubleshooting.md)
