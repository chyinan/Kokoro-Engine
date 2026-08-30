# Content registry

Kokoro Engine uses a static, versioned JSON registry. It has no accounts,
ratings, comments, creator payments, or marketplace service. The application
downloads the index and then validates each archive locally before it stages or
installs anything.

## Official endpoint and trust

The canonical index is:

```text
https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json
```

The official registry identity is
`github.com/chyinan/Kokoro-Engine/registry-v1`. The `official` label is valid
only when the index was obtained from that exact endpoint and the entry carries
that identity. An entry cannot grant itself official status through its own
JSON, package manifest, or a different mirror. A custom HTTPS endpoint and an
install-from-URL package are treated as community or unverified content.

The index is a transport and discovery document, not a security boundary. A
checksum detects corrupted or replaced bytes; it does not make executable MOD
code trustworthy. Review the trust label, author, license, permissions, and
capabilities before installing.

## Registry entry contract

`registry/schema/registry-v1.schema.json` is the machine-readable schema. Each
entry has these fields:

| Field | Contract |
| --- | --- |
| `content_type` | `character` or `mod`. The two types use separate install and trust paths. |
| `id`, `version`, `name`, `author`, `description` | Stable package identity and display metadata. `id` is lowercase kebab-case; `version` is semantic versioning. |
| `preview` | Zero or more HTTPS or package-relative preview references. An empty list is valid. |
| `engine_version` | A semver requirement checked against the running engine and the package manifest. |
| `download_url` | HTTPS URL whose basename is exactly `<id>-<version>.zip`. |
| `archive_size` | Positive byte count for the published ZIP. |
| `sha256` | Lowercase SHA-256 digest of the complete ZIP, exactly 64 hexadecimal characters. |
| `trust`, `trust_source`, `registry_identity` | Trust metadata resolved from the source endpoint. Only the canonical endpoint may produce `official`. |
| `permissions` | Unique, known MOD permissions. Character entries normally leave this empty. |
| `recommendations` | Boolean `vision`/`memory` suggestions plus `mcp_servers` and `bot_platforms` names. Recommendations never enable capabilities. |

The index must contain `schema_version: 1`, `registry_version: 1`, and unique
`content_type:id` pairs. A registry entry and its archive must agree on ID,
version, engine range, manifest type, filename, byte size, and checksum.

## Package boundaries

Character archives are declarative. A character package contains a root
`character.json`, `LICENSE.md` (or another license-named file), optional
`cues.json`, and supported media or Live2D data. It must not contain JavaScript,
HTML components, shell scripts, native libraries, executables, secrets, or
arbitrary network endpoints. See
[Creating character packages](creating-character-packages.md).

MOD archives follow the existing MOD runtime and may contain HTML, CSS,
JavaScript, JSON, fonts, and media. They are executable content. Requested
permissions and capabilities are reviewed before installation, and a URL MOD
always shows an untrusted-code warning. See
[Creating MOD packages](creating-mod-packages.md).

Both paths reject absolute paths, parent-directory traversal, symlinks, missing
manifests, incompatible engine requirements, corrupt ZIPs, and oversized
archives. Failed extraction leaves the previous installed version in place.
Removing a package removes package resources only; character instances,
conversations, memories, settings, and global Live2D library models survive.

## Build and validate locally

The deterministic builder reads bundled package directories, writes
`registry/packages/<id>-<version>.zip`, calculates the byte size and SHA-256,
and writes `registry/v1/index.json`. It sorts paths and uses stored ZIP entries
so a repeat build is reviewable. Run these checks from the repository root:

```bash
# Rebuild the local registry artifacts and index.
node scripts/build-content-registry.mjs

# Validate registry shape, HTTPS/basename rules, checksums, and trust metadata.
npm test -- scripts/build-content-registry.test.mjs

# Exercise Rust manifest and registry validation tests.
cargo test --manifest-path src-tauri/Cargo.toml registry::manifest -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml characters::manifest -- --nocapture
```

`characters/template` is documentation scaffolding, not a publishable entry.
The current builder walks every child directory under `characters/`, so a
registry build from this checkout will also see the scaffold. Generate the
official index from a package-only source tree (or a clean checkout with the
scaffold absent), then copy the generated index and ZIPs into the release
checkout. Review the generated entry list and abort if `my-character` or any
other scaffold appears. Copy the template to a new package directory only
after the package-only build input has been prepared. Do not hand-edit a
published checksum or archive size.

This is an input-layout rule, not a trust bypass: the template is still
validated as a declarative package when a creator copies it, but it must never
be presented as an official registry entry.

For a manual archive check, compare the generated values with the archive:

```bash
sha256sum registry/packages/my-character-0.1.0.zip
stat -c '%s' registry/packages/my-character-0.1.0.zip
# Windows PowerShell equivalents:
Get-FileHash registry/packages/my-character-0.1.0.zip -Algorithm SHA256
(Get-Item registry/packages/my-character-0.1.0.zip).Length
```

The Rust tests also cover package-path safety and declarative character
content. Use the application install path for a final smoke test: browse the
registry, inspect compatibility/trust, install a package, switch to it, then
remove it and confirm the user instance still exists.

## Checksums and publication

Only publish artifacts produced by the builder. A release review records the
exact index URL, each archive URL, SHA-256, byte size, commit or release that
contains the files, and the result of browse/install smoke tests. Record an
owner, date, and status for every check. The review must distinguish local
validation from external publication; a local green test is not evidence that
GitHub accepted an upload.

The maintainer publication sequence is:

1. Review manifests, complete license notices, permissions, and generated
   archive contents.
2. Run the builder and all validation commands above.
3. Commit `registry/v1/index.json` and the corresponding ZIPs together.
4. Publish the JSON and ZIPs to the official GitHub branch or release location.
5. Fetch the raw URLs from a clean environment and verify HTTP status, byte
   size, and SHA-256 against the committed index.
6. Run browse/install/remove smoke tests and record evidence in
   `docs/release-reviews/registry-publication.md`.

Do not label a mirror or a URL-installed package official. If publication is
not complete, leave the evidence status as `Pending` and name the maintainer
who owns the external action.

## PR submission checklist

Creator submissions should include:

- One immutable package ID and semantic version.
- A manifest and license file that match the package contents.
- Compatibility range tested against the supported engine.
- A complete list of assets, source URLs, authors, and redistribution rights.
- The generated archive size and SHA-256.
- A short description and preview references with no secrets or private paths.
- Permission and capability rationale for every MOD request.
- Results for schema, archive, and install smoke tests.

Submit the package source and generated registry changes in a pull request.
Keep unrelated application changes out of the PR. A maintainer reviews content,
licenses, compatibility, archive safety, and trust metadata before merging.
