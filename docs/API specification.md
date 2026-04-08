# Kokoro Engine — API specification

> **Version:** 2.0
> **Last updated:** 2026-04-09
> **Transport:** Tauri IPC (`invoke`) + Tauri events (`emit` / `listen`)
> **Source of truth:** `src-tauri/src/lib.rs` for registered commands, `src/lib/kokoro-bridge.ts` for frontend bridge wrappers
> **Related doc:** [architecture.md](file:///d:/Program/Kokoro%20Engine/docs/architecture.md)

---

## Table of contents

1. [Scope](#scope)
2. [Calling convention](#calling-convention)
3. [Data types](#data-types)
4. [Command reference](#command-reference)
5. [Event reference](#event-reference)
6. [Custom protocols](#custom-protocols)
7. [Error handling](#error-handling)
8. [Bridge reference](#bridge-reference)
9. [Compatibility notes](#compatibility-notes)

---

## Scope

This document describes the current IPC surface of Kokoro Engine.

It covers:

- commands registered in `src-tauri/src/lib.rs`
- bridge wrappers exported from `src/lib/kokoro-bridge.ts`
- events emitted by the backend and consumed by the frontend
- custom URI schemes used by MODs and Live2D

It does not try to explain internal architecture. Use `architecture.md` for that.

---

## Calling convention

### Commands

Frontend code calls backend commands with `invoke`:

```ts
import { invoke } from "@tauri-apps/api/core";

const info = await invoke("get_engine_info");
```

Most commands return `Result<T, String>` at the IPC boundary.

### Events

Backend code pushes events with `emit`. Frontend code listens with `listen`.

```ts
import { listen } from "@tauri-apps/api/event";

const off = await listen("chat-turn-delta", (event) => {
  console.log(event.payload);
});
```

### Naming

- Rust command names use `snake_case`.
- Bridge wrappers use `camelCase`.
- Event names stay in the backend string form, such as `chat-turn-delta`.

---

## Data types

This section lists the types that are part of the public bridge surface.

### `EngineInfo`

```ts
interface EngineInfo {
  name: string;
  version: string;
  platform: string;
}
```

### `SystemStatus`

```ts
interface SystemStatus {
  engine_running: boolean;
  active_modules: string[];
  memory_usage_mb: number;
}
```

### `CharacterState`

```ts
interface CharacterState {
  name: string;
  current_cue: string;
  mood: number;
  is_speaking: boolean;
}
```

### `ChatResponse`

```ts
interface ChatResponse {
  text: string;
  cue: string;
  mood_delta: number;
}
```

### `ChatRequest`

```ts
interface ChatRequest {
  message: string;
  api_key?: string;
  endpoint?: string;
  model?: string;
  allow_image_gen?: boolean;
  images?: string[];
  character_id?: string;
  hidden?: boolean;
}
```

### `ContextSettings`

```ts
interface ContextSettings {
  strategy: "window" | "summary";
  max_message_chars: number;
}
```

### `LlmConfig`

```ts
interface LlmConfig {
  active_provider: string;
  system_provider?: string;
  system_model?: string;
  providers: LlmProviderConfig[];
  presets?: LlmPreset[];
}
```

### `TtsConfig`

```ts
interface TtsConfig {
  provider_id?: string;
  api_key?: string;
  endpoint?: string;
  model?: string;
  voice?: string;
  speed?: number;
  pitch?: number;
  emotion?: string;
}
```

### `TtsSystemConfig`

```ts
interface TtsSystemConfig {
  default_provider?: string | null;
  cache: {
    enabled: boolean;
    max_entries: number;
    ttl_secs: number;
  };
  queue: {
    max_concurrent: number;
  };
  providers: ProviderConfigData[];
}
```

### `VisionConfig`

```ts
interface VisionConfig {
  enabled: boolean;
  interval_secs: number;
  change_threshold: number;
  vlm_provider: string;
  vlm_base_url: string | null;
  vlm_model: string;
  vlm_api_key: string | null;
  camera_enabled: boolean;
  camera_device_id: string | null;
}
```

### `ImageGenSystemConfig`

```ts
interface ImageGenSystemConfig {
  default_provider?: string;
  enabled: boolean;
  providers: ImageGenProviderConfig[];
}
```

### `ImageGenParams`

```ts
interface ImageGenParams {
  prompt: string;
  negative_prompt?: string;
  size?: string;
  quality?: string;
  style?: string;
  n: number;
}
```

### `ImageGenResult`

```ts
interface ImageGenResult {
  image_url: string;
  prompt: string;
  provider_id: string;
}
```

### `SttConfig`

```ts
interface SttConfig {
  active_provider: string;
  language?: string;
  auto_send: boolean;
  continuous_listening: boolean;
  wake_word_enabled: boolean;
  wake_word?: string;
  providers: SttProviderConfig[];
}
```

### `SenseVoiceLocalModelStatus`

```ts
interface SenseVoiceLocalModelStatus {
  installed: boolean;
  download_instructions_url: string;
  recommended_model_id: string;
  download_url: string;
  install_dir: string;
  model_path: string;
  tokens_path: string;
}
```

### `SenseVoiceLocalDownloadProgress`

```ts
interface SenseVoiceLocalDownloadProgress {
  stage: "downloading" | "extracting" | "complete" | "ready" | string;
  message: string;
  downloaded_bytes: number;
  total_bytes: number | null;
}
```

### `ToolSettings`

```ts
interface ToolSettings {
  max_tool_rounds: number;
  enabled_tools: Record<string, boolean>;
  max_permission_level: "safe" | "elevated";
  blocked_risk_tags: ("read" | "write" | "external" | "sensitive")[];
}
```

### `ActionInfo`

```ts
interface ActionInfo {
  id: string;
  name: string;
  source: "builtin" | "mcp";
  server_name?: string;
  description: string;
  parameters: { name: string; description: string; required: boolean }[];
  needs_feedback: boolean;
  risk_tags: ("read" | "write" | "external" | "sensitive")[];
  permission_level: "safe" | "elevated";
}
```

### `ActionResult`

```ts
interface ActionResult {
  success: boolean;
  message: string;
  data?: unknown;
}
```

### `McpServerConfig`

```ts
interface McpServerConfig {
  name: string;
  type?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  enabled: boolean;
}
```

### `McpServerStatus`

```ts
interface McpServerStatus {
  name: string;
  enabled: boolean;
  connected: boolean;
  tool_count: number;
  server_version: string | null;
  status: "connected" | "connecting" | "disconnected";
  error: string | null;
}
```

### `Conversation`

```ts
interface Conversation {
  id: string;
  character_id: string;
  title: string;
  topic: string;
  pinned_state: string;
  created_at: string;
  updated_at: string;
}
```

### `LoadedConversation`

```ts
interface LoadedConversation {
  topic: string;
  pinned_state: string;
  messages: ConversationMessage[];
}
```

### `MemoryRecord`

```ts
interface MemoryRecord {
  id: number;
  content: string;
  created_at: number;
  importance: number;
  tier: string;
}
```

### `ListMemoriesResponse`

```ts
interface ListMemoriesResponse {
  memories: MemoryRecord[];
  total: number;
}
```

### `Live2dModelInfo`

```ts
interface Live2dModelInfo {
  name: string;
  path: string;
}
```

### `Live2dModelProfile`

```ts
interface Live2dModelProfile {
  version: number;
  model_path: string;
  available_expressions: string[];
  available_motion_groups: Record<string, number>;
  available_hit_areas: string[];
  cue_map: Record<string, Live2dCueBinding>;
  semantic_cue_map: Record<string, string>;
}
```

### `ModManifest`

```ts
interface ModManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  engine_version?: string;
  layout?: string;
  theme?: string;
  components?: Record<string, string>;
  scripts?: string[];
  permissions?: string[];
  entry?: string;
  ui_entry?: string;
}
```

### `ModThemeJson`

```ts
interface ModThemeJson {
  id?: string;
  name?: string;
  variables: Record<string, string>;
  assets?: {
    fonts?: string[];
    background?: string;
    noise_texture?: string;
    [key: string]: string | string[] | undefined;
  };
  animations?: Record<string, {
    initial?: Record<string, number | string>;
    animate?: Record<string, number | string>;
    exit?: Record<string, number | string>;
    transition?: Record<string, number | string>;
  }>;
}
```

### `TelegramConfig`

```ts
interface TelegramConfig {
  enabled: boolean;
  bot_token?: string;
  bot_token_env?: string;
  allowed_chat_ids: number[];
  send_voice_reply: boolean;
  character_id?: string;
}
```

### `TelegramStatus`

```ts
interface TelegramStatus {
  running: boolean;
  enabled: boolean;
  has_token: boolean;
}
```

### `BackupStats`

```ts
interface BackupStats {
  memories: number;
  conversations: number;
  messages: number;
  configs: number;
}
```

### `ExportResult`

```ts
interface ExportResult {
  path: string;
  size_bytes: number;
  stats: BackupStats;
}
```

### `BackupManifest`

```ts
interface BackupManifest {
  version: string;
  created_at: string;
  app_version: string;
}
```

### `ImportPreview`

```ts
interface ImportPreview {
  manifest: BackupManifest;
  has_database: boolean;
  has_configs: boolean;
  config_files: string[];
  stats: BackupStats;
}
```

### `ImportOptions`

```ts
interface ImportOptions {
  import_database: boolean;
  import_configs: boolean;
  conflict_strategy: "skip" | "overwrite";
  target_character_id?: string;
}
```

### `ImportResult`

```ts
interface ImportResult {
  imported_memories: number;
  imported_conversations: number;
  imported_configs: number;
  characters_json?: string;
  debug_log?: string[];
}
```

### `CharacterRecord`

```ts
interface CharacterRecord {
  id: string;
  name: string;
  persona: string;
  user_nickname: string;
  source_format: string;
  created_at: number;
  updated_at: number;
}
```

### `AutoBackupConfig`

```ts
interface AutoBackupConfig {
  enabled: boolean;
  backup_dir: string;
  interval_days: number;
  auto_cleanup: boolean;
  keep_days: number;
}
```

---

## Command reference

The tables below list the current IPC commands. The `Bridge` column shows whether `src/lib/kokoro-bridge.ts` exports a wrapper for the command.

### System

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `get_engine_info` | `getEngineInfo` | none | `EngineInfo` | Returns app metadata. |
| `get_system_status` | `getSystemStatus` | none | `SystemStatus` | Returns runtime status. |
| `set_window_size` | `setWindowSize` | `width: number`, `height: number` | `void` | Stores the current UI size for image generation. |

### Character

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `get_character_state` | `getCharacterState` | none | `CharacterState` | Returns the current character state. |
| `play_cue` | `playCue` | `cue: string` | `CharacterState` | Updates the active cue. |
| `send_message` | `sendMessage` | `message: string` | `ChatResponse` | Legacy non-streaming chat entry point. |

### Database

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `init_db` | `initDb` | none | `string` | Initializes the SQLite database. |
| `test_vector_store` | `testVectorStore` | none | `DbTestResult` | Smoke test for memory storage. |

### Context

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `set_persona` | `setPersona` | `prompt: string` | `void` | Sets the system prompt. |
| `set_character_name` | `setCharacterName` | `name: string` | `void` | Sets the character display name. |
| `set_active_character_id` | `setActiveCharacterId` | `id: string` | `void` | Persists the active character id. |
| `set_user_name` | `setUserName` | `name: string` | `void` | Sets the user name used in prompts. |
| `set_response_language` | `setResponseLanguage` | `language: string` | `void` | Sets assistant response language. |
| `set_user_language` | `setUserLanguage` | `language: string` | `void` | Sets user language. |
| `set_jailbreak_prompt` | `setJailbreakPrompt` | `prompt: string` | `void` | Persists the jailbreak prompt. |
| `get_jailbreak_prompt` | `getJailbreakPrompt` | none | `string` | Returns the current jailbreak prompt. |
| `set_proactive_enabled` | `setProactiveEnabled` | `enabled: boolean` | `void` | Enables or disables proactive messages. |
| `get_proactive_enabled` | `getProactiveEnabled` | none | `boolean` | Returns proactive toggle state. |
| `set_memory_enabled` | none | `enabled: boolean` | `void` | Enables or disables memory persistence. |
| `get_memory_enabled` | none | none | `boolean` | Returns memory toggle state. |
| `clear_history` | `clearHistory` | none | `void` | Clears conversation history. |
| `delete_last_messages` | `deleteLastMessages` | `count: number` | `void` | Deletes the last visible messages. |
| `get_context_settings` | `getContextSettings` | none | `ContextSettings` | Returns chat context strategy settings. |
| `set_context_settings` | `setContextSettings` | `settings: ContextSettings` | `void` | Saves chat context strategy settings. |
| `end_session` | none | `request: EndSessionRequest` | `void` | Generates a summary in the background and clears history. |

### LLM management

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `get_llm_config` | `getLlmConfig` | none | `LlmConfig` | Returns the active LLM config. |
| `save_llm_config` | `saveLlmConfig` | `config: LlmConfig` | `void` | Saves the active LLM config. |
| `list_ollama_models` | `listOllamaModels` | `baseUrl: string` | `OllamaModelInfo[]` | Lists models from an Ollama server. |

### Chat

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `stream_chat` | `streamChat` | `request: ChatRequest` | `void` | Streaming chat entry point. Emits turn events. |
| `cancel_chat_turn` | `cancelChatTurn` | `turnId: string`, `reason?: string` | `void` | Cancels an in-flight turn. |
| `approve_tool_approval` | `approveToolApproval` | `approvalRequestId: string` | `void` | Approves a pending tool execution. |
| `reject_tool_approval` | `rejectToolApproval` | `approvalRequestId: string`, `reason?: string` | `void` | Rejects a pending tool execution. |

### TTS

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `synthesize` | `synthesize` | `text: string`, `config: TtsConfig` | `void` | Streams audio through TTS events. |
| `list_tts_providers` | `listTtsProviders` | none | `ProviderStatus[]` | Lists configured TTS providers. |
| `list_tts_voices` | `listTtsVoices` | none | `VoiceProfile[]` | Lists available voices. |
| `get_tts_provider_status` | `getTtsProviderStatus` | `providerId: string` | `ProviderStatus \| null` | Returns one provider's status. |
| `clear_tts_cache` | `clearTtsCache` | none | `void` | Clears the synthesis cache. |
| `get_tts_config` | `getTtsConfig` | none | `TtsSystemConfig` | Returns the TTS system config. |
| `save_tts_config` | `saveTtsConfig` | `config: TtsSystemConfig` | `void` | Saves the TTS system config. |
| `list_gpt_sovits_models` | `listGptSovitsModels` | `installPath: string` | `GptSovitsModels` | Lists GPT-SoVITS models. |

### Mod system

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_mods` | `listMods` | none | `ModManifest[]` | Lists discovered mods. |
| `load_mod` | `loadMod` | `modId: string` | `ModManifest` | Loads and activates a mod. |
| `install_mod` | `installMod` | `filePath: string` | `ModManifest` | Installs a mod archive. |
| `get_mod_theme` | `getModTheme` | none | `ModThemeJson \| null` | Returns the active mod theme override. |
| `get_mod_layout` | `getModLayout` | none | `unknown \| null` | Returns the active mod layout override. |
| `dispatch_mod_event` | `dispatchModEvent` | `event: string`, `payload: unknown` | `void` | Sends an event into the active mod. |
| `unload_mod` | `unloadMod` | none | `void` | Unloads the active mod. |

### Live2D

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `import_live2d_zip` | `importLive2dZip` | `zipPath: string` | `string` | Imports a Live2D archive. |
| `import_live2d_folder` | `importLive2dFolder` | `modelJsonPath: string` | `string` | Imports a Live2D folder from a model JSON path. |
| `export_live2d_model` | `exportLive2dModel` | `modelPath: string`, `exportPath: string` | `string` | Exports a Live2D model. |
| `list_live2d_models` | `listLive2dModels` | none | `Live2dModelInfo[]` | Lists installed models. |
| `delete_live2d_model` | `deleteLive2dModel` | `modelName: string` | `void` | Deletes a model. |
| `rename_live2d_model` | `renameLive2dModel` | `modelPath: string`, `newName: string` | `string` | Renames a model. |
| `get_live2d_model_profile` | `getLive2dModelProfile` | `modelPath: string` | `Live2dModelProfile` | Returns the cue/profile mapping. |
| `save_live2d_model_profile` | `saveLive2dModelProfile` | `profile: Live2dModelProfile` | `Live2dModelProfile` | Saves the profile and returns the merged profile. |
| `set_active_live2d_model` | `setActiveLive2dModel` | `modelPath: string \| null` | `void` | Sets the active model. |

### Image generation

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `generate_image` | `generateImage` | `prompt: string`, `providerId?: string` | `ImageGenResult` | Bridge wrapper only exposes prompt and provider selection. The backend builds the rest from config and window size state. |
| `get_imagegen_config` | `getImageGenConfig` | none | `ImageGenSystemConfig` | Returns image generation config. |
| `save_imagegen_config` | `saveImageGenConfig` | `config: ImageGenSystemConfig` | `void` | Saves image generation config. |
| `test_sd_connection` | `testSdConnection` | `baseUrl: string` | `string[]` | Returns Stable Diffusion model names from the server. |

### Vision

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `camera-observation` | `onCameraObservation` | event | `string` | Emitted by the bridge listener, not a Rust command. |
| `start_vision_watcher` | none | none | `void` | Starts the background watcher. |
| `stop_vision_watcher` | none | none | `void` | Stops the background watcher. |
| `capture_screen_now` | `captureScreenNow` | none | `string` | Captures the screen and returns a description. |
| `upload_vision_image` | `uploadVisionImage` | `fileBytes: number[]`, `filename: string` | `string` | Uploads an image to the vision server. |
| `get_vision_config` | `getVisionConfig` | none | `VisionConfig` | Returns the vision watcher config. |
| `save_vision_config` | `saveVisionConfig` | `config: VisionConfig` | `void` | Saves config and starts/stops the watcher. |

### Memory

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_memories` | `listMemories` | `request: { character_id: string; limit: number; offset: number }` | `ListMemoriesResponse` | Lists memories for one character. |
| `update_memory` | `updateMemory` | `request: { id: number; content: string; importance: number }` | `void` | Updates a memory record. |
| `delete_memory` | `deleteMemory` | `request: { id: number }` | `void` | Deletes a memory record. |
| `update_memory_tier` | `updateMemoryTier` | `request: { id: number; tier: string }` | `void` | Updates the memory tier. |

### Characters

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_characters` | `listCharacters` | none | `CharacterRecord[]` | Lists stored characters. |
| `create_character` | `createCharacter` | `request: CharacterRecord` | `void` | Creates a character row. |
| `update_character` | `updateCharacter` | `request: Omit<CharacterRecord, "created_at">` | `void` | Updates a character row. |
| `delete_character` | `deleteCharacter` | `id: string` | `void` | Deletes a character row. |

### Conversation

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_conversations` | `listConversations` | `request: { character_id: string }` | `Conversation[]` | Lists conversations for one character. |
| `load_conversation` | `loadConversation` | `request: { id: string }` | `LoadedConversation` | Loads a conversation. |
| `update_conversation_state` | `updateConversationState` | `request: { id: string; topic?: string; pinned_state?: string }` | `void` | Updates topic or pinned state. |
| `delete_conversation` | `deleteConversation` | `request: { id: string }` | `void` | Deletes a conversation. |
| `create_conversation` | `createConversation` | none | `string` | Creates a new conversation id. |
| `rename_conversation` | `renameConversation` | `request: { id: string; title: string }` | `void` | Renames a conversation. |
| `list_character_ids` | `listCharacterIds` | none | `string[]` | Lists known character ids. |

### STT

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `transcribe_audio` | `transcribeAudio` | `audioBytes: number[]`, `format: string` | `string` | Transcribes one audio clip. |
| `get_stt_config` | `getSttConfig` | none | `SttConfig` | Returns STT config. |
| `save_stt_config` | `saveSttConfig` | `config: SttConfig` | `void` | Saves STT config. |
| `transcribe_wake_word_audio` | none | `samples: Vec<f32>` | `string` | Short one-shot transcription for wake-word detection. |
| `start_native_mic` | none | `auto_stop_on_silence?: boolean` | `void` | Starts the native microphone worker. |
| `stop_native_mic` | none | none | `void` | Stops the native microphone worker. |
| `start_native_wake_word` | none | `wake_word: string`, `trigger_on_speech?: boolean` | `void` | Starts the native wake-word worker. |
| `stop_native_wake_word` | none | none | `void` | Stops the native wake-word worker. |
| `get_sensevoice_local_status` | `getSenseVoiceLocalStatus` | none | `SenseVoiceLocalModelStatus` | Returns the recommended local SenseVoice status. |
| `download_sensevoice_local_model` | `downloadSenseVoiceLocalModel` | none | `SenseVoiceLocalModelStatus` | Downloads the recommended local model. |

### Actions

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_actions` | `listActions` | none | `ActionInfo[]` | Lists all actions. |
| `list_builtin_tools` | `listBuiltinTools` | none | `ActionInfo[]` | Lists only builtin tools. |
| `execute_action` | `executeAction` | `name: string`, `args: Record<string, string>`, `characterId?: string` | `ActionResult` | Executes one action. |
| `get_tool_settings` | `getToolSettings` | none | `ToolSettings` | Returns tool settings. |
| `save_tool_settings` | `saveToolSettings` | `settings: ToolSettings` | `void` | Saves tool settings. |
| `approve_tool_approval` | `approveToolApproval` | `approvalRequestId: string` | `void` | Approval flow for pending tools. |
| `reject_tool_approval` | `rejectToolApproval` | `approvalRequestId: string`, `reason?: string` | `void` | Approval flow for pending tools. |

### MCP

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `list_mcp_servers` | `listMcpServers` | none | `McpServerStatus[]` | Lists configured servers with live status. |
| `add_mcp_server` | `addMcpServer` | `config: McpServerConfig` | `void` | Adds a server and connects in background. |
| `remove_mcp_server` | `removeMcpServer` | `name: string` | `void` | Removes a server. |
| `refresh_mcp_tools` | `refreshMcpTools` | none | `void` | Rebuilds the tool registry from connected servers. |
| `reconnect_mcp_server` | `reconnectMcpServer` | `name: string` | `void` | Reconnects one server. |
| `toggle_mcp_server` | `toggleMcpServer` | `name: string`, `enabled: boolean` | `void` | Enables or disables a server. |

### Telegram

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `get_telegram_config` | `getTelegramConfig` | none | `TelegramConfig` | Returns Telegram config. |
| `save_telegram_config` | `saveTelegramConfig` | `config: TelegramConfig` | `void` | Saves Telegram config. |
| `start_telegram_bot` | `startTelegramBot` | none | `void` | Starts the bot. |
| `stop_telegram_bot` | `stopTelegramBot` | none | `void` | Stops the bot. |
| `get_telegram_status` | `getTelegramStatus` | none | `TelegramStatus` | Returns runtime bot status. |

### Backup and restore

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `export_data` | `exportData` | `exportPath: string`, `charactersJson?: string` | `ExportResult` | Exports database and configs into a `.kokoro` archive. |
| `preview_import` | `previewImport` | `filePath: string` | `ImportPreview` | Reads archive metadata without importing. |
| `import_data` | `importData` | `filePath: string`, `options: ImportOptions` | `ImportResult` | Imports data from a `.kokoro` archive. |
| `get_auto_backup_config` | `getAutoBackupConfig` | none | `AutoBackupConfig` | Returns auto backup config. |
| `save_auto_backup_config` | `saveAutoBackupConfig` | `config: AutoBackupConfig` | `void` | Saves auto backup config. |
| `run_auto_backup_now` | `runAutoBackupNow` | none | `string` | Runs a backup immediately. |

### Singing

| Command | Bridge | Request | Response | Notes |
|---|---|---|---|---|
| `check_rvc_status` | `checkRvcStatus` | none | `boolean` | Checks whether RVC is available. |
| `list_rvc_models` | `listRvcModels` | none | `RvcModelInfo[]` | Lists available RVC models. |
| `convert_singing` | `convertSinging` | `audioPath: string`, `modelName?: string`, `pitchShift?: number`, `separateVocals?: boolean`, `f0Method?: string`, `indexPath?: string`, `indexRate?: number` | `SingingResult` | Converts singing audio. |

### Commands registered in Rust but not exposed by the bridge

These commands exist in `src-tauri/src/lib.rs`, but `src/lib/kokoro-bridge.ts` does not export wrappers for them yet.

| Command | Module | Notes |
|---|---|---|
| `end_session` | `context` | Summarizes the current session in the background. |
| `start_vision_watcher` | `vision` | Starts the background vision loop. |
| `stop_vision_watcher` | `vision` | Stops the background vision loop. |
| `transcribe_wake_word_audio` | `stt` | One-shot wake-word transcription. |
| `start_native_mic` | `stt` | Starts the native microphone worker. |
| `stop_native_mic` | `stt` | Stops the native microphone worker. |
| `start_native_wake_word` | `stt` | Starts the wake-word worker. |
| `stop_native_wake_word` | `stt` | Stops the wake-word worker. |
| `show_pet_window` | `pet` | Shows the floating pet window. |
| `hide_pet_window` | `pet` | Hides the floating pet window. |
| `set_pet_drag_mode` | `pet` | Toggles pet drag mode. |
| `get_pet_config` | `pet` | Returns pet window config. |
| `save_pet_config` | `pet` | Saves pet window config. |
| `move_pet_window` | `pet` | Moves the pet window. |
| `resize_pet_window` | `pet` | Resizes the pet window. |
| `show_bubble_window` | `pet` | Shows the speech bubble window. |
| `update_bubble_text` | `pet` | Updates the bubble text. |
| `hide_bubble_window` | `pet` | Hides the speech bubble window. |

---

## Event reference

### Chat events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `chat-typing` | `TypingParams` | `chat.rs` | none |
| `chat-turn-start` | `{ turn_id: string }` | `chat.rs` | `onChatTurnStart` |
| `chat-turn-delta` | `{ turn_id: string; delta: string; ... }` | `chat.rs` | `onChatTurnDelta` |
| `chat-turn-finish` | `{ turn_id: string; status: "completed" \| "error" }` | `chat.rs` | `onChatTurnFinish` |
| `chat-turn-translation` | `{ turn_id: string; translation: string }` | `chat.rs` | `onChatTurnTranslation` |
| `chat-turn-tool` | `ToolTraceItem`-style payload | `chat.rs` | `onChatTurnTool` |
| `chat-cue` | `{ cue: string; source?: string }` | `chat.rs`, `mods/manager.rs` | `onChatCue` |
| `chat-imagegen` | `{ prompt: string }` | `actions/builtin.rs` | `onChatImageGen` |
| `chat-error` | `string` | `chat.rs` | `onChatError` |

### TTS events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `tts:start` | `{ text: string }` | `tts/manager.rs` | none |
| `tts:audio` | `{ data: number[] }` | `tts/manager.rs` | none |
| `tts:end` | `{ text: string }` | `tts/manager.rs` | none |
| `tts:browser-delegate` | `{ text: string; voice?: string; speed?: number; pitch?: number }` | `tts/manager.rs` | none |

### Vision events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `vision-status` | `"active" \| "inactive"` | `vision/watcher.rs` | none |
| `vision-observation` | `string` | `vision/watcher.rs` | `onVisionObservation` |
| `proactive-trigger` | `{ trigger: string; idle_seconds: number; instruction: string }` | `vision/watcher.rs`, `ai/heartbeat.rs` | none |

### STT events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `stt:sensevoice-local-progress` | `SenseVoiceLocalDownloadProgress` | `commands/stt.rs` | `onSenseVoiceLocalProgress` |
| `stt:mic-volume` | `{ volume: number; rms: number }` | `stt/mic.rs` | none |
| `stt:mic-auto-stop` | `()` | `stt/mic.rs` | none |
| `stt:wake-word-detected` | `string` | `stt/wake_word.rs` | none |

### Idle and proactive events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `idle-behavior` | `{ behavior: unknown }` | `ai/heartbeat.rs` | none |

### Live2D and MOD events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `live2d-profile-updated` | `Live2dModelProfile`-style payload | `commands/live2d.rs` | none |
| `mod:theme-override` | `ModThemeJson` | `mods/manager.rs` | `onModThemeOverride` |
| `mod:layout-override` | `unknown` | `mods/manager.rs` | `onModLayoutOverride` |
| `mod:components-register` | `Record<string, string>` | `mods/manager.rs` | `onModComponentsRegister` |
| `mod:ui-message` | `{ component: string; payload: unknown }` | `mods/manager.rs` | `onModUiMessage` |
| `mod:unload` | `()` | `mods/manager.rs` | `onModUnload` |
| `mod:script-event` | `{ event: string; payload: unknown }` | `mods/api.ts` bridge path | `onModScriptEvent` |

### Image generation events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `imagegen:done` | `ImageGenResult` | `chat.rs` | `onImageGenDone` |
| `imagegen:error` | `string` | `chat.rs` | `onImageGenError` |

### Telegram events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `telegram:chat-sync` | `{ role: string; text: string; translation?: string }` | `telegram/bot.rs` | `onTelegramChatSync` |

### Backup and memory events

| Event | Payload | Emitted by | Bridge wrapper |
|---|---|---|---|
| `memory:updated` | `string` | `actions/builtin.rs` | none |
| `pet-window-closed` | `()` | `commands/pet.rs` | none |
| `bubble-text-update` | `string` | `commands/pet.rs` | none |
| `toggle-chat-input` | `()` | `lib.rs` | none |

---

## Custom protocols

### `mod://`

Serves MOD assets from the installed `mods/` directory.

- HTML files are served with the MOD SDK injected automatically.
- Path traversal is blocked.
- The response sets a strict Content Security Policy.
- CORS is limited to the app origin.

Example:

```html
<img src="mod://example-mod/assets/icon.png" />
<script src="mod://example-mod/index.js"></script>
```

### `live2d://`

Serves Live2D runtime assets from `{app_data_dir}/live2d_models/`.

- Path traversal is blocked.
- The protocol resolves files relative to the app data directory.
- It supports the runtime file layout expected by pixi-live2d-display.

Example:

```txt
live2d://localhost/my-model/runtime/model3.json
```

---

## Error handling

### IPC error shape

Most commands return `Result<T, String>` at the IPC boundary.

The backend uses `KokoroError` internally.
`KokoroError` serializes as:

```ts
{
  code: string;
  message: string;
}
```

If serialization fails, the backend falls back to a plain string.

### Common failures

| Command | Error | Condition |
|---|---|---|
| `send_message` | `Message cannot be empty` | Blank message input |
| `stream_chat` | `API Key is required` | Missing API key for direct API calls |
| `synthesize` | `No TTS provider available` | No provider is configured |
| `synthesize` | `Provider {id} not found` | Selected provider does not exist |
| `run_auto_backup_now` | `Backup directory not set` | Auto backup directory is empty |
| `load_conversation` | `NotFound`-style error | Conversation id does not exist |

### Frontend pattern

```ts
try {
  await streamChat({
    message: "Hello",
    api_key: "sk-...",
  });
} catch (error) {
  console.error("command failed", error);
}
```

If you want structured handling, use `parseKokoroError` from `kokoro-bridge.ts`.

---

## Bridge reference

`src/lib/kokoro-bridge.ts` is the typed frontend entry point.

### Exported command wrappers

- system: `getEngineInfo`, `getSystemStatus`, `setWindowSize`
- character: `getCharacterState`, `playCue`
- database: `initDb`, `testVectorStore`, `sendMessage`
- context: `setPersona`, `setCharacterName`, `setActiveCharacterId`, `setUserName`, `setResponseLanguage`, `setUserLanguage`, `setJailbreakPrompt`, `getJailbreakPrompt`, `setProactiveEnabled`, `getProactiveEnabled`, `clearHistory`, `setMemoryEnabled`, `getMemoryEnabled`, `getContextSettings`, `setContextSettings`, `deleteLastMessages`
- llm/chat: `getLlmConfig`, `saveLlmConfig`, `listOllamaModels`, `streamChat`, `cancelChatTurn`, `approveToolApproval`, `rejectToolApproval`
- mod/live2d/imagegen/vision/memory/stt/actions/mcp/telegram/backup/characters/singing: see the command tables above

### Exported event wrappers

- chat: `onChatError`, `onChatTurnStart`, `onChatTurnDelta`, `onChatTurnFinish`, `onChatTurnTranslation`, `onChatCue`, `onChatTurnTool`
- mod: `onModThemeOverride`, `onModLayoutOverride`, `onModComponentsRegister`, `onModUiMessage`, `onModUnload`, `onModScriptEvent`
- imagegen: `onChatImageGen`, `onImageGenDone`, `onImageGenError`
- vision: `onVisionObservation`, `onCameraObservation`
- STT: `onSenseVoiceLocalProgress`
- singing: `onSingingProgress`
- telegram: `onTelegramChatSync`

### Bridge-only helpers

These are local TypeScript helpers and not IPC commands:

- `fetchModels`
- `hasPinnedConversationState`
- `getConversationDisplayTitle`
- `parseKokoroError`
- `safeInvoke`
- `isKokoroErrorCode`

---

## Compatibility notes

### Old entries removed from the previous spec

The following items in the old document no longer match the current code and should not be treated as current API:

- `get_emotion_settings`
- `save_emotion_settings`
- old `ChatRequest` shape without image support
- old `ToolSettings` shape without permission/risk controls
- old TTS / STT / ImageGen / MCP return types that no longer match the bridge
- old quick reference entries that returned `Uint8Array` for streaming APIs

### Bridge coverage

Not every backend command has a bridge wrapper yet.
The command tables above mark those gaps explicitly.

### Event coverage

Some events are emitted by the backend but do not yet have dedicated bridge helpers.
That is intentional. The backend event string is still the contract.

---

## Appendix

### Backend modules used by this document

- `src-tauri/src/lib.rs`
- `src-tauri/src/commands/*.rs`
- `src-tauri/src/tts/manager.rs`
- `src-tauri/src/vision/watcher.rs`
- `src-tauri/src/stt/*.rs`
- `src-tauri/src/mods/*.rs`
- `src-tauri/src/ai/*.rs`
- `src/lib/kokoro-bridge.ts`
- `src/core/types/mod.ts`
