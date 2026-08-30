# Registry publication review

This file records release evidence for the static registry. It separates local
generation checks from external GitHub publication and application smoke tests.
Do not change a `Pending` status until a maintainer has the stated evidence.

## Registry identity

| Item | Value |
| --- | --- |
| Official JSON URL | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json` |
| Official registry identity | `github.com/chyinan/Kokoro-Engine/registry-v1` |
| Source index | `registry/v1/index.json` at commit `148af89` |
| Review date | 2026-08-30 |
| External publication owner | Kokoro Engine maintainer (GitHub credentials required) |

The following values are copied from the committed `registry/v1/index.json`.
They are exact metadata values, not a claim that the raw URLs have already
been uploaded or fetched successfully.

## Official package metadata

| Type | ID/version | Official archive URL | Archive size | SHA-256 |
| --- | --- | --- | ---: | --- |
| Character | `kokoro` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/kokoro-1.0.0.zip` | 2585 | `7392c1266db926138072dab774bd33fccb59279c0d8c8a4f6b7f1336838159e8` |
| Character | `pico` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/pico-1.0.0.zip` | 2636 | `06d75eb065ad353ea431f8faa7fb3e9667b775b7e8f4ae9155901a3ff1a02e06` |
| Character | `seren` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/seren-1.0.0.zip` | 2916 | `5b4e6e153f764bffc272658d289796e9da6fefe0868926bc46d75fb9f0e6a134` |
| MOD | `genshin-theme` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/genshin-theme-1.0.0.zip` | 3583358 | `25ad6d262475170bf991f24fc367a1b780f6fe441eb352b1a4eec17605fbd3cb` |

## Evidence status

| Check | Status | Owner | Date | Evidence and next action |
| --- | --- | --- | --- | --- |
| Deterministic local registry generation | Pass | Task 5 maintainer | 2026-08-30 | `node scripts/build-content-registry.mjs` generated 4 entries at commit `148af89`, before the documentation scaffold was added. The generated archive metadata matches the committed index. |
| Registry contract tests | Pass | Task 5 maintainer | 2026-08-30 | `npm test -- scripts/build-content-registry.test.mjs`: 1 file, 5 tests passed. |
| Official JSON publication | Pending | Kokoro Engine maintainer | Pending | No GitHub branch/release upload was performed in this task. Publish the index and record the commit or release URL. |
| Official archive publication | Pending | Kokoro Engine maintainer | Pending | The four archive URLs and checksums above are copied from the local index. Fetch each URL from a clean environment after publication and record HTTP status, byte count, and digest. |
| Browse smoke test | Pending | Kokoro Engine maintainer | Pending | Requires a published index and a running Kokoro application. Record registry endpoint, engine version, date, and screenshot or test log after a maintainer runs it. |
| Install/update/remove smoke test | Pending | Kokoro Engine maintainer | Pending | Requires published archives and a disposable app profile. Verify checksum, compatibility, activation, update preservation, and safe removal; record the result here. |

Pending is intentional. Local tests prove the generated contract but do not
prove that raw GitHub content exists, that an external release was accepted,
or that a desktop install succeeded. Do not replace these statuses with Pass
without external evidence.

## Publication checklist

The maintainer should attach or link:

1. The GitHub commit or release containing `registry/v1/index.json` and all
   listed archives.
2. Clean-environment fetch results for the JSON and each ZIP.
3. SHA-256 and byte-size comparisons against this table.
4. Browse evidence showing separate Character/MOD views, compatibility, trust,
   and permission/recommendation review.
5. Install, update, and remove evidence showing that character instances and
   MOD rollback behavior remain safe.

If any external check fails, record the failure, owner, date, and corrective
commit rather than editing history to imply a successful publication.
