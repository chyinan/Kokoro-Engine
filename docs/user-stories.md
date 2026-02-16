# Kokoro Engine — User Stories

> **Version:** 1.0  
> **Last Updated:** 2026-02-11  
> **Derived from:** [PRD.md](file:///d:/Program/Kokoro%20Engine/docs/PRD.md)

---

## Story Format

> **As a** [role], **I want to** [action], **so that** [outcome].

Each story includes **acceptance criteria** and a **priority** label:

- 🔴 **P0** — Must-have for MVP
- 🟡 **P1** — Important, planned for Phase 2
- 🔵 **P2** — Future / nice-to-have

---

## Epic 1: Live2D Character Interaction

### US-1.1 — View a Live2D Character 🔴

**As a** user, **I want to** see a Live2D character rendered in the app window, **so that** I have a visual companion to interact with.

**Acceptance Criteria:**
- [ ] Live2D model loads and renders at 60fps via PixiJS + Cubism SDK
- [ ] Character is displayed in the main viewport with generous space
- [ ] Model loads from a configurable URL or local path
- [ ] Loading failures show a graceful fallback, not a crash

---

### US-1.2 — Character Gaze Tracking 🔴

**As a** user, **I want** the character's eyes to follow my cursor, **so that** interactions feel alive and responsive.

**Acceptance Criteria:**
- [ ] Character gaze follows mouse position within the viewport
- [ ] Movement is smooth and natural (interpolated, not snapping)
- [ ] Gaze resets gracefully when cursor leaves the window

---

### US-1.3 — Character Expressions 🔴

**As a** user, **I want** the character's expression to change based on mood and conversation context, **so that** the character feels emotionally responsive.

**Acceptance Criteria:**
- [ ] Expressions map to named states (e.g., `neutral`, `happy`, `sad`, `surprised`)
- [ ] Expression changes animate smoothly (blend, not snap)
- [ ] Expressions can be triggered via `set_expression` IPC command
- [ ] AI responses can suggest an expression via `ChatResponse.expression`

---

### US-1.4 — Hit Area Interactions 🟡

**As a** user, **I want to** click or tap on parts of the character (head, body) to trigger reactions, **so that** the character feels interactive beyond just chat.

**Acceptance Criteria:**
- [ ] Hit areas are defined per model
- [ ] Clicking a hit area triggers an expression or animation
- [ ] Reactions are configurable per character mod

---

## Epic 2: Chat System

### US-2.1 — Send a Text Message 🔴

**As a** user, **I want to** type a message and send it to the character, **so that** I can have a conversation.

**Acceptance Criteria:**
- [ ] Text input field with send button
- [ ] Input clears on send with entrance animation for the message
- [ ] Empty messages are rejected with feedback
- [ ] Message appears in conversation view immediately

---

### US-2.2 — Receive Streaming AI Responses 🔴

**As a** user, **I want to** see the character's response appear word-by-word in real time, **so that** the conversation feels natural and alive.

**Acceptance Criteria:**
- [ ] Response text streams token-by-token via `chat-delta` events
- [ ] A typing/thinking indicator shows before first token arrives
- [ ] Streaming errors display a gentle notification
- [ ] Full response is saved to conversation history on `chat-done`

---

### US-2.3 — View Conversation History 🔴

**As a** user, **I want to** scroll through past messages in the current session, **so that** I can reference earlier conversation.

**Acceptance Criteria:**
- [ ] Messages render in chronological order with user/assistant labels
- [ ] Scrollable container with smooth auto-scroll on new messages
- [ ] User can scroll up without being forced back down

---

### US-2.4 — Clear Conversation History 🔴

**As a** user, **I want to** clear the conversation and start fresh, **so that** I can reset the context.

**Acceptance Criteria:**
- [ ] Clear button in the UI
- [ ] Calls `clear_history` backend command
- [ ] Chat area visually resets
- [ ] Confirmation prompt before clearing (optional)

---

## Epic 3: AI Configuration

### US-3.1 — Configure LLM API Connection 🔴

**As a** user, **I want to** enter my API key and endpoint, **so that** I can connect the character to an LLM service.

**Acceptance Criteria:**
- [ ] Settings panel with fields for API key, endpoint URL, and model name
- [ ] Supports any OpenAI-compatible API
- [ ] API key is stored locally (never sent to non-configured servers)
- [ ] Connection error shows clear feedback

---

### US-3.2 — Set Character Persona 🔴

**As a** user, **I want to** define the character's personality via a system prompt, **so that** the character responds in-character.

**Acceptance Criteria:**
- [ ] Text area for editing the system prompt / personality card
- [ ] Changes take effect on the next message via `set_persona`
- [ ] Default persona loads if none is set
- [ ] Persona is persisted per character

---

### US-3.3 — Character Mood State 🔴

**As a** user, **I want** the character to have a mood that evolves over the conversation, **so that** interactions feel emotionally dynamic.

**Acceptance Criteria:**
- [ ] Mood is a float from `0.0` (sad) to `1.0` (happy)
- [ ] AI responses include `mood_delta` to shift mood
- [ ] Mood influences character expression
- [ ] Mood is visible in the UI (subtly, e.g., ambient color shift)

---

## Epic 4: TTS (Text-to-Speech)

### US-4.1 — Hear the Character Speak 🔴

**As a** user, **I want** the character's response to be spoken aloud, **so that** the experience feels like a real conversation.

**Acceptance Criteria:**
- [ ] TTS synthesizes AI response text to audio
- [ ] Audio plays via Web Audio API
- [ ] Character shows a speaking indicator during playback
- [ ] Text-only fallback if no TTS provider is configured

---

### US-4.2 — Configure TTS Provider 🟡

**As a** user, **I want to** choose which TTS provider to use (e.g., OpenAI, local, cloud), **so that** I control voice quality and cost.

**Acceptance Criteria:**
- [ ] Settings panel lists available providers
- [ ] User can select default provider
- [ ] Voice, speed, and pitch options per provider
- [ ] Falls back gracefully if provider fails

---

### US-4.3 — Emotion-Driven Voice 🟡

**As a** user, **I want** the character's voice to vary with emotion, **so that** speech sounds natural and expressive.

**Acceptance Criteria:**
- [ ] `TtsParams.emotion` is passed to the provider
- [ ] Providers that support emotion vary tone/style accordingly
- [ ] Providers that don't support it ignore the parameter gracefully

---

## Epic 5: Mods & Customization

### US-5.1 — View Installed Mods 🔴

**As a** user, **I want to** see a list of installed mods, **so that** I know what's available.

**Acceptance Criteria:**
- [ ] Mod list panel shows all discovered mods from `mods/` directory
- [ ] Each mod shows name, version, description
- [ ] `list_mods` command is called on panel open

---

### US-5.2 — Load a Mod 🔴

**As a** user, **I want to** activate a mod, **so that** it changes the character, theme, or behavior.

**Acceptance Criteria:**
- [ ] Click to load/activate a mod
- [ ] `load_mod` command triggers backend activation
- [ ] Mod effects (theme, character, UI) apply immediately
- [ ] Load errors show a gentle notification

---

### US-5.3 — Theme Switching 🟡

**As a** user, **I want to** change the visual theme, **so that** I can personalize the look and feel.

**Acceptance Criteria:**
- [ ] Themes are defined as token sets (colors, radius, shadows)
- [ ] Theme switch applies instantly via `ThemeProvider`
- [ ] Mods can bundle custom themes
- [ ] Default theme is always available as a fallback

---

### US-5.4 — Custom Character via Mod 🟡

**As a** creator, **I want to** package a custom character (Live2D model + persona + voice config) as a mod, **so that** others can use my character.

**Acceptance Criteria:**
- [ ] Mod manifest supports `character` field with model path, system prompt, TTS config
- [ ] Loading a character mod swaps the active model and persona
- [ ] Character-specific expression mapping is supported

---

## Epic 6: System & Infrastructure

### US-6.1 — Offline-First Startup 🔴

**As a** user, **I want** the app to launch without network access, **so that** I can use it anywhere.

**Acceptance Criteria:**
- [ ] App launches and renders UI with no internet
- [ ] AI features degrade gracefully (show "offline" state)
- [ ] Previously loaded models render from cache/local storage
- [ ] No crash or blocking spinner on network failure

---

### US-6.2 — View Engine Status 🔴

**As a** user, **I want to** see the engine's running status, **so that** I know if systems are healthy.

**Acceptance Criteria:**
- [ ] Engine info (name, version, platform) is accessible
- [ ] Active modules are listed
- [ ] Status is queryable via `get_system_status`

---

### US-6.3 — Persistent Conversation Storage 🟡

**As a** user, **I want** my conversations to be saved, **so that** I can resume them later.

**Acceptance Criteria:**
- [ ] Conversations are stored in local SQLite
- [ ] History loads on app restart
- [ ] User can manage (view, delete) past conversations

---

## Epic 7: Future Features (Post-MVP)

### US-7.1 — Semantic Memory Recall 🔵

**As a** user, **I want** the character to remember things I've told it, **so that** long-term interactions feel meaningful.

**Acceptance Criteria:**
- [ ] Important facts are stored in vector memory
- [ ] Character can recall relevant memories during conversation
- [ ] Memory is searchable via semantic similarity

---

### US-7.2 — Branching Narrative / Story Mode 🔵

**As a** user, **I want** to experience branching storylines with the character, **so that** interactions feel like a game or visual novel.

**Acceptance Criteria:**
- [ ] Story events trigger based on mood, keywords, or conversation milestones
- [ ] Branching choices are presented to the user
- [ ] Narrative state persists across sessions

---

### US-7.3 — Mobile Companion 🔵

**As a** user, **I want** to chat with my character on my phone, **so that** I can interact on the go.

**Acceptance Criteria:**
- [ ] Mobile app with core chat and character display
- [ ] Shared conversation history (optional sync)
- [ ] Touch-optimized UI

---

### US-7.4 — Community Mod Ecosystem 🔵

**As a** creator, **I want** to share my mods with the community, **so that** others can use my characters, themes, and plugins.

**Acceptance Criteria:**
- [ ] Mod marketplace or registry (browsable)
- [ ] One-click install from the marketplace
- [ ] Mod versioning and update support

---

## Story Map Summary

```
                    P0 (MVP)              P1 (Phase 2)           P2 (Future)
                    ─────────             ────────────           ───────────
Live2D              US-1.1 View           US-1.4 Hit areas
                    US-1.2 Gaze
                    US-1.3 Expressions

Chat                US-2.1 Send msg
                    US-2.2 Streaming
                    US-2.3 History
                    US-2.4 Clear

AI Config           US-3.1 LLM setup
                    US-3.2 Persona
                    US-3.3 Mood

TTS                 US-4.1 Speak          US-4.2 Provider cfg
                                          US-4.3 Emotion voice

Mods                US-5.1 View mods      US-5.3 Themes
                    US-5.2 Load mod       US-5.4 Custom chars

System              US-6.1 Offline        US-6.3 Persistence
                    US-6.2 Status

Future                                                           US-7.1 Memory
                                                                 US-7.2 Story
                                                                 US-7.3 Mobile
                                                                 US-7.4 Marketplace
```
