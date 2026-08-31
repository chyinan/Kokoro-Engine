# Registry Domain

Last verified: 2026-08-31

## Purpose

Provide a static, versioned content source whose metadata and archives can be verified before any install or restore mutates user state.

## Contracts

- **Exposes**: registry manifest validation, bounded client fetch, authoritative character/MOD install/remove commands, and deterministic archive tooling.
- **Guarantees**: official labels require the canonical endpoint and identity; custom/URL sources are community/untrusted; archive checksum, size, manifest, engine range, license, path, and permission checks agree across JS and Rust.
- **Expects**: HTTPS sources, safe package IDs/versions, explicit permission confirmation for executable MODs, and staging paths inside managed roots.

## Dependencies

- **Uses**: character catalog, MOD manager, backup resolver, typed frontend bridge, registry schema and builder.
- **Used by**: ContentLibrary, data-only restore, creator publication tooling.
- **Boundary**: package removal must not delete user instances, conversations, memories, or settings.

## Invariants

- Reject symlink/junction/reparse parents, traversal, repeated/case-folded duplicate paths, unsafe files, redirects, oversized bodies, and invalid trust metadata.
- Stage and verify before atomic promotion; active character removal uses built-in presentation fallback and rollback on failure.
- `characters/template` is never published as an official artifact.

## Gotchas

- Rust runtime tests may fail to start locally because of the bundled ONNX DLL; compile/no-run/check/clippy gates remain required.
