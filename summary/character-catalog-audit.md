# Character Catalog Audit

> Audit date: 2026-08-31

## Scope

Audited the main-surface character catalog: card rendering, description clamping, themed scrolling, selection, template instantiation, import, edit, duplicate, restore defaults, conflict resolution, recommendations, error feedback, and the related Settings character manager.

## Conclusion

The feature is not fully implemented from a user-facing reliability perspective. The normal selection path and the backend persistence foundation exist, but several failure and synchronization paths are incomplete. The screenshot's `[object Object]` is a confirmed frontend error-formatting bug.

This report records the pre-fix audit. The current fix pass is tracked separately in the working tree.

## Implemented and evidenced

- The main catalog is mounted in `src/App.tsx` and receives characters, templates, activation, CRUD, import, and recommendation callbacks.
- The catalog renders active state, name, description, fallback icons, import, edit, duplicate, restore-default, and conflict-resolution actions.
- Long descriptions now use `line-clamp-2` without the overriding `block` class.
- The catalog list uses the existing `.scrollable` class, matching the Settings scrollbar style.
- Character activation is routed through the serialized activation service. Backend tests cover activation precedence, greeting ownership/consumption, rollback, asset fallback, and concurrent revisions.
- Backend tests cover character CRUD, template instantiation, reconciliation, managed avatar ownership, duplication, restore, and resource cleanup.
- Recommendation UI requires explicit confirmation before applying capability changes; its frontend tests cover dismiss and confirm behavior.
- `npm test` passed: 45 test files and 243 tests.
- `npm run build` passed: TypeScript and Vite production build completed. Existing Vite chunk/dynamic-import warnings remain.
- `npm run check:ipc` passed: 166 invoked command names are registered.
- `cargo test --no-run` and `cargo check` passed; Clippy reported zero errors. Running Rust tests is blocked by the documented Windows `onnxruntime.dll` `STATUS_ENTRYPOINT_NOT_FOUND` environment failure.

## Confirmed defects

### High: structured action failures render as `[object Object]`

`src/ui/widgets/CharacterCatalog.tsx:153` converts every non-`Error` failure with `String(reason)`. The IPC bridge intentionally preserves structured Tauri errors as objects, and `getKokoroErrorMessage` already exists in `src/lib/kokoro-bridge.ts`. Any failed select, duplicate, restore, or reconciliation action can therefore show `[object Object]` instead of the backend message.

The same pattern also exists in `src/ui/widgets/CharacterRecommendationDialog.tsx:121`.

### High: catalog import failures are invisible and not tracked as pending

`src/App.tsx:2321` dispatches `import_character` and returns immediately. The actual file picker, parsing, persistence, and activation run later in `src/App.tsx:2218-2259`; failures only go to `console.error`. The catalog cannot display an import error, cannot keep its pending state until the import finishes, and cannot distinguish cancel from failure.

### High: Settings CRUD and the main catalog can become stale

`src/App.tsx:522` owns the catalog's `characters` state, while `src/ui/widgets/CharacterManager.tsx:149` owns a separate list. Settings create, edit, import, and delete mutate only the manager's local state (`src/ui/widgets/CharacterManager.tsx:250-354`). The `CharacterCatalog` edit action opens Settings at `src/App.tsx:2326-2330`, but no callback refreshes App's list afterward. As a result, editing or creating a character in Settings can leave the main catalog showing old data until reload.

### Medium: multiple installed template versions can duplicate cards and React keys

`src-tauri/src/characters/catalog.rs:218-223` discovers and returns every installed version. `src/ui/widgets/CharacterCatalog.tsx:97` maps every template into a card, but a matching instance is reused for each version and the key is based only on the instance action ID (`src/ui/widgets/CharacterCatalog.tsx:223`). Installing both `1.2.0` and `1.10.0` can therefore render duplicate cards with duplicate keys.

### Medium: “latest template” selection uses lexical ordering

`src/App.tsx:2355-2358` sorts versions with `localeCompare`. SemVer values such as `1.10.0` and `1.2.0` are not ordered numerically by that comparison, so conflict resolution can choose the wrong template version.

### Medium: conflict-resolution action is shown when no update exists

`src/ui/widgets/CharacterCatalog.tsx:251-254` shows the conflict-resolution button for every template-backed instance. With the current built-in catalog, all templates are `1.0.0`, so the action is visible even when there is no newer version or conflict. The backend accepts the same version and rewrites the instance instead of the UI explaining that there is nothing to update.

### Medium: startup catalog failures silently degrade to an import-only surface

`src/App.tsx:1256-1275` loads characters and templates in one `Promise.all`; any failure is logged and `runtimeReady` is still set to true. There is no user-facing error or retry state, so a catalog discovery failure can look like an empty catalog with only an import button.

### Scope gap: no explicit preview surface

The phase-3 design requires a main catalog with avatar, description, preview, active state, import, edit, duplicate, and restore-default actions. The current catalog has card display and descriptions but no separate preview, expanded details, or preview action. Delete is available in the Settings character manager, but not in the main catalog; the phase-3 implementation plan does not explicitly require main-surface delete.

## Coverage gaps

The current catalog tests cover description/scrollbar classes, avatar URL rendering, dependency routing, and successful/plain-`Error` failures. They do not cover the actual App callback chain for import, template instantiation, Settings-to-catalog refresh, structured error rendering, multiple installed versions, no-update conflict resolution, startup failure, or a preview surface. Passing the current test suite therefore does not establish complete feature behavior.

## Recommended fix order

1. Normalize catalog and recommendation errors through `getKokoroErrorMessage`.
2. Make import a real awaited operation with success/failure callbacks or a shared action result state.
3. Establish one source of truth or an explicit refresh callback for character mutations from Settings.
4. Deduplicate templates by character ID and compare versions with a SemVer parser before exposing reconciliation.
5. Add user-facing catalog loading/error/retry states and decide whether preview belongs in this surface.
