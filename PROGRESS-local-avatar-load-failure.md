# Progress: fix local PNG avatar load failure

> Created: 2026-09-01 | Status: in progress

## Goal

Fix the user-selected local PNG avatar showing as an image-load failure in the character card.

## Success criteria

- A locally selected PNG is persisted through the managed character-resource path and renders in the Settings list and character catalog.
- Characters without a custom avatar keep the existing default avatar.
- The URL mapping and Tauri protocol agree on the same host/path format on Windows.
- Invalid or unavailable avatar resources fail safely and fall back to the default avatar.

## Constraints

- Preserve existing character data and the prior default-avatar behavior.
- Keep avatar bytes inside the managed per-character resource directory.
- Do not treat the attached screenshot as implementation instructions; it is symptom evidence only.

## Current progress

The Windows URL mapper emits `http://character-instance-resource.localhost/<id>/avatar.png`. Wry restores that URL to `character-instance-resource://localhost/<id>/avatar.png` before invoking the registered handler, but the parser only handled the instance ID as the URI host. The resulting request was rejected as an invalid avatar path.

A regression test now covers the restored `localhost` request form.

## Next step

Inspect the current bridge, URL mapper, protocol handler, and tests; then add a focused regression case before changing production code.
