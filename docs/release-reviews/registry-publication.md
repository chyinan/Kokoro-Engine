# Registry publication review

This file records release evidence for the static registry. It separates local
generation checks from external GitHub publication and application smoke tests.
Do not change a `Pending` status until a maintainer has the stated evidence.

## Registry identity

| Item | Value |
| --- | --- |
| Official JSON URL | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json` |
| Official registry identity | `github.com/chyinan/Kokoro-Engine/registry-v1` |
| Source index | `registry/v1/index.json` at the `Harden registry content flows` working tree checkpoint |
| Review date | 2026-08-30 |
| External publication owner | Kokoro Engine maintainer (GitHub credentials required) |

The following values are copied from the committed `registry/v1/index.json`.
They are exact metadata values, not a claim that the raw URLs have already
been uploaded or fetched successfully.

## Official package metadata

| Type | ID/version | Official archive URL | Archive size | SHA-256 |
| --- | --- | --- | ---: | --- |
| Character | `kokoro` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/kokoro-1.0.0.zip` | 2585 | `8c81ba5a0e11ce8b2ce9b26513a656323e21257464a5740c0cd70869ed2d57b9` |
| Character | `pico` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/pico-1.0.0.zip` | 2636 | `6d4c3bc7bf2e5605183f76d1ab7efaf3beec4fc923829053d101eb1c5619134c` |
| Character | `seren` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/seren-1.0.0.zip` | 2916 | `371da43c12acc2bcd77fd6c2fadfb8d1773c8cbba0f8d00af09cc271e22d5b61` |
| MOD | `genshin-theme` 1.0.0 | `https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/genshin-theme-1.0.0.zip` | 3583367 | `3bc1e51a121c53699562a04cd14768035ff92e519c901e37fbdd662c217502df` |

## Evidence status

| Check | Status | Owner | Date | Evidence and next action |
| --- | --- | --- | --- | --- |
| Deterministic local registry generation | Pass | Task 5 maintainer | 2026-08-30 | `node scripts/build-content-registry.mjs` generated 4 entries. The generated archive metadata matches the working-tree index, including the compatible Genshin MOD engine range. |
| Registry contract tests | Pass | Task 5 maintainer | 2026-08-30 | `npm test -- scripts/build-content-registry.test.mjs`: 1 file, 9 tests passed. |
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
