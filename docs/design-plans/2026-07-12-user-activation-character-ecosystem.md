# User activation and character ecosystem design

## Summary

Kokoro Engine will remain a single desktop application while gaining a first-class character content model. Versioned character templates provide avatars, personas, greetings, dialogue examples, and optional Live2D, background, voice, cue, and behavior presets. Users work with editable character instances, so template and application updates don't overwrite their changes, conversations, or memories.

The rollout starts with clearer positioning, three built-in characters, and a first-run path that reaches a successful reply without exposing the full settings surface or blocking on the memory model. The same package contract later supports a static official content registry. An AstrBot webhook adapter provides an external distribution channel without adding a new protocol to Kokoro.

## Definition of done

- Kokoro Engine remains one application and one installer. It does not split into separate companion, developer, or VTuber editions.
- A new user can choose a built-in character, connect an LLM provider, and receive a successful first reply within ten minutes without navigating the full settings surface.
- The application ships with at least three distinct built-in character templates. Every template has an avatar, persona, greeting, example dialogue, and presentation metadata. A template may embed its own Live2D model, but Live2D is optional and falls back to the built-in default model.
- Selecting a character creates or activates a user-owned character instance. Application and template updates never overwrite user edits, conversations, or memories.
- Character activation can apply character-specific presentation and behavior settings, including Live2D, background, TTS, cues, language, and proactive behavior. Sensitive capabilities such as vision, MCP tools, or external bot access still require explicit user consent.
- A minimal remote content registry supports browsing and installing official characters and MODs without introducing accounts, ratings, comments, or a marketplace backend.
- An initial AstrBot adapter uses Kokoro's existing authenticated webhook contract and is released as a distribution experiment. A bidirectional real-time embodiment protocol is out of scope until adoption demonstrates demand.
- README, quick-start documentation, repository metadata, release material, and community entry points explain Kokoro through user outcomes and demonstrable character experiences rather than its technology stack.

## Glossary

- **Character template:** Versioned, read-only source content distributed with the application or through the registry.
- **Character instance:** The user's editable character record, including conversations, memories, and runtime overrides.
- **Character package:** A ZIP archive containing `character.json`, a required avatar, license metadata, and optional presentation assets.
- **Character activation:** The coordinated operation that makes a character active and applies its prompt and safe runtime profile.
- **Runtime profile:** Optional per-character Live2D, background, TTS, cue, language, and proactive behavior settings.
- **Capability recommendation:** A suggestion that a character works well with a sensitive feature. It doesn't grant permission or enable the feature.
- **Content registry:** A static index of installable character packages and MODs with metadata, compatibility, checksums, and trust labels.
- **Official label:** A registry trust marker for content reviewed and distributed by the Kokoro project. It doesn't apply to arbitrary install-from-URL packages.
- **AstrBot adapter:** A separate AstrBot plugin that forwards channel messages to Kokoro's generic webhook and returns Kokoro replies.

## Architecture

### Product structure

Kokoro Engine keeps one installer. Built-in characters are content shipped inside that installer, not separate product editions.

The design separates an immutable character template from a user-owned character instance:

```text
built-in or remote character package
    -> character template catalog
    -> user character instance
    -> character activation service
    -> persona, Live2D, avatar, background, TTS, cues, and safe behavior defaults
```

A template describes the official character. An instance stores the user's editable copy, conversation identity, and template origin. Template updates may be offered to the user, but they never replace instance fields automatically.

### Character package contract

Character packages use a versioned ZIP format. Bundled packages live under `characters/` and are copied to the application data directory during first-run initialization. Remote packages use the same format.

```text
character-id/
|-- character.json
|-- avatar.webp
|-- LICENSE.md
|-- background.webp                 # optional
|-- live2d/                         # optional
|   `-- ...model3.json and assets
`-- cues.json                       # optional
```

`avatar` is required. `live2d` is optional. A character without a valid Live2D asset uses `BUILTIN_LIVE2D_MODEL_PATH`.

```typescript
interface CharacterTemplateManifest {
  schema_version: 1;
  id: string;
  version: string;
  name: string;
  description: string;
  author: string;
  license: string;
  locale?: string;
  avatar: string;
  persona: string;
  greeting: string;
  example_dialogue?: string;
  assets?: {
    live2d_model?: string;
    background?: string;
    cue_profile?: string;
  };
  runtime?: {
    tts_provider?: string;
    tts_voice?: string;
    tts_speed?: number;
    tts_pitch?: number;
    response_language?: string;
    proactive_enabled?: boolean;
  };
  recommendations?: {
    vision?: boolean;
    memory?: boolean;
    mcp_servers?: string[];
  };
}
```

`runtime` contains settings that are safe to apply when a character becomes active. `recommendations` never grants permissions or enables sensitive capabilities. The UI explains the recommendation and asks for confirmation.

### Storage model

The `characters` table remains the source of truth for user-owned instances. A migration adds:

- `template_id` and `template_version` for origin tracking.
- `avatar_path` and `greeting` for first-class character presentation.
- `example_dialogue` for prompt composition without flattening it into `persona`.
- `runtime_profile_json` for optional Live2D, background, TTS, cue, language, and proactive behavior bindings.
- `user_modified_at` so updates can distinguish user edits from the original template snapshot.

Binary assets stay on disk. SQLite stores only normalized paths and structured metadata. Existing conversations and memories already use `character_id`, so their isolation model remains unchanged.

Imported SillyTavern cards create custom user instances without a `template_id`. PNG imports retain the source PNG as the avatar. Greeting and example dialogue become separate fields instead of being appended to the persona string.

### Character activation

A single character activation service owns switching behavior. Both the main UI and settings UI call the same service.

Activation performs these actions in order:

1. Persist the active character ID.
2. Compose and apply the character prompt.
3. Resolve the character Live2D path, falling back to the built-in model when missing or invalid.
4. Apply the character background, TTS selection, language, proactive behavior, and cue profile when configured.
5. Emit one runtime settings event so chat, Live2D, TTS, bot, and MOD consumers refresh from the same state.
6. Present capability recommendations separately. Activation does not silently enable vision, MCP servers, microphone access, or external bots.

User overrides take precedence over template defaults. Switching back to a character restores that character's last saved runtime profile.

### Built-in character catalog

The first release ships at least three original or redistributable characters:

1. A warm daily companion focused on memory, longer conversations, and gentle proactive messages.
2. A lively friend with concise replies, teasing humor, stronger expressions, and more frequent cues.
3. An immersive role-play character with a defined world, scenario, greeting, and example dialogue.

Each character must have a distinct avatar, description, greeting, persona, dialogue style, background, and cue behavior. Independent Live2D models are preferred when licensed assets are available, but aren't a release requirement.

The character selector appears in the first-run flow and in the main application surface. Users don't need to open the full settings panel to switch characters.

### First-run flow

The current settings-led tour becomes an outcome-led setup:

```text
language -> choose character -> connect LLM -> connection test -> first chat
```

The setup uses a focused provider wizard for OpenAI-compatible APIs and local Ollama discovery. It doesn't expose every generation or context option.

The memory embedding model downloads in the background. Until it is ready, chat runs without semantic retrieval and shows a non-blocking memory status. A missing memory model cannot block the first reply.

After the first successful response, the application introduces optional voice, vision, MCP, MOD, bot, and advanced settings.

### Content registry

The first registry is a static JSON document hosted on GitHub. It lists official characters and MODs and points to versioned ZIP assets.

Each entry includes:

- Content type, ID, name, version, author, description, preview images, and tags.
- Minimum and maximum compatible engine versions.
- Download URL, archive size, and SHA-256 checksum.
- Requested MOD permissions or character capability recommendations.
- Trust level. Only entries from the official registry receive the official label.

The application supports browse, install, update, remove, and install-from-URL. It doesn't include accounts, ratings, comments, creator payouts, personalized recommendations, or a registry upload backend.

Remote MOD installation retains the existing permission review. A checksum verifies transport integrity, but it doesn't make third-party code trusted.

### AstrBot adapter

The initial AstrBot integration is a separate AstrBot plugin. It forwards supported AstrBot messages to Kokoro's existing generic webhook and returns the `BotReply` to AstrBot.

The plugin config contains:

- Kokoro endpoint and bearer token.
- Default character ID.
- Conversation mapping strategy for private and group chats.
- Text, image, audio, and voice-reply toggles supported by the existing webhook contract.

Version one treats Kokoro as the character and response engine behind an AstrBot channel. It doesn't attempt to make Kokoro a live rendering client for an AstrBot-controlled agent. That bidirectional event protocol requires separate evidence and a separate design.

### Distribution and feedback

Repository and release material lead with observable experiences:

- A character notices visible screen context and reacts.
- Two built-in characters respond differently to the same message.
- A new user imports a SillyTavern card and starts chatting.
- An AstrBot installation binds a channel to a Kokoro character.

GitHub Discussions provides feedback, character sharing, MOD sharing, and support categories. Early activation quality is measured through clean-machine usability sessions, release downloads, support reports, registry downloads, and AstrBot plugin adoption. Anonymous remote telemetry is not part of this design and requires a separate privacy review.

## Existing patterns

- Character CRUD already uses Tauri commands in `src-tauri/src/commands/characters.rs` with TypeScript bridge functions in `src/lib/kokoro-bridge.ts`.
- Character identity already isolates conversations and memory through `character_id` in the SQLite schema. This design extends that record instead of introducing a second character database.
- SillyTavern v1/v2/v3 JSON and PNG parsing already exists in `src/lib/character-card-parser.ts`.
- Live2D models already use filesystem assets, a custom protocol, profiles, import/export commands, and a built-in fallback in `src-tauri/src/commands/live2d.rs`.
- Tauri bundle resources are declared in `src-tauri/tauri.conf.json`. Bundled MODs are copied into application data on first run in `src-tauri/src/lib.rs`; character packages follow the same lifecycle.
- MOD installation already accepts ZIP archives through `src-tauri/src/mods/` and `src/ui/mods/ModList.tsx`. The registry wraps this existing installation path rather than replacing the MOD runtime.
- Runtime settings currently use `src/lib/app-settings.ts` and global state in `src/App.tsx`. Character activation introduces a controlled per-character overlay, then emits the same runtime settings events existing consumers use.
- The generic authenticated webhook in `src-tauri/src/commands/bot.rs` already accepts text, images, and audio and returns a structured reply. AstrBot version one uses this contract.
- Frontend tests live beside their modules as `*.test.ts` or `*.test.tsx`. Rust command, migration, and security tests stay beside the affected backend modules.

## Implementation phases

<!-- START_PHASE_1 -->
### Phase 1: Positioning and community entry points

**Goal:** Make the project understandable and testable before the product changes ship.

**Components:**

- `README.md`, translated READMEs, and `docs/` quick-start material: user-outcome headline, short demo, direct download path, three example scenarios, troubleshooting, and clearer separation between user and developer content.
- GitHub repository metadata: description, homepage, discovery topics, Discussions categories, issue forms, and release template.
- Character briefs and licensing records for the initial built-in catalog.

**Dependencies:** None.

**Done when:** Five people unfamiliar with the repository can identify the product, intended user, primary experience, and download path from the first README viewport; all initial character assets have recorded redistribution terms.
<!-- END_PHASE_1 -->

<!-- START_PHASE_2 -->
### Phase 2: Character package and persistence foundation

**Goal:** Add a stable package contract and user-owned instance model without changing the current chat experience.

**Components:**

- New character package domain under `src-tauri/src/characters/`: manifest parsing, path validation, catalog discovery, built-in resource installation, and package metadata.
- New SQLite migration under `src-tauri/migrations/` for template origin, avatar, greeting, example dialogue, runtime profile, and edit tracking.
- Extended character commands in `src-tauri/src/commands/characters.rs` and matching contracts in `src/lib/kokoro-bridge.ts`.
- Tauri resources in `src-tauri/tauri.conf.json` and first-run copy behavior in `src-tauri/src/lib.rs`.
- Updated SillyTavern import behavior in `src/lib/character-card-parser.ts` and `src/ui/widgets/CharacterManager.tsx`.

**Dependencies:** Phase 1 character asset and license decisions.

**Done when:** Bundled and imported character packages validate safely; templates create editable instances; avatar, greeting, examples, and runtime profile round-trip through IPC and backup/restore; migrations preserve existing characters, conversations, and memories; focused frontend and Rust tests pass.
<!-- END_PHASE_2 -->

<!-- START_PHASE_3 -->
### Phase 3: Character activation and main-surface catalog

**Goal:** Let users see, select, and switch complete characters without entering advanced settings.

**Components:**

- Character activation service shared by `src/App.tsx`, `src/ui/widgets/CharacterManager.tsx`, and the main character selector.
- Main-surface character catalog with avatar, description, preview, active state, import, edit, duplicate, and restore-default actions.
- Per-character runtime profile integration with Live2D, background, TTS, language, proactive behavior, and cue settings.
- Three built-in character packages and localized catalog text.
- Fallback and error states for missing avatars, unavailable voices, invalid backgrounds, and missing Live2D assets.

**Dependencies:** Phase 2 package and persistence foundation.

**Done when:** Users can switch between three built-in characters from the main UI; each character has isolated conversations and memories; character-specific settings restore on return; a missing optional asset falls back without blocking chat; tests cover activation precedence and fallback behavior.
<!-- END_PHASE_3 -->

<!-- START_PHASE_4 -->
### Phase 4: First-reply onboarding

**Goal:** Reduce first-run setup to the minimum path needed for one successful conversation.

**Components:**

- Reworked `src/ui/widgets/OnboardingOverlay.tsx` flow: language, character, LLM connection, connection test, and chat.
- Focused provider setup built from the existing API configuration in `src/ui/widgets/settings/ApiTab.tsx`, including Ollama discovery and OpenAI-compatible presets.
- Non-blocking memory initialization across `src/lib/memory-model-gate.ts`, `src/ui/widgets/MemoryModelDownloadDialog.tsx`, `src/ui/widgets/ChatPanel.tsx`, and backend memory initialization.
- Basic and advanced settings grouping in `src/ui/widgets/SettingsPanel.tsx` without removing existing functionality.
- Localized setup errors and recovery actions.

**Dependencies:** Phase 3 character catalog and activation.

**Done when:** At least four of five clean-machine testers receive a successful first response within ten minutes without opening advanced settings; memory download failure doesn't block basic chat; onboarding tests cover continuation, retry, dismissal, and resume behavior.
<!-- END_PHASE_4 -->

<!-- START_PHASE_5 -->
### Phase 5: Minimal character and MOD registry

**Goal:** Provide one content discovery and installation surface without building a marketplace service.

**Components:**

- Versioned static registry contract and official registry repository or hosted JSON artifact.
- Content library UI with separate character and MOD views, preview media, compatibility, permissions, trust label, install, update, removal, and install-from-URL.
- Character package download and installation in `src-tauri/src/characters/`.
- Registry-backed MOD installation using the existing commands in `src-tauri/src/commands/mods.rs` and manager in `src-tauri/src/mods/`.
- SHA-256 verification, archive path validation, size limits, compatibility checks, and actionable failure messages.
- Creator templates, validation instructions, and PR-based submission documentation.

**Dependencies:** Phase 2 package contract. Phase 3 provides the user-facing character catalog patterns.

**Done when:** Official character and MOD entries can be browsed, installed, updated, and removed; incompatible or corrupt packages are rejected; user instances and settings survive package updates; security and path traversal tests pass.
<!-- END_PHASE_5 -->

<!-- START_PHASE_6 -->
### Phase 6: AstrBot distribution adapter

**Goal:** Publish a small integration that exposes Kokoro characters to the AstrBot user base.

**Components:**

- Separate AstrBot plugin repository with endpoint, bearer token, character, conversation, and media settings.
- Documented request and reply contract matching `src-tauri/src/commands/bot.rs`.
- Kokoro documentation for enabling the loopback webhook, selecting a character, and testing the adapter.
- Integration tests for text and supported media, plus failure handling for unavailable Kokoro instances and invalid tokens.
- Marketplace listing, screenshots, and one short demonstration.

**Dependencies:** Existing generic webhook. Phase 3 provides a stronger character selection experience but doesn't block initial protocol work.

**Done when:** An AstrBot user can install the plugin, bind a Kokoro character, and receive replies in a supported channel without modifying Kokoro source code; the integration is published and documented.
<!-- END_PHASE_6 -->

<!-- START_PHASE_7 -->
### Phase 7: Release and feedback loop

**Goal:** Turn the product changes into repeatable acquisition and learning rather than a one-time release.

**Components:**

- Release material demonstrating character selection, first-reply onboarding, character import, registry installation, and AstrBot integration.
- GitHub Discussions workflows for support, character submissions, MOD submissions, and product feedback.
- A small creator outreach program focused on original character cards, Live2D assets, voices, backgrounds, and MODs with clear license terms.
- A lightweight metrics review covering release downloads, support failures, setup completion from usability sessions, registry downloads, content submissions, repeat community participation, and AstrBot adoption.

**Dependencies:** Publish after Phase 4 for the activation release, then repeat after Phases 5 and 6.

**Done when:** Each release has one observable experience, one demo asset, one direct call to action, and a documented review of user feedback before the next feature phase begins.
<!-- END_PHASE_7 -->

## Additional considerations

### Scope convergence

This design adds one new domain abstraction: the character template. It doesn't create separate product editions, a second scripting runtime, or a marketplace backend.

The following work stays out of scope:

- Mobile clients, cloud sync, social feeds, creator accounts, ratings, payments, and personalized recommendations.
- A new plugin protocol that duplicates the MOD system.
- Automatic installation or activation of MCP servers, vision, microphone, filesystem, command execution, or bot permissions from a character package.
- Per-character LLM credentials. Provider credentials remain application-level secrets; a character may recommend a provider type but never embed credentials.
- Bidirectional real-time AstrBot embodiment until the webhook adapter produces concrete demand.

### Asset policy

Every bundled or official registry package includes author and license metadata. The official catalog accepts only assets with explicit redistribution rights. Package updates preserve previous license records for auditability.

Avatars should use PNG or WebP and enforce pixel and file-size limits. Live2D packages retain their upstream license files and are checked independently from the character persona or avatar license.

### Update policy

Template updates are immutable versions. Existing user instances keep their data. The UI may offer three explicit actions: keep current instance, create a new instance from the updated template, or reset selected fields after showing a diff.

### Failure policy

A character remains usable when optional presentation assets fail. The fallback order is:

1. Character-specific asset.
2. Previously valid user override.
3. Built-in Kokoro default.

Manifest validation and archive extraction failures return specific errors and leave the previous installed version intact.

### Measurement policy

The initial release doesn't add automatic remote analytics. Activation is measured with repeatable clean-machine tests and opt-in user feedback. Any later anonymous telemetry needs its own data inventory, consent UI, retention policy, endpoint security review, and documentation before implementation.

### Complexity review

The main complexity increase is the relationship between immutable templates, editable instances, and per-character runtime settings. That complexity directly supports built-in characters, imports, registry updates, and safe switching.

The design deliberately avoids accounts, server-side search, ratings, package signing infrastructure, and a bidirectional AstrBot protocol. Those features remain blocked until content supply or adoption proves the simpler design insufficient.
