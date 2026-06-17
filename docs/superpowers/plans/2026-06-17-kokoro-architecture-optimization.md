# Kokoro Architecture Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce maintenance risk in Kokoro Engine by deepening the chat, MOD action, settings, IPC, and memory modules without changing user-visible behavior.

**Architecture:** Keep public behavior stable and move complexity behind smaller interfaces. Start with tests and contract checks, then migrate one seam at a time: frontend settings, MOD action dispatch, chat turn state, backend chat helpers, and memory internals.

**Tech Stack:** Tauri v2, React 19, TypeScript, Vitest, Rust, Tokio, sqlx, Cargo tests, PowerShell on Windows.

---

## How To Use This Plan

Use one task per fresh conversation. Copy the "Prompt for new conversation" from the task you want to run. Do not ask a worker to do multiple tasks unless the earlier tasks are already merged or the files are disjoint.

Global rules for every task:

- Preserve runtime behavior unless the task explicitly says otherwise.
- Add or move tests before changing behavior.
- Keep commits optional. If the user wants commits, use the suggested commit message. Do not push unless the user explicitly asks.
- Run the targeted tests listed in the task. Run broader checks when a task touches shared interfaces.
- Prefer `rg` for search and `apply_patch` for manual edits.
- On this repo, shell commands should be prefixed with `rtk`, for example `rtk npm test`.

Recommended task order:

1. Task 1: Add architecture context docs.
2. Task 2: Add IPC contract check.
3. Task 3: Create frontend settings store.
4. Task 4: Migrate ChatPanel settings reads.
5. Task 5: Migrate SettingsPanel and App settings writes.
6. Task 6: Add MOD action dispatcher seam.
7. Task 7: Move low-risk MOD actions into dispatcher.
8. Task 8: Extract frontend chat turn reducer.
9. Task 9: Extract backend chat tag parser.
10. Task 10: Split memory embedding model lifecycle from MemoryManager.

## Baseline Commands

Run these before a broad refactor if the workspace state is uncertain:

```powershell
rtk git status --short
rtk npm test
rtk npm run build
rtk cargo test --manifest-path src-tauri/Cargo.toml
```

For Rust linting before review:

```powershell
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings
```

---

## Task 1: Add Architecture Context Docs

**Goal:** Give future agents stable domain language and record the architectural direction before code changes.

**Files:**

- Create: `CONTEXT.md`
- Create: `docs/adr/0001-frontend-runtime-settings-store.md`
- Create: `docs/adr/0002-chat-turn-state-machine.md`
- Create: `docs/adr/0003-ipc-contract-check.md`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 1 only. Create CONTEXT.md and the three ADR files listed in Task 1. Do not change application code. Run a quick file existence check afterward.
```

- [ ] **Step 1: Create the context doc**

Write `CONTEXT.md` with this content:

```markdown
# Kokoro Engine Context

Kokoro Engine is a Tauri v2 desktop virtual character interaction engine.

## Domain Terms

- Character: The active virtual persona, including name, persona prompt, Live2D model, cue mapping, memory, and conversation history.
- Chat Turn: One assistant response lifecycle from `chat-turn-start` through deltas, tool traces, text completion, translation, and `chat-turn-finish`.
- Turn Event: A Tauri event emitted by the backend during a Chat Turn.
- Tool Trace: User-visible metadata for a tool call, approval request, approval resolution, result, or error.
- MOD Action: A host action requested by a MOD UI through the `kokoro:mod-action` browser event.
- Runtime Setting: A frontend setting currently persisted through localStorage and sometimes mirrored to backend config.
- Provider Config: A backend persisted configuration for LLM, TTS, STT, Vision, ImageGen, Bot, Telegram, or tools.
- Memory: Character-scoped facts stored in SQLite with embedding, keyword retrieval, observability, and dreaming support.
- Dreaming: Background memory consolidation and proposal generation.
- Cue: A semantic Live2D action or expression that can be triggered by chat, tools, MODs, or interaction events.

## Architecture Priorities

- Keep frontend UI modules thin by moving event sequencing and persistence rules into deeper modules.
- Keep `kokoro-bridge.ts` as a typed IPC seam, but protect it with contract checks.
- Keep backend command modules as IPC adapters. Long-lived behavior should live in domain modules behind smaller interfaces.
- Prefer pure reducer or parser modules for complex state transitions, because they are cheaper to test than Tauri or React integration.
```

- [ ] **Step 2: Create ADR directory**

Run:

```powershell
rtk powershell -NoProfile -Command "New-Item -ItemType Directory -Force docs\adr | Out-Null"
```

- [ ] **Step 3: Create ADR 0001**

Write `docs/adr/0001-frontend-runtime-settings-store.md`:

```markdown
# ADR 0001: Frontend Runtime Settings Store

## Status

Accepted

## Context

Runtime settings are currently read and written directly through `localStorage` from `App.tsx`, `SettingsPanel.tsx`, `ChatPanel.tsx`, hooks, and services. Some settings also require custom browser events such as `kokoro-stt-settings-changed` and `kokoro-vision-settings-changed`.

## Decision

Introduce a small frontend settings module that owns localStorage keys, defaults, parsing, writing, and synchronization events. Migrate call sites incrementally.

## Consequences

Callers stop depending on raw storage key names and event names. Tests can verify persistence and event behavior through one interface.
```

- [ ] **Step 4: Create ADR 0002**

Write `docs/adr/0002-chat-turn-state-machine.md`:

```markdown
# ADR 0002: Chat Turn State Machine

## Status

Accepted

## Context

`ChatPanel.tsx` currently understands backend event ordering, streaming reveal, cancellation, tool traces, approval states, vision context, Telegram sync, proactive triggers, and TTS auto-play.

## Decision

Extract pure Chat Turn state transitions into a frontend module. React should wire events and render snapshots, not encode the state machine inline.

## Consequences

Most streaming and tool trace regressions can be tested without mounting React or running Tauri.
```

- [ ] **Step 5: Create ADR 0003**

Write `docs/adr/0003-ipc-contract-check.md`:

```markdown
# ADR 0003: IPC Contract Check

## Status

Accepted

## Context

Rust commands are registered in `src-tauri/src/lib.rs`, while TypeScript invokes commands through `src/lib/kokoro-bridge.ts` and occasional direct `invoke()` calls.

## Decision

Add a lightweight script that checks every TypeScript command invocation is registered by the Rust Tauri handler.

## Consequences

Renamed or removed commands fail in CI before becoming runtime errors.
```

- [ ] **Step 6: Verify**

Run:

```powershell
rtk powershell -NoProfile -Command "Test-Path CONTEXT.md; Test-Path docs\adr\0001-frontend-runtime-settings-store.md; Test-Path docs\adr\0002-chat-turn-state-machine.md; Test-Path docs\adr\0003-ipc-contract-check.md"
```

Expected: four `True` lines.

Suggested commit message if committing:

```text
docs: add architecture context and ADRs
```

---

## Task 2: Add IPC Contract Check

**Goal:** Catch TypeScript calls to unregistered Tauri commands.

**Files:**

- Create: `scripts/check-ipc-contract.mjs`
- Modify: `package.json`
- Optional modify: `.github/workflows/ci.yml`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 2 only. Add the IPC contract checker script, add npm script check:ipc, run it, and optionally wire it into CI if the existing CI layout makes that low risk.
```

- [ ] **Step 1: Add the checker script**

Create `scripts/check-ipc-contract.mjs`:

```javascript
import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const rustLibPath = path.join(root, "src-tauri", "src", "lib.rs");
const srcDir = path.join(root, "src");

function read(filePath) {
  return fs.readFileSync(filePath, "utf8");
}

function walkFiles(dir, result = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkFiles(fullPath, result);
      continue;
    }
    if (/\.(ts|tsx)$/.test(entry.name)) {
      result.push(fullPath);
    }
  }
  return result;
}

function registeredCommands() {
  const rust = read(rustLibPath);
  const match = rust.match(/generate_handler!\s*\[\s*([\s\S]*?)\s*\]\)/);
  if (!match) {
    throw new Error("Could not find tauri::generate_handler! block in src-tauri/src/lib.rs");
  }

  const block = match[1];
  const commands = new Set();
  const commandPattern = /(?:commands::[a-zA-Z0-9_]+::|stt::stream::)([a-zA-Z0-9_]+)/g;
  for (const item of block.matchAll(commandPattern)) {
    commands.add(item[1]);
  }
  return commands;
}

function invokedCommands() {
  const commands = new Map();
  const invokePattern = /\binvoke(?:<[^>]+>)?\(\s*["'`]([a-zA-Z0-9_]+)["'`]/g;
  const safeInvokePattern = /\bsafeInvoke(?:<[^>]+>)?\(\s*["'`]([a-zA-Z0-9_]+)["'`]/g;

  for (const filePath of walkFiles(srcDir)) {
    const text = read(filePath);
    const relative = path.relative(root, filePath).replaceAll("\\", "/");
    for (const pattern of [invokePattern, safeInvokePattern]) {
      for (const item of text.matchAll(pattern)) {
        const command = item[1];
        if (!commands.has(command)) {
          commands.set(command, []);
        }
        commands.get(command).push(relative);
      }
    }
  }
  return commands;
}

const registered = registeredCommands();
const invoked = invokedCommands();
const missing = [...invoked.keys()].filter((command) => !registered.has(command)).sort();

if (missing.length > 0) {
  console.error("TypeScript invokes commands that are not registered in src-tauri/src/lib.rs:");
  for (const command of missing) {
    console.error(`- ${command}: ${invoked.get(command).join(", ")}`);
  }
  process.exit(1);
}

console.log(`IPC contract check passed: ${invoked.size} invoked command names are registered.`);
```

- [ ] **Step 2: Add npm script**

Modify `package.json`:

```json
"scripts": {
  "dev": "vite",
  "build": "tsc && vite build",
  "preview": "vite preview",
  "test": "vitest run",
  "check:ipc": "node scripts/check-ipc-contract.mjs",
  "tauri": "tauri"
}
```

- [ ] **Step 3: Run the checker**

Run:

```powershell
rtk npm run check:ipc
```

Expected: `IPC contract check passed`.

- [ ] **Step 4: Wire into CI if low risk**

Open `.github/workflows/ci.yml`. If there is already an npm install/test block, add:

```yaml
- name: Check IPC contract
  run: npm run check:ipc
```

If the CI file has separate jobs and adding this is unclear, skip CI wiring and mention it in the final response.

- [ ] **Step 5: Verify**

Run:

```powershell
rtk npm run check:ipc
rtk npm test -- --run src/lib/kokoro-bridge.error.test.ts
```

Expected: both pass.

Suggested commit message if committing:

```text
test: add ipc contract check
```

---

## Task 3: Create Frontend Settings Store

**Goal:** Add one module that owns frontend runtime setting keys, defaults, parsing, writing, and sync events.

**Files:**

- Create: `src/lib/app-settings.ts`
- Create: `src/lib/app-settings.test.ts`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 3 only. Create src/lib/app-settings.ts and its tests. Do not migrate call sites yet except imports used by tests.
```

- [ ] **Step 1: Write tests first**

Create `src/lib/app-settings.test.ts`:

```typescript
import { describe, expect, it, vi } from "vitest";
import {
  APP_SETTING_KEYS,
  readBooleanSetting,
  readNumberSetting,
  readStringSetting,
  writeBooleanSetting,
  writeNumberSetting,
  writeStringSetting,
  dispatchRuntimeSettingsChanged,
} from "./app-settings";

function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (key: string) => data.get(key) ?? null,
    key: (index: number) => [...data.keys()][index] ?? null,
    removeItem: (key: string) => {
      data.delete(key);
    },
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
}

describe("app-settings", () => {
  it("reads boolean settings with defaults", () => {
    const storage = memoryStorage();
    expect(readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false, storage)).toBe(false);
    storage.setItem(APP_SETTING_KEYS.ttsEnabled, "true");
    expect(readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false, storage)).toBe(true);
  });

  it("writes string, number, and boolean settings", () => {
    const storage = memoryStorage();
    writeStringSetting(APP_SETTING_KEYS.ttsVoice, "alice", storage);
    writeNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1.25, storage);
    writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, true, storage);

    expect(readStringSetting(APP_SETTING_KEYS.ttsVoice, "", storage)).toBe("alice");
    expect(readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1, storage)).toBe(1.25);
    expect(readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false, storage)).toBe(true);
  });

  it("dispatches stable sync events", () => {
    const target = new EventTarget();
    const vision = vi.fn();
    const stt = vi.fn();
    target.addEventListener("kokoro-vision-settings-changed", vision);
    target.addEventListener("kokoro-stt-settings-changed", stt);

    dispatchRuntimeSettingsChanged("vision", target);
    dispatchRuntimeSettingsChanged("stt", target);

    expect(vision).toHaveBeenCalledTimes(1);
    expect(stt).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: Run test and confirm it fails**

Run:

```powershell
rtk npx vitest run src/lib/app-settings.test.ts
```

Expected: fail because `src/lib/app-settings.ts` does not exist.

- [ ] **Step 3: Add implementation**

Create `src/lib/app-settings.ts`:

```typescript
export const APP_SETTING_KEYS = {
  activeCharacterId: "kokoro_active_character_id",
  appLanguage: "kokoro_app_language",
  bgConfig: "kokoro_bg_config",
  customModelPath: "kokoro_custom_model_path",
  displayMode: "kokoro_display_mode",
  gazeTracking: "kokoro_gaze_tracking",
  persona: "kokoro_persona",
  proactiveEnabled: "kokoro_proactive_enabled",
  responseLanguage: "kokoro_response_language",
  settingsActiveTab: "kokoro_settings_active_tab",
  sttAutoSend: "kokoro_stt_auto_send",
  sttContinuousListening: "kokoro_stt_continuous_listening",
  sttEnabled: "kokoro_stt_enabled",
  ttsEnabled: "kokoro_tts_enabled",
  ttsPitch: "kokoro_tts_pitch",
  ttsProvider: "kokoro_tts_provider",
  ttsSpeed: "kokoro_tts_speed",
  ttsVoice: "kokoro_tts_voice",
  userLanguage: "kokoro_user_language",
  userName: "kokoro_user_name",
  userPersona: "kokoro_user_persona",
  visionConfig: "kokoro_vision_config",
  visionEnabled: "kokoro_vision_enabled",
  voiceInterrupt: "kokoro_voice_interrupt",
  wakeWord: "kokoro_wake_word",
  wakeWordEnabled: "kokoro_wake_word_enabled",
} as const;

export type AppSettingKey = typeof APP_SETTING_KEYS[keyof typeof APP_SETTING_KEYS];
export type RuntimeSettingsKind = "vision" | "stt";

const browserStorage = () => window.localStorage;

export function readStringSetting(
  key: AppSettingKey,
  fallback: string,
  storage: Storage = browserStorage(),
): string {
  return storage.getItem(key) ?? fallback;
}

export function writeStringSetting(
  key: AppSettingKey,
  value: string,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, value);
}

export function readBooleanSetting(
  key: AppSettingKey,
  fallback: boolean,
  storage: Storage = browserStorage(),
): boolean {
  const raw = storage.getItem(key);
  if (raw === null) return fallback;
  return raw === "true";
}

export function writeBooleanSetting(
  key: AppSettingKey,
  value: boolean,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, value ? "true" : "false");
}

export function readNumberSetting(
  key: AppSettingKey,
  fallback: number,
  storage: Storage = browserStorage(),
): number {
  const raw = storage.getItem(key);
  if (raw === null) return fallback;
  const parsed = Number.parseFloat(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

export function writeNumberSetting(
  key: AppSettingKey,
  value: number,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, String(value));
}

export function readJsonSetting<T>(
  key: AppSettingKey,
  fallback: T,
  storage: Storage = browserStorage(),
): T {
  const raw = storage.getItem(key);
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function writeJsonSetting<T>(
  key: AppSettingKey,
  value: T,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, JSON.stringify(value));
}

export function removeSetting(
  key: AppSettingKey,
  storage: Storage = browserStorage(),
): void {
  storage.removeItem(key);
}

export function dispatchRuntimeSettingsChanged(
  kind: RuntimeSettingsKind,
  target: Pick<EventTarget, "dispatchEvent"> = window,
): void {
  const eventName = kind === "vision"
    ? "kokoro-vision-settings-changed"
    : "kokoro-stt-settings-changed";
  target.dispatchEvent(new Event(eventName));
}
```

- [ ] **Step 4: Run tests**

Run:

```powershell
rtk npx vitest run src/lib/app-settings.test.ts
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: add frontend runtime settings module
```

---

## Task 4: Migrate ChatPanel Settings Reads

**Goal:** Reduce raw `localStorage` knowledge inside `ChatPanel.tsx`.

**Depends on:** Task 3.

**Files:**

- Modify: `src/ui/widgets/ChatPanel.tsx`
- Test: `src/ui/widgets/chat-streaming-state.test.ts`
- Test: `src/lib/app-settings.test.ts`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 4 only. Migrate ChatPanel.tsx from direct localStorage reads to src/lib/app-settings.ts for TTS, STT, Vision, active character, and background mode reads. Preserve behavior and run the listed tests.
```

- [ ] **Step 1: Add imports**

In `src/ui/widgets/ChatPanel.tsx`, import the settings helpers:

```typescript
import {
  APP_SETTING_KEYS,
  readBooleanSetting,
  readJsonSetting,
  readNumberSetting,
  readStringSetting,
} from "@/lib/app-settings";
```

Use relative import if this file currently avoids `@/*` imports in the surrounding import block.

- [ ] **Step 2: Replace initialization reads**

Replace these raw reads with helper calls:

```typescript
const [visionEnabled, setVisionEnabled] = useState(() =>
  readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false)
);
const [cameraEnabled, setCameraEnabled] = useState(() =>
  readJsonSetting<{ camera_enabled?: boolean }>(
    APP_SETTING_KEYS.visionConfig,
    {},
  ).camera_enabled === true
);
const [sttEnabled, setSttEnabled] = useState(() =>
  readBooleanSetting(APP_SETTING_KEYS.sttEnabled, false)
);
const [sttAutoSend, setSttAutoSend] = useState(() =>
  readBooleanSetting(APP_SETTING_KEYS.sttAutoSend, false)
);
const [continuousListening, setContinuousListening] = useState(() =>
  readBooleanSetting(APP_SETTING_KEYS.sttContinuousListening, false)
);
const [wakeWordEnabled, setWakeWordEnabled] = useState(() =>
  readBooleanSetting(APP_SETTING_KEYS.wakeWordEnabled, false)
);
const [wakeWord, setWakeWord] = useState(() =>
  readStringSetting(APP_SETTING_KEYS.wakeWord, "")
);
```

- [ ] **Step 3: Replace repeated active character reads**

Use:

```typescript
const getActiveCharacterIdForRequest = () =>
  readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || undefined;
```

Replace `localStorage.getItem("kokoro_active_character_id") || undefined` with `getActiveCharacterIdForRequest()`.

- [ ] **Step 4: Replace TTS playback reads**

Use this helper near the existing TTS auto-speak code:

```typescript
const getTtsPlaybackSettings = () => ({
  enabled: readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false),
  provider_id: readStringSetting(APP_SETTING_KEYS.ttsProvider, "") || undefined,
  voice: readStringSetting(APP_SETTING_KEYS.ttsVoice, "") || undefined,
  speed: readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1.0),
  pitch: readNumberSetting(APP_SETTING_KEYS.ttsPitch, 1.0),
});
```

Then call `synthesize(cleanText.trim(), playback)` only when `playback.enabled` is true.

- [ ] **Step 5: Replace background generated mode reads**

Use:

```typescript
const isGeneratedBackgroundMode = () =>
  readJsonSetting<{ mode?: string }>(APP_SETTING_KEYS.bgConfig, {}).mode === "generated";
```

Replace the repeated inline `JSON.parse(localStorage.getItem("kokoro_bg_config") || "{}")` blocks.

- [ ] **Step 6: Run tests**

Run:

```powershell
rtk npx vitest run src/lib/app-settings.test.ts src/ui/widgets/chat-streaming-state.test.ts
rtk npm run build
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: route chat panel settings through settings module
```

---

## Task 5: Migrate SettingsPanel And App Settings Writes

**Goal:** Centralize setting writes and sync event dispatch for `SettingsPanel.tsx` and the most repetitive `App.tsx` paths.

**Depends on:** Task 3 and preferably Task 4.

**Files:**

- Modify: `src/ui/widgets/SettingsPanel.tsx`
- Modify: `src/App.tsx`
- Test: `src/lib/app-settings.test.ts`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 5 only. Migrate SettingsPanel.tsx and obvious App.tsx setting writes to src/lib/app-settings.ts. Keep behavior identical and do not refactor MOD action dispatch yet.
```

- [ ] **Step 1: Use helper imports**

Add:

```typescript
import {
  APP_SETTING_KEYS,
  dispatchRuntimeSettingsChanged,
  readBooleanSetting,
  readStringSetting,
  writeBooleanSetting,
  writeNumberSetting,
  writeStringSetting,
} from "@/lib/app-settings";
```

Use relative imports if the file's existing style makes that clearer.

- [ ] **Step 2: Replace settings panel initialization**

Replace direct reads for persona, TTS, Vision, voice interrupt, response language, and user language with:

```typescript
readStringSetting(APP_SETTING_KEYS.persona, "You are a friendly, warm companion character. Respond with personality and emotion.")
readStringSetting(APP_SETTING_KEYS.ttsVoice, "")
readStringSetting(APP_SETTING_KEYS.ttsSpeed, "1.0")
readStringSetting(APP_SETTING_KEYS.ttsPitch, "1.0")
readStringSetting(APP_SETTING_KEYS.ttsProvider, "browser")
readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false)
readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false)
readBooleanSetting(APP_SETTING_KEYS.voiceInterrupt, false)
readStringSetting(APP_SETTING_KEYS.responseLanguage, "")
readStringSetting(APP_SETTING_KEYS.userLanguage, "")
```

- [ ] **Step 3: Replace settings panel save writes**

In `handleSave`, replace the corresponding `localStorage.setItem(...)` calls with:

```typescript
writeStringSetting(APP_SETTING_KEYS.persona, persona);
writeStringSetting(APP_SETTING_KEYS.ttsVoice, ttsVoice);
writeStringSetting(APP_SETTING_KEYS.ttsSpeed, ttsSpeed);
writeStringSetting(APP_SETTING_KEYS.ttsPitch, ttsPitch);
writeStringSetting(APP_SETTING_KEYS.ttsProvider, ttsProviderId);
writeBooleanSetting(APP_SETTING_KEYS.ttsEnabled, ttsEnabled);
writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, visionEnabled);
dispatchRuntimeSettingsChanged("vision");
writeBooleanSetting(APP_SETTING_KEYS.voiceInterrupt, voiceInterrupt);
writeStringSetting(APP_SETTING_KEYS.responseLanguage, responseLang);
writeStringSetting(APP_SETTING_KEYS.userLanguage, userLang);
```

For STT writes use:

```typescript
writeBooleanSetting(APP_SETTING_KEYS.sttEnabled, activeSttProvider?.enabled === true);
writeBooleanSetting(APP_SETTING_KEYS.sttAutoSend, localSttConfig.auto_send);
writeStringSetting(APP_SETTING_KEYS.wakeWord, localSttConfig.wake_word || "");
writeBooleanSetting(APP_SETTING_KEYS.wakeWordEnabled, localSttConfig.wake_word_enabled);
dispatchRuntimeSettingsChanged("stt");
```

- [ ] **Step 4: Replace obvious App writes**

In `src/App.tsx`, replace repeated writes for:

```text
kokoro_display_mode
kokoro_custom_model_path
kokoro_gaze_tracking
kokoro_response_language
kokoro_user_language
kokoro_user_name
kokoro_user_persona
kokoro_active_character_id
kokoro_tts_enabled
kokoro_tts_speed
kokoro_tts_pitch
kokoro_tts_voice
kokoro_tts_provider
kokoro_vision_enabled
kokoro_proactive_enabled
```

Use helper functions from `src/lib/app-settings.ts`. Do not move the large `handleModAction` function in this task.

- [ ] **Step 5: Run tests**

Run:

```powershell
rtk npx vitest run src/lib/app-settings.test.ts
rtk npm run build
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: centralize frontend settings writes
```

---

## Task 6: Add MOD Action Dispatcher Seam

**Goal:** Introduce a typed dispatcher for MOD actions without moving the whole `App.tsx` handler yet.

**Files:**

- Create: `src/core/mod-actions/dispatcher.ts`
- Create: `src/core/mod-actions/dispatcher.test.ts`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 6 only. Add a pure MOD action dispatcher module and tests. Do not wire App.tsx yet.
```

- [ ] **Step 1: Write tests first**

Create `src/core/mod-actions/dispatcher.test.ts`:

```typescript
import { describe, expect, it, vi } from "vitest";
import {
  createModActionDispatcher,
  getModActionFromEvent,
  type ModActionEnvelope,
} from "./dispatcher";

describe("mod action dispatcher", () => {
  it("extracts action envelope from CustomEvent detail", () => {
    const event = new CustomEvent("kokoro:mod-action", {
      detail: { action: "close_settings", data: { source: "test" } },
    });

    expect(getModActionFromEvent(event)).toEqual({
      action: "close_settings",
      data: { source: "test" },
    });
  });

  it("ignores malformed events", () => {
    expect(getModActionFromEvent(new Event("kokoro:mod-action"))).toBeNull();
    expect(getModActionFromEvent(new CustomEvent("kokoro:mod-action", { detail: {} }))).toBeNull();
  });

  it("dispatches registered handler and ignores unknown action", async () => {
    const close = vi.fn();
    const dispatcher = createModActionDispatcher({
      close_settings: close,
    });

    await dispatcher({ action: "close_settings" });
    await dispatcher({ action: "unknown_action" });

    expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledWith({ action: "close_settings" });
  });
});
```

- [ ] **Step 2: Run test and confirm it fails**

Run:

```powershell
rtk npx vitest run src/core/mod-actions/dispatcher.test.ts
```

Expected: fail because module does not exist.

- [ ] **Step 3: Add implementation**

Create `src/core/mod-actions/dispatcher.ts`:

```typescript
export interface ModActionEnvelope {
  action: string;
  data?: unknown;
}

export type ModActionHandler = (action: ModActionEnvelope) => void | Promise<void>;
export type ModActionHandlerMap = Record<string, ModActionHandler>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function getModActionFromEvent(event: Event): ModActionEnvelope | null {
  if (!(event instanceof CustomEvent)) return null;
  const detail = event.detail;
  if (!isRecord(detail)) return null;
  if (typeof detail.action !== "string" || detail.action.trim() === "") return null;
  return {
    action: detail.action,
    data: "data" in detail ? detail.data : undefined,
  };
}

export function createModActionDispatcher(handlers: ModActionHandlerMap) {
  return async (action: ModActionEnvelope): Promise<boolean> => {
    const handler = handlers[action.action];
    if (!handler) return false;
    await handler(action);
    return true;
  };
}
```

- [ ] **Step 4: Run tests**

Run:

```powershell
rtk npx vitest run src/core/mod-actions/dispatcher.test.ts
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: add mod action dispatcher
```

---

## Task 7: Move Low-Risk MOD Actions Into Dispatcher

**Goal:** Prove the dispatcher works by moving low-risk actions out of `App.tsx` while leaving complex actions in place.

**Depends on:** Task 6.

**Files:**

- Modify: `src/App.tsx`
- Modify: `src/core/mod-actions/dispatcher.ts` if a small helper is needed
- Test: `src/core/mod-actions/dispatcher.test.ts`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 7 only. Wire the MOD action dispatcher into App.tsx and migrate only close_settings, set_display_mode, set_gaze_tracking, set_render_fps, set_background, set_voice_interrupt, and set_vision_enabled. Leave complex actions in the existing handler.
```

- [ ] **Step 1: Import dispatcher**

In `src/App.tsx`, add:

```typescript
import {
  createModActionDispatcher,
  getModActionFromEvent,
  type ModActionEnvelope,
} from "./core/mod-actions/dispatcher";
```

- [ ] **Step 2: Create a small dispatched action group**

Near `handleModAction`, create:

```typescript
const dispatchSimpleModAction = createModActionDispatcher({
  close_settings: () => {
    setSettingsOpen(false);
  },
  set_display_mode: ({ data }) => {
    if (data && typeof data === "object" && "mode" in data) {
      handleDisplayModeChange(String((data as { mode: unknown }).mode));
    }
  },
  set_gaze_tracking: ({ data }) => {
    const enabled = Boolean((data as { enabled?: unknown } | undefined)?.enabled);
    handleGazeTrackingChange(enabled);
  },
  set_render_fps: ({ data }) => {
    const raw = (data as { fps?: unknown } | undefined)?.fps;
    const fps = Number.parseInt(String(raw), 10);
    if (Number.isFinite(fps)) {
      void handleRenderFpsChange(Math.max(0, fps));
    }
  },
  set_background: ({ data }) => {
    const url = (data as { url?: unknown } | undefined)?.url;
    if (typeof url === "string" && url) {
      setGeneratedImage(url);
      bgSlideshow.setConfig({ mode: "generated" });
    }
  },
  set_voice_interrupt: ({ data }) => {
    setVoiceInterrupt(Boolean((data as { enabled?: unknown } | undefined)?.enabled));
  },
  set_vision_enabled: ({ data }) => {
    const enabled = Boolean((data as { enabled?: unknown } | undefined)?.enabled);
    writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, enabled);
    dispatchRuntimeSettingsChanged("vision");
  },
});
```

If Task 5 has not been completed, use the existing `localStorage.setItem` and `window.dispatchEvent` only for `set_vision_enabled`.

- [ ] **Step 3: Use dispatcher at top of handler**

Change the first lines of `handleModAction`:

```typescript
const handleModAction = (e: Event) => {
  const action = getModActionFromEvent(e);
  if (!action) return;

  void dispatchSimpleModAction(action).then((handled) => {
    if (handled) return;
    handleLegacyModAction(action);
  });
};
```

Then wrap the remaining old logic in:

```typescript
const handleLegacyModAction = (detail: ModActionEnvelope) => {
  // Existing logic that still checks detail.action and detail.data.
};
```

Keep the old checks for the migrated actions removed from `handleLegacyModAction`.

- [ ] **Step 4: Run tests**

Run:

```powershell
rtk npx vitest run src/core/mod-actions/dispatcher.test.ts
rtk npm run build
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: route simple mod actions through dispatcher
```

---

## Task 8: Extract Frontend Chat Turn Reducer

**Goal:** Move pure Chat Turn message updates out of `ChatPanel.tsx` into a testable module.

**Files:**

- Create: `src/ui/widgets/chat/turn-state.ts`
- Create: `src/ui/widgets/chat/turn-state.test.ts`
- Modify: `src/ui/widgets/ChatPanel.tsx`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 8 only. Extract pure chat turn state helpers from ChatPanel.tsx into src/ui/widgets/chat/turn-state.ts with tests. Keep event listener wiring inside ChatPanel.
```

- [ ] **Step 1: Create reducer tests**

Create `src/ui/widgets/chat/turn-state.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import {
  ensureTurnMessage,
  stripStreamingMarkup,
  updateTurnMessage,
  type ChatPanelMessage,
  type PendingTurnState,
} from "./turn-state";

function turn(overrides: Partial<PendingTurnState> = {}): PendingTurnState {
  return {
    turnId: "turn-1",
    messageIndex: null,
    rawText: "",
    visibleTextStarted: false,
    translationPending: false,
    tools: [],
    ...overrides,
  };
}

describe("chat turn state", () => {
  it("strips streamed control markup", () => {
    expect(stripStreamingMarkup("hello[TOOL_CALL:get_time|{}]world")).toBe("helloworld");
    expect(stripStreamingMarkup("hello[TRANSLATE:你好]")).toBe("hello");
  });

  it("creates one assistant message for a turn", () => {
    const state = turn();
    const messages = ensureTurnMessage([], state);
    expect(messages).toEqual([{ role: "kokoro", text: "", turnId: "turn-1" }]);
    expect(state.messageIndex).toBe(0);
  });

  it("updates the active assistant message", () => {
    const messages: ChatPanelMessage[] = [{ role: "kokoro", text: "", turnId: "turn-1" }];
    const state = turn({ messageIndex: 0 });
    const next = updateTurnMessage(messages, state, (message) => ({ ...message, text: "hello" }));
    expect(next[0]?.text).toBe("hello");
  });
});
```

- [ ] **Step 2: Run test and confirm it fails**

Run:

```powershell
rtk npx vitest run src/ui/widgets/chat/turn-state.test.ts
```

Expected: fail because module does not exist.

- [ ] **Step 3: Move pure types and helpers**

Create `src/ui/widgets/chat/turn-state.ts` and move these definitions from `ChatPanel.tsx`:

```typescript
import type { ToolTraceItem } from "@/lib/kokoro-bridge";

export interface ChatPanelMessage {
  role: "user" | "kokoro" | "context";
  text: string;
  images?: string[];
  translation?: string;
  translationPending?: boolean;
  tools?: ToolTraceItem[];
  isError?: boolean;
  turnId?: string;
  capturedAt?: string;
  source?: string;
}

export interface PendingTurnState {
  turnId: string;
  messageIndex: number | null;
  rawText: string;
  visibleTextStarted: boolean;
  translation?: string;
  translationPending: boolean;
  tools: ToolTraceItem[];
  pendingContext?: ChatPanelMessage;
}
```

Move the existing helper functions without changing their behavior:

```text
stripStreamingMarkup
stripStoredMarkup
ensureTurnMessage
updateTurnMessage
mergeToolTraceItems
buildToolTraceItem
removePendingApprovalHint
createRejectedToolTrace
createApprovedToolTrace
filterVisibleTools
normalizeToolList
hasRenderableTurnContent
removeTurnContext
removeTurnMessages
mergeToolIntoTurn
updateTurnToolsInMessages
getToolEventStateUpdate
```

Export the functions used by `ChatPanel.tsx`.

- [ ] **Step 4: Update ChatPanel imports**

In `ChatPanel.tsx`, import moved functions from `./chat/turn-state`. Delete duplicate local definitions after the build passes.

- [ ] **Step 5: Run tests**

Run:

```powershell
rtk npx vitest run src/ui/widgets/chat/turn-state.test.ts src/ui/widgets/chat-streaming-state.test.ts
rtk npm run build
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: extract chat turn state helpers
```

---

## Task 9: Extract Backend Chat Tag Parser

**Goal:** Move control tag parsing out of `commands/chat.rs` into a reusable Rust module.

**Files:**

- Create: `src-tauri/src/chat/mod.rs`
- Create: `src-tauri/src/chat/tags.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/chat.rs`

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 9 only. Extract ToolCall and chat tag parsing helpers from src-tauri/src/commands/chat.rs into src-tauri/src/chat/tags.rs. Do not change parsing behavior.
```

- [ ] **Step 1: Create module shell**

Create `src-tauri/src/chat/mod.rs`:

```rust
pub mod tags;
```

In `src-tauri/src/lib.rs`, add the module declaration near the other `pub mod` declarations:

```rust
pub mod chat;
```

- [ ] **Step 2: Move parser code**

Move these items from `src-tauri/src/commands/chat.rs` into `src-tauri/src/chat/tags.rs`:

```text
ToolCall
tool_call_fingerprint
merge_round_tool_calls
parse_tool_call_tags
strip_translate_tags
extract_translate_tags
strip_leaked_tags
merge_continuation_text
find_safe_emit_boundary
```

Make `ToolCall` public if `commands/chat.rs` still needs it:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub args: std::collections::HashMap<String, String>,
}
```

Export parser functions with `pub(crate)` unless TypeScript or other crates need them.

- [ ] **Step 3: Update chat command imports**

In `src-tauri/src/commands/chat.rs`, import:

```rust
use crate::chat::tags::{
    extract_translate_tags,
    find_safe_emit_boundary,
    merge_continuation_text,
    merge_round_tool_calls,
    parse_tool_call_tags,
    strip_leaked_tags,
    strip_translate_tags,
    ToolCall,
};
```

Remove the moved definitions from `commands/chat.rs`.

- [ ] **Step 4: Move existing parser tests**

Move the parser-focused tests from `commands/chat.rs` into `chat/tags.rs` under `#[cfg(test)]`. Include at least:

```text
test_extract_translate_tags_basic
test_extract_translate_tags_none
test_extract_translate_tags_multiple
test_extract_translate_tags_unclosed
test_strip_translate_tags
test_strip_leaked_tags_removes_tool_result
test_safe_emit_boundary_partial_tool_call
test_parse_tool_call_basic
test_parse_tool_call_simplified_format
test_merge_round_tool_calls_deduplicates_matching_textual_calls
```

- [ ] **Step 5: Run tests**

Run:

```powershell
rtk cargo test --manifest-path src-tauri/Cargo.toml chat::tags
rtk cargo test --manifest-path src-tauri/Cargo.toml commands::chat
```

Expected: pass.

Suggested commit message if committing:

```text
refactor: extract chat tag parser
```

---

## Task 10: Split Memory Embedding Model Lifecycle

**Goal:** Reduce `ai/memory.rs` size by moving embedding model status and download lifecycle into its own module while keeping `MemoryManager` public behavior unchanged.

**Files:**

- Create: `src-tauri/src/ai/memory_embedding_model.rs`
- Modify: `src-tauri/src/ai/mod.rs`
- Modify: `src-tauri/src/ai/memory.rs`
- Modify: `src-tauri/src/commands/memory.rs` if imports need updating

**Prompt for new conversation:**

```text
Read docs/superpowers/plans/2026-06-17-kokoro-architecture-optimization.md and execute Task 10 only. Move memory embedding model status/download path helpers from ai/memory.rs into ai/memory_embedding_model.rs. Preserve public function names through re-exports if needed.
```

- [ ] **Step 1: Identify exact functions to move**

Use:

```powershell
rtk rg -n "memory_embedding_model_status|download_memory_embedding_model|memory_model_endpoint_candidates|memory_model_file_url|MODEL_REF_NAME|MODEL_FILE" src-tauri/src/ai/memory.rs src-tauri/src/commands/memory.rs
```

Move only embedding model lifecycle code. Do not move retrieval, writing, dreaming, or consolidation logic in this task.

- [ ] **Step 2: Create module and re-export**

Create `src-tauri/src/ai/memory_embedding_model.rs` with moved constants, structs, helpers, and tests.

In `src-tauri/src/ai/mod.rs`, add:

```rust
pub mod memory_embedding_model;
```

In `src-tauri/src/ai/memory.rs`, re-export functions used by commands so call sites can remain stable:

```rust
pub use crate::ai::memory_embedding_model::{
    download_memory_embedding_model,
    memory_embedding_model_status,
    MemoryEmbeddingModelDownloadProgress,
    MemoryEmbeddingModelStatus,
};
```

- [ ] **Step 3: Move tests with the code**

Move these tests from `ai/memory.rs` to `ai/memory_embedding_model.rs`:

```text
memory_model_endpoint_candidates_adds_hf_mirror_fallback
memory_model_endpoint_candidates_deduplicates_hf_mirror
memory_model_file_url_uses_original_repo_layout_for_mirror
download_memory_model_file_retries_mirror_after_primary_failure
```

- [ ] **Step 4: Run targeted tests**

Run:

```powershell
rtk cargo test --manifest-path src-tauri/Cargo.toml memory_embedding_model
rtk cargo test --manifest-path src-tauri/Cargo.toml ai::memory
rtk cargo test --manifest-path src-tauri/Cargo.toml commands::memory
```

Expected: pass.

- [ ] **Step 5: Check formatting**

Run:

```powershell
rtk cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Expected: pass. If formatting fails, run:

```powershell
rtk cargo fmt --manifest-path src-tauri/Cargo.toml
```

Suggested commit message if committing:

```text
refactor: split memory embedding model lifecycle
```

---

## Later Candidates

Do these after the first 10 tasks, because they touch broader behavior:

- Extract `src-tauri/src/chat/tool_trace.rs` from `commands/chat.rs` payload builders.
- Migrate `src-tauri/src/commands/bot.rs` to reuse `crate::chat::tags` if the bot parser is intentionally identical to desktop chat.
- Split `src-tauri/src/ai/memory_dreaming.rs` from `ai/memory.rs`.
- Split `src/ui/widgets/SettingsPanel.tsx` into a controller hook and pure tab shell after settings store migration.
- Move direct Tauri `invoke()` calls in UI modules behind `kokoro-bridge.ts` wrappers, then strengthen the IPC contract check.

## Completion Checklist

- [ ] `npm test` passes.
- [ ] `npm run build` passes.
- [ ] `npm run check:ipc` passes after Task 2.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes for touched Rust areas.
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings` passes before review.
- [ ] No unrelated user changes were reverted.
- [ ] Final response lists changed files and tests run.
