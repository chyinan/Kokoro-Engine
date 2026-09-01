# Creating character packages

A character package is a versioned, declarative ZIP. It supplies persona and
presentation metadata; it does not run code. Users can edit their own character
instances after installation, so a later package version must never overwrite
their conversations, memories, greeting state, or runtime overrides.

## Start from the template

Copy the scaffold to a new directory and replace every placeholder before
submitting it:

```bash
cp -R characters/template characters/my-character
```

On PowerShell:

```powershell
Copy-Item -Recurse characters/template characters/my-character
```

The scaffold is intentionally non-executable. It contains only JSON and a
license document. It is not itself a registry package and must not be included
as a generated registry entry.

## Package layout

The package root must contain `character.json` and a complete license notice.
`cues.json` is optional but is included by the template.

```text
my-character/
|-- character.json
|-- LICENSE.md
|-- cues.json                         # optional
|-- avatar.webp                       # optional
|-- background.webp                   # optional
`-- live2d/                            # optional, licensed assets only
    |-- my-character.model3.json
    `-- ...
```

Supported character files are `character.json`, `cues.json`, license-named
files, PNG/JPEG/WebP images, audio files, and the supported Live2D model,
motion, physics, pose, expression, and user-data formats. Paths are relative
to the package root, use `/`, and cannot contain `..`, a drive letter, a
leading slash, or a symlink. JavaScript, HTML, CSS, shell scripts, native
libraries, ZIP files, and arbitrary JSON files are rejected.

## Fill in `character.json`

The manifest uses schema version `1` and is the source of truth for package
identity. Required fields are:

| Field | Guidance |
| --- | --- |
| `schema_version` | Set to `1`. |
| `engine_version` | Semver requirement supported by the package, for example `>=0.3.1, <0.5.0`. |
| `id` | Lowercase letters, numbers, and hyphens; 1–64 characters; no leading or trailing hyphen. |
| `version` | Semantic version for this immutable package, for example `1.0.0`. |
| `name`, `description`, `author`, `license` | User-facing identity and the license identifier matching `LICENSE.md`. |
| `persona` | The system persona. State voice, boundaries, and behavior without embedding credentials or private data. |
| `greeting` | Editable first greeting. It is inserted at most once for each new character instance. |

Optional fields include `locale`, `avatar`, `example_dialogue`, `assets`,
`runtime`, and `recommendations`. `avatar` and every path in `assets` must be
present in the archive and use supported relative paths. With no valid avatar,
background, or Live2D model, Kokoro uses its built-in presentation fallback.

Use runtime fields only for safe defaults and references:

```json
{
  "runtime": {
    "response_language": "en",
    "proactive_enabled": false,
    "tts": {
      "provider_type": "edge_tts",
      "voice": "en-US-JennyNeural",
      "speed": 1.0,
      "pitch": 0.0
    }
  },
  "recommendations": {
    "vision": false,
    "memory": true,
    "mcp_servers": [],
    "bot_platforms": []
  }
}
```

Provider IDs may refer to an already configured provider. A package must not
contain API keys, bearer tokens, custom base URLs, local model paths, or a
command that probes or changes the user's configuration. A recommendation is a
suggestion only: vision, memory, MCP, and bot access require separate user
confirmation and are never enabled by parsing the manifest.

## Cues and licenses

`cues.json` is data consumed by the presentation layer. Keep cue names stable,
use an explicit `default`, and keep intensity values in the range `0.0`–`1.0`.
If a cue references an asset, that asset must be in the package and listed in
the license record when its terms differ.

`LICENSE.md` must contain the complete license text or a clear license notice,
author, source URL, and attribution requirements for each bundled asset. Do
not copy a voice, avatar, background, or Live2D model unless its license grants
redistribution in this application. The manifest's `license` field and the
license file must agree.

## Validate before submission

Run JSON parsing first, then the repository validators:

```bash
# Parse both JSON documents without executing package content.
node -e "JSON.parse(require('fs').readFileSync('characters/my-character/character.json', 'utf8')); JSON.parse(require('fs').readFileSync('characters/my-character/cues.json', 'utf8')); console.log('JSON valid')"

# Run manifest, safe-path, declarative-content, and catalog tests.
cargo test --manifest-path src-tauri/Cargo.toml characters::manifest -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml characters::catalog -- --nocapture

# Run registry contract checks when preparing an archive.
npm test -- scripts/build-content-registry.test.mjs
```

For a local install smoke test, install the ZIP into a disposable profile,
activate the character, start a conversation, and verify that the greeting is
inserted once. Remove the package and confirm the instance, conversation, and
memory remain available with the built-in fallback. Record the result in the
registry publication review; do not describe a local test as official
publication evidence.

## Submit a package

Use the registry builder to create `<id>-<version>.zip` and update the index.
Review the generated archive, byte size, and SHA-256. Include the source
directory, generated metadata, license evidence, compatibility range, and
smoke-test result in a pull request. Keep the package version immutable: a
content change requires a new version and a new checksum.

The maintainer accepts only packages that pass content validation, have clear
redistribution rights, and keep the declarative character boundary. See
[Content registry](content-registry.md) for the official endpoint and trust
rules.
