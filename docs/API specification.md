# Kokoro Engine — API Specification

> **Version:** 1.1
> **Last Updated:** 2026-03-01
> **Transport:** Tauri IPC (`invoke` + event system)  
> **Companion:** [architecture.md](file:///d:/Program/Kokoro%20Engine/docs/architecture.md)

---

## Table of Contents

1. [Overview](#1-overview)
2. [Data Types](#2-data-types)
3. [System Commands](#3-system-commands)
4. [Character Commands](#4-character-commands)
5. [Chat Commands](#5-chat-commands)
6. [Context Management](#6-context-management)
7. [Database Commands](#7-database-commands)
8. [TTS Commands](#8-tts-commands)
9. [STT Commands](#9-stt-commands)
10. [LLM Commands](#10-llm-commands)
11. [Vision Commands](#11-vision-commands)
12. [ImageGen Commands](#12-imagegen-commands)
13. [Memory Commands](#13-memory-commands)
14. [Live2D Commands](#14-live2d-commands)
15. [MCP Commands](#15-mcp-commands)
16. [Mod System Commands](#16-mod-system-commands)
17. [Telegram Commands](#17-telegram-commands)
18. [Action Commands](#18-action-commands)
19. [Events](#19-events)
20. [Custom Protocols](#20-custom-protocols)
21. [Error Handling](#21-error-handling)
22. [Frontend Bridge Reference](#22-frontend-bridge-reference)

---

## 1. Overview

All API communication uses **Tauri's IPC** mechanism:

- **Commands** — Request/response via `invoke("command_name", { args })`. The frontend calls, the backend responds.
- **Events** — Push-based via `emit()` / `listen()`. The backend pushes data to the frontend asynchronously.
- **Custom protocols** — URI-based asset serving (e.g., `mod://`).

```
Frontend                          Backend
────────                          ───────
invoke("command", args)  ──────▶  #[tauri::command] fn handler()
                         ◀──────  Result<T, String>

listen("event-name")     ◀──────  window.emit("event-name", payload)
```

---

## 2. Data Types

### 2.1 Backend Types (Rust)

#### `EngineInfo`

```rust
pub struct EngineInfo {
    pub name: String,       // "Kokoro Engine"
    pub version: String,    // from Cargo.toml
    pub platform: String,   // "windows" | "macos" | "linux"
}
```

#### `SystemStatus`

```rust
pub struct SystemStatus {
    pub engine_running: bool,
    pub active_modules: Vec<String>,   // e.g. ["core", "ui"]
    pub memory_usage_mb: f64,
}
```

#### `CharacterState`

```rust
pub struct CharacterState {
    pub name: String,                // "Kokoro"
    pub current_expression: String,  // "neutral" | "happy" | "sad" | ...
    pub mood: f32,                   // 0.0 – 1.0
    pub is_speaking: bool,
}
```

#### `ChatResponse`

```rust
pub struct ChatResponse {
    pub text: String,           // AI reply text
    pub expression: String,     // Suggested expression
    pub mood_delta: f32,        // Mood change (-1.0 to 1.0)
}
```

#### `ChatRequest`

```rust
pub struct ChatRequest {
    pub message: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,   // Custom OpenAI-compatible endpoint URL
    pub model: Option<String>,      // e.g. "gpt-4o-mini"
}
```

#### `TtsConfig`

```rust
pub struct TtsConfig {
    pub provider_id: Option<String>,  // e.g. "openai"
    pub voice: Option<String>,        // e.g. "alloy", "nova"
    pub speed: Option<f32>,           // default: 1.0
    pub pitch: Option<f32>,           // default: 1.0
    pub emotion: Option<String>,      // e.g. "cheerful", "sad"
}
```

#### `TtsParams`

```rust
pub struct TtsParams {
    pub voice: Option<String>,
    pub speed: Option<f32>,    // default: 1.0
    pub pitch: Option<f32>,    // default: 1.0
    pub emotion: Option<String>,
}
```

#### `DbTestResult`

```rust
pub struct DbTestResult {
    pub success: bool,
    pub message: String,
    pub record_count: usize,
}
```

#### `ModManifest`

```rust
pub struct ModManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub permissions: Vec<String>,
    pub entry: Option<String>,      // JS entry point
    pub ui_entry: Option<String>,   // HTML UI entry point
}
```

### 2.2 Frontend Types (TypeScript)

Mirrored in [`kokoro-bridge.ts`](file:///d:/Program/Kokoro%20Engine/src/lib/kokoro-bridge.ts):

```typescript
interface EngineInfo      { name: string; version: string; platform: string }
interface SystemStatus    { engine_running: boolean; active_modules: string[]; memory_usage_mb: number }
interface CharacterState  { name: string; current_expression: string; mood: number; is_speaking: boolean }
interface ChatResponse    { text: string; expression: string; mood_delta: number }
interface ChatRequest     { message: string; api_key?: string; endpoint?: string; model?: string }
interface DbTestResult    { success: boolean; message: string; record_count: number }
```

Additional types in [`core/types/mod.ts`](file:///d:/Program/Kokoro%20Engine/src/core/types/mod.ts):

```typescript
interface TtsConfig       { api_key?: string; endpoint?: string; model?: string; voice?: string }
interface CharacterConfig { model_path: string; system_prompt: string; tts: TtsConfig }
interface ModThemeConfig  { primary_color: string; background: string }
interface ModManifest     { id: string; name: string; version: string; description: string;
                           permissions?: string[]; entry?: string; ui_entry?: string;
                           character?: CharacterConfig; theme?: ModThemeConfig }
```

---

## 3. System Commands

### `get_engine_info`

Returns engine metadata.

| Property | Value |
|---|---|
| **Command** | `get_engine_info` |
| **Parameters** | *none* |
| **Returns** | `EngineInfo` |
| **Errors** | *none* |
| **Source** | [system.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/system.rs#L18-L25) |

**Example:**

```typescript
const info = await getEngineInfo();
// { name: "Kokoro Engine", version: "0.1.0", platform: "windows" }
```

---

### `get_system_status`

Returns runtime status and active modules.

| Property | Value |
|---|---|
| **Command** | `get_system_status` |
| **Parameters** | *none* |
| **Returns** | `SystemStatus` |
| **Errors** | *none* |
| **Source** | [system.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/system.rs#L28-L38) |

**Example:**

```typescript
const status = await getSystemStatus();
// { engine_running: true, active_modules: ["core", "ui"], memory_usage_mb: 0.0 }
```

---

## 4. Character Commands

### `get_character_state`

Returns the current character state for Live2D synchronization.

| Property | Value |
|---|---|
| **Command** | `get_character_state` |
| **Parameters** | *none* |
| **Returns** | `CharacterState` |
| **Errors** | *none* |
| **Source** | [character.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/character.rs#L19-L28) |

**Example:**

```typescript
const state = await getCharacterState();
// { name: "Kokoro", current_expression: "neutral", mood: 0.5, is_speaking: false }
```

---

### `set_expression`

Sets the character's current expression and returns the updated state.

| Property | Value |
|---|---|
| **Command** | `set_expression` |
| **Parameters** | `expression: string` |
| **Returns** | `CharacterState` |
| **Errors** | *none* |
| **Source** | [character.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/character.rs#L31-L40) |

**Example:**

```typescript
const state = await setExpression("happy");
// { name: "Kokoro", current_expression: "happy", mood: 0.5, is_speaking: false }
```

---

### `send_message`

Sends a message and returns a non-streaming AI response. *(Legacy — prefer `stream_chat` for real-time responses.)*

| Property | Value |
|---|---|
| **Command** | `send_message` |
| **Parameters** | `message: string` |
| **Returns** | `Result<ChatResponse, string>` |
| **Errors** | `"Message cannot be empty"` if input is blank |
| **Source** | [character.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/character.rs#L44-L56) |

**Example:**

```typescript
const response = await sendMessage("Hello!");
// { text: "Echo from Kokoro Engine: Hello!", expression: "happy", mood_delta: 0.1 }
```

---

## 5. Chat Commands

### `stream_chat`

Initiates a streaming chat session. The response is delivered incrementally via events.

| Property | Value |
|---|---|
| **Command** | `stream_chat` |
| **Parameters** | `request: ChatRequest` |
| **Returns** | `Result<void, string>` |
| **Errors** | `"API Key is required"` if `api_key` is missing |
| **Events emitted** | `chat-delta`, `chat-error`, `chat-done` |
| **Source** | [chat.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/chat.rs#L14-L72) |

**Internal flow:**

1. Adds user message to conversation history
2. Assembles prompt via `AIOrchestrator.compose_prompt()`
3. Sends to LLM via `OpenAIClient.chat_stream()`
4. Emits `chat-delta` for each SSE chunk
5. Emits `chat-error` on any stream error
6. Adds full response to history
7. Emits `chat-done` when complete

**Example:**

```typescript
// 1. Register event listeners
const offDelta = await onChatDelta((text) => appendToUI(text));
const offError = await onChatError((err) => showError(err));
const offDone  = await onChatDone(() => finalize());

// 2. Start streaming
await streamChat({
  message: "Tell me a story",
  api_key: "sk-...",
  endpoint: "https://api.openai.com/v1",
  model: "gpt-4o-mini"
});

// 3. Clean up listeners when done
offDelta(); offError(); offDone();
```

---

## 6. Context Management

### `set_persona`

Sets the character's system prompt (personality card).

| Property | Value |
|---|---|
| **Command** | `set_persona` |
| **Parameters** | `prompt: string` |
| **Returns** | `Result<void, string>` |
| **Errors** | *none* |
| **Source** | [context.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/context.rs#L4-L11) |

**Example:**

```typescript
await setPersona("You are Kokoro, a cheerful virtual companion who loves anime.");
```

---

### `clear_history`

Clears the conversation history (rolling window).

| Property | Value |
|---|---|
| **Command** | `clear_history` |
| **Parameters** | *none* |
| **Returns** | `Result<void, string>` |
| **Errors** | *none* |
| **Source** | [context.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/context.rs#L13-L19) |

**Example:**

```typescript
await clearHistory();
```

---

## 7. Database Commands

### `init_db`

Initializes the SQLite database. The database is primarily managed by the `AIOrchestrator`.

| Property | Value |
|---|---|
| **Command** | `init_db` |
| **Parameters** | *none* (requires `AIOrchestrator` state) |
| **Returns** | `Result<string, string>` |
| **Errors** | *none* |
| **Source** | [database.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/database.rs#L11-L16) |

---

### `test_vector_store`

Tests the vector store by inserting a test memory and performing a semantic search.

| Property | Value |
|---|---|
| **Command** | `test_vector_store` |
| **Parameters** | *none* (requires `AIOrchestrator` state) |
| **Returns** | `Result<DbTestResult, string>` |
| **Errors** | Database write/read failures |
| **Source** | [database.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/database.rs#L18-L38) |

**Example:**

```typescript
const result = await testVectorStore();
// { success: true, message: "Found: Test memory: Kokoro loves apples.", record_count: 1 }
```

---

## 8. TTS Commands

### `synthesize`

Synthesizes text to speech and streams audio chunks via events.

| Property | Value |
|---|---|
| **Command** | `synthesize` |
| **Parameters** | `text: string`, `config: TtsConfig` |
| **Returns** | `Result<void, string>` |
| **Errors** | `"No TTS provider available"`, `"Provider {id} not found"` |
| **Events emitted** | `tts:start`, `tts:audio`, `tts:end` |
| **Source** | [tts.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/tts.rs#L13-L28) |

**Internal flow:**

1. Resolves the provider (by `provider_id` or falls back to default)
2. Emits `tts:start` with the input text
3. Splits text into sentences (by `.` `!` `?`)
4. For each sentence, synthesizes audio and emits `tts:audio` with raw bytes
5. Emits `tts:end` when all sentences are processed

**Example:**

```typescript
await synthesize("Hello! How are you today?", {
  provider_id: "openai",
  voice: "nova",
  speed: 1.0
});
```

---

## 9. Mod System Commands

### `list_mods`

Scans the `mods/` directory and returns all discovered mod manifests.

| Property | Value |
|---|---|
| **Command** | `list_mods` |
| **Parameters** | *none* |
| **Returns** | `Result<ModManifest[], string>` |
| **Errors** | *none* |
| **Source** | [mods.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/mods.rs#L5-L9) |

**Example:**

```typescript
const mods = await listMods();
// [{ id: "example-mod", name: "Example Mod", version: "0.1.0", ... }]
```

---

### `load_mod`

Loads and activates a mod by its ID.

| Property | Value |
|---|---|
| **Command** | `load_mod` |
| **Parameters** | `mod_id: string` |
| **Returns** | `Result<void, string>` |
| **Errors** | Mod not found, load failure |
| **Source** | [mods.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/commands/mods.rs#L11-L15) |

**Example:**

```typescript
await loadMod("example-mod");
```

---

## 10. Events

Events are push-based messages from the backend to the frontend. Subscribe via `listen()`.

### Chat Events

| Event | Payload | Description |
|---|---|---|
| `chat-delta` | `string` | Incremental text chunk from LLM streaming |
| `chat-error` | `string` | Error message during streaming |
| `chat-done` | `void` | Stream completed successfully |

### TTS Events

| Event | Payload | Description |
|---|---|---|
| `tts:start` | `{ text: string }` | Synthesis has started for the given text |
| `tts:audio` | `{ data: number[] }` | Raw audio bytes for one sentence chunk |
| `tts:end` | `{ text: string }` | All sentences synthesized and delivered |

### Frontend Event Subscription

```typescript
import { listen } from "@tauri-apps/api/event";

// Chat events
const off = await listen<string>("chat-delta", (event) => {
  console.log("Chunk:", event.payload);
});

// TTS events
await listen<{ data: number[] }>("tts:audio", (event) => {
  playAudioChunk(new Uint8Array(event.payload.data));
});

// Unsubscribe
off();
```

---

## 11. Custom Protocols

### `mod://` Protocol

Serves static assets from installed mods.

| Property | Value |
|---|---|
| **Scheme** | `mod://` |
| **Base path** | `mods/` directory |
| **Source** | [protocol.rs](file:///d:/Program/Kokoro%20Engine/src-tauri/src/mods/protocol.rs) |

**Security:**

- Path traversal (`..`) is blocked → `403 Forbidden`
- Only serves files within `mods/` directory

**MIME types:**

| Extension | Content-Type |
|---|---|
| `.html` | `text/html` |
| `.js` | `text/javascript` |
| `.css` | `text/css` |
| `.json` | `application/json` |
| `.png` | `image/png` |
| `.jpg` / `.jpeg` | `image/jpeg` |
| *other* | `application/octet-stream` |

**Response codes:**

| Code | Condition |
|---|---|
| `200` | File served successfully |
| `403` | Path traversal detected |
| `404` | File not found |
| `500` | File read error |

**Example:**

```html
<!-- In a mod's UI -->
<img src="mod://example-mod/assets/icon.png" />
<script src="mod://example-mod/index.js"></script>
```

---

## 12. Error Handling

All commands return `Result<T, String>`. Errors are plain string messages.

### Error Catalog

| Command | Error | Condition |
|---|---|---|
| `send_message` | `"Message cannot be empty"` | Empty or whitespace-only input |
| `stream_chat` | `"API Key is required"` | `api_key` is `None` |
| `synthesize` | `"No TTS provider available"` | No providers registered |
| `synthesize` | `"Provider {id} not found"` | Specified `provider_id` doesn't exist |
| `test_vector_store` | *(database error)* | SQLite write/read failure |

### Frontend Error Handling

```typescript
try {
  await streamChat({ message: "Hello", api_key: key });
} catch (error) {
  // error is a string from the Rust Result::Err
  console.error("Command failed:", error);
}
```

---

## 13. Frontend Bridge Reference

All commands are accessed through [`kokoro-bridge.ts`](file:///d:/Program/Kokoro%20Engine/src/lib/kokoro-bridge.ts) which provides typed wrappers.

### Quick Reference

```typescript
// System
getEngineInfo():                             Promise<EngineInfo>
getSystemStatus():                           Promise<SystemStatus>

// Character
getCharacterState():                         Promise<CharacterState>
setExpression(expression: string):           Promise<CharacterState>
sendMessage(message: string):                Promise<ChatResponse>

// Chat (streaming)
streamChat(request: ChatRequest):            Promise<void>
onChatDelta(cb: (delta: string) => void):    Promise<UnlistenFn>
onChatError(cb: (error: string) => void):    Promise<UnlistenFn>
onChatDone(cb: () => void):                  Promise<UnlistenFn>
onChatTranslation(cb):                       Promise<UnlistenFn>
onChatExpression(cb):                        Promise<UnlistenFn>

// Context
setPersona(prompt: string):                  Promise<void>
setCharacterName(name: string):              Promise<void>
setUserName(name: string):                   Promise<void>
setResponseLanguage(lang: string):           Promise<void>
setUserLanguage(lang: string):               Promise<void>
setJailbreakPrompt(prompt: string):          Promise<void>
getJailbreakPrompt():                        Promise<string>
setProactiveEnabled(enabled: boolean):       Promise<void>
getProactiveEnabled():                       Promise<boolean>
clearHistory():                              Promise<void>
deleteLastMessages(count: number):           Promise<void>
endSession():                                Promise<void>
getEmotionState():                           Promise<EmotionStateResponse>

// Database
initDb():                                    Promise<string>
testVectorStore():                           Promise<DbTestResult>

// TTS
synthesize(text, config):                    Promise<Uint8Array>
getTtsConfig():                              Promise<TtsConfig>
saveTtsConfig(config):                       Promise<void>

// STT
transcribeAudio(data, config):               Promise<string>
getSttConfig():                              Promise<SttConfig>
saveSttConfig(config):                       Promise<void>

// LLM
getLlmConfig():                              Promise<LlmConfig>
saveLlmConfig(config):                       Promise<void>
listOllamaModels(baseUrl):                   Promise<OllamaModelInfo[]>
fetchModels(baseUrl, apiKey):                Promise<string[]>

// Vision
captureScreen():                             Promise<string>

// ImageGen
generateImage(prompt, width, height):        Promise<ImageGenResult>
getImagegenConfig():                         Promise<ImageGenSystemConfig>
saveImagegenConfig(config):                  Promise<void>

// Memory
listMemories(characterId, opts):             Promise<Memory[]>
updateMemory(id, updates):                   Promise<void>
deleteMemory(id):                            Promise<void>
updateMemoryTier(id, tier):                  Promise<void>

// Live2D
listLive2dModels():                          Promise<Live2dModelInfo[]>
importLive2dZip(zipPath):                    Promise<string>
importLive2dFolder(folderPath):              Promise<string>
deleteLive2dModel(modelName):                Promise<void>

// MCP
listMcpServers():                            Promise<McpServerConfig[]>
addMcpServer(config):                        Promise<void>
removeMcpServer(id):                         Promise<void>
refreshMcpTools(id):                         Promise<void>
reconnectMcpServer(id):                      Promise<void>

// Mods
listMods():                                  Promise<ModManifest[]>
loadMod(modId):                              Promise<ModManifest>
installMod(zipPath):                         Promise<void>
unloadMod(modId):                            Promise<void>

// Telegram
getTelegramConfig():                         Promise<TelegramConfig>
saveTelegramConfig(config):                  Promise<void>
startTelegramBot():                          Promise<void>
stopTelegramBot():                           Promise<void>
getTelegramStatus():                         Promise<TelegramStatus>

// Actions
listActions():                               Promise<ActionInfo[]>
executeAction(name, args):                   Promise<string>

// Singing (RVC)
checkRvcStatus():                            Promise<boolean>
listRvcModels():                             Promise<RvcModelInfo[]>
convertSinging(params):                      Promise<Uint8Array>
```
