# Custom Avatar Audit

> Audit date: 2026-09-01

## Conclusion

Custom avatar support is implemented across the backend and frontend, with the existing default avatar retained when no custom avatar is available or when a custom image cannot be loaded.

## Confirmed implemented

- The typed bridge exposes `createCharacterWithAvatar`.
- PNG character-card imports pass the complete PNG bytes to the backend.
- The backend validates and stores managed avatar resources under the character instance, copies them on duplicate, and removes/reverts owned resources during update, restore, and delete paths.
- The main Character Catalog renders a non-null `avatar_path` through `mapCharacterAvatarUrl`.
- Existing tests cover managed protocol URL mapping, PNG import bytes, and PNG-backed duplication/resource ownership.

## Frontend behavior

- Settings renders `character.avatar_path`, provides a PNG file picker and remove action, and persists both through the typed bridge.
- Missing or failed custom avatars fall back to the existing `UserCircle` default icon.
- Avatar updates disable both controls during the async operation and cache-bust the preview; the protocol also sends `Cache-Control: no-store`.
- The main catalog continues to resolve managed avatar URLs and retains its existing icon fallback for records without custom avatars.

## Resource safety and failure handling

- Managed avatar roots reject symlink/reparse redirects before creation, reads, staging, cleanup, or protocol serving.
- Avatar replacement stages the old directory, cleans the new directory on failure, explicitly restores the old directory, and reports cleanup/restoration failures.
- Successful replacement/removal finalization reports backup cleanup failures instead of silently logging them.

## Verification

- Frontend verification: 45 test files, 253 tests passed; production build and IPC contract check passed.
- Rust verification: `cargo test --no-run`, `cargo check`, and Clippy passed. Targeted runtime execution remains blocked before test startup by the existing Windows `onnxruntime.dll` `STATUS_ENTRYPOINT_NOT_FOUND` error.
