/**
 * Kokoro Engine — IPC Bridge
 * 
 * Typed wrapper around Tauri's invoke API.
 * All backend commands are accessed through this module.
 */
// pattern: Mixed (unavoidable)
// Reason: 前端 bridge 同时承担 IPC 副作用封装与类型导出，是前端与 Tauri 边界的集中编排层。
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ModManifest, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson } from "../core/types/mod";
export type { ModManifest, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson };

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
        return await tauriInvoke<T>(cmd, args);
    } catch (error) {
        throw parseKokoroError(error);
    }
}

// ── Types ──────────────────────────────────────────

export interface EngineInfo {
    name: string;
    version: string;
    platform: string;
}

export interface SystemStatus {
    engine_running: boolean;
    active_modules: string[];
    memory_usage_mb: number;
}

export interface CharacterState {
    name: string;
    current_cue: string;
    mood: number;
    is_speaking: boolean;
}

export interface ChatResponse {
    text: string;
    cue: string;
    mood_delta: number;
}

// ── System Commands ────────────────────────────────

export async function getEngineInfo(): Promise<EngineInfo> {
    return invoke<EngineInfo>("get_engine_info");
}

export async function getSystemStatus(): Promise<SystemStatus> {
    return invoke<SystemStatus>("get_system_status");
}

export async function setWindowSize(width: number, height: number): Promise<void> {
    return invoke("set_window_size", { width, height });
}

// ── Character Commands ─────────────────────────────

export async function getCharacterState(): Promise<CharacterState> {
    return invoke<CharacterState>("get_character_state");
}

export async function playCue(cue: string): Promise<CharacterState> {
    return invoke<CharacterState>("play_cue", { cue });
}

// ── Database Commands ──────────────────────────────

export interface DbTestResult {
    success: boolean;
    message: string;
    record_count: number;
}

export async function initDb(): Promise<string> {
    return invoke<string>("init_db");
}

export async function testVectorStore(): Promise<DbTestResult> {
    return invoke<DbTestResult>("test_vector_store");
}

export async function sendMessage(message: string): Promise<ChatResponse> {
    return invoke<ChatResponse>("send_message", { message });
}

// ── Context Management ─────────────────────────────

export async function setPersona(prompt: string): Promise<void> {
    return invoke("set_persona", { prompt });
}

export async function setCharacterName(name: string): Promise<void> {
    return invoke("set_character_name", { name });
}

export async function setUserName(name: string): Promise<void> {
    return invoke("set_user_name", { name });
}

export async function setResponseLanguage(language: string): Promise<void> {
    return invoke("set_response_language", { language });
}

export async function setUserLanguage(language: string): Promise<void> {
    return invoke("set_user_language", { language });
}

export async function setJailbreakPrompt(prompt: string): Promise<void> {
    return invoke("set_jailbreak_prompt", { prompt });
}

export async function getJailbreakPrompt(): Promise<string> {
    return invoke("get_jailbreak_prompt");
}

export async function setProactiveEnabled(enabled: boolean): Promise<void> {
    return invoke("set_proactive_enabled", { enabled });
}

export async function getProactiveEnabled(): Promise<boolean> {
    return invoke("get_proactive_enabled");
}

export async function clearHistory(): Promise<void> {
    return invoke("clear_history");
}

export async function setMemoryEnabled(enabled: boolean): Promise<void> {
    return invoke("set_memory_enabled", { enabled });
}

export async function getMemoryEnabled(): Promise<boolean> {
    return invoke<boolean>("get_memory_enabled");
}

// ── Context Settings ───────────────────────────────

export interface ContextSettings {
    strategy: "window" | "summary";
    max_message_chars: number;
}

export async function getContextSettings(): Promise<ContextSettings> {
    return invoke<ContextSettings>("get_context_settings");
}

export async function setContextSettings(settings: ContextSettings): Promise<void> {
    return invoke("set_context_settings", { settings });
}

export async function deleteLastMessages(count: number): Promise<void> {
    return invoke("delete_last_messages", { count });
}

// ── LLM Config Management ──────────────────────────

export interface LlmProviderConfig {
    id: string;
    provider_type: string;
    enabled: boolean;
    supports_native_tools: boolean;
    api_key?: string;
    api_key_env?: string;
    base_url?: string;
    model?: string;
    extra?: Record<string, unknown>;
}

export interface LlmPreset {
    id: string;
    name: string;
    active_provider: string;
    system_provider?: string;
    system_model?: string;
    providers: LlmProviderConfig[];
}

export interface LlmConfig {
    active_provider: string;
    system_provider?: string;
    system_model?: string;
    providers: LlmProviderConfig[];
    presets?: LlmPreset[];
}

export interface OllamaModelInfo {
    name: string;
    size?: number;
    modified_at?: string;
}

export async function getLlmConfig(): Promise<LlmConfig> {
    return invoke<LlmConfig>("get_llm_config");
}

export async function saveLlmConfig(config: LlmConfig): Promise<void> {
    return invoke("save_llm_config", { config });
}

export async function listOllamaModels(baseUrl: string): Promise<OllamaModelInfo[]> {
    return invoke<OllamaModelInfo[]>("list_ollama_models", { baseUrl });
}

// ── LLM Streaming ──────────────────────────────────

export interface ChatRequest {
    message: string;
    api_key?: string;
    endpoint?: string;
    model?: string;
    allow_image_gen?: boolean;
    images?: string[];
    character_id?: string;
    /** If true, neither user message nor response is saved to chat history */
    hidden?: boolean;
}

export async function streamChat(request: ChatRequest): Promise<void> {
    return invoke("stream_chat", { request });
}

export async function cancelChatTurn(turnId: string, reason?: string): Promise<void> {
    return invoke("cancel_chat_turn", { turnId, reason: reason ?? null });
}

export async function onChatError(callback: (error: string) => void): Promise<UnlistenFn> {
    return listen<string>("chat-error", (event) => callback(event.payload));
}

export interface FailureEventContext {
    deny_kind?: "hook_denied" | "policy_denied" | "fail_closed" | "pending_approval" | "execution_error";
    approval_status?: "requested" | "approved" | "rejected";
    [key: string]: unknown;
}

export interface FailureEvent {
    event_id: string;
    timestamp: string;
    domain: string;
    stage: string;
    code: string;
    message: string;
    retryable: boolean;
    trace_id: string;
    conversation_id?: string | null;
    turn_id?: string | null;
    character_id?: string | null;
    context?: FailureEventContext | null;
}

export async function onChatFailure(callback: (event: FailureEvent) => void): Promise<UnlistenFn> {
    return listen<FailureEvent | string>("chat-failure", (event) => {
        const payload = event.payload;
        if (typeof payload === "string") {
            try {
                const parsed = JSON.parse(payload) as FailureEvent;
                callback(parsed);
            } catch {
                callback({
                    event_id: "",
                    timestamp: "",
                    domain: "chat",
                    stage: "unknown",
                    code: "CHAT_FAILURE",
                    message: payload,
                    retryable: false,
                    trace_id: "",
                });
            }
            return;
        }

        callback(payload);
    });
}

export function parseFailureEvent(payload: unknown): FailureEvent | null {
    if (typeof payload === "string") {
        try {
            return JSON.parse(payload) as FailureEvent;
        } catch {
            return null;
        }
    }

    if (typeof payload === "object" && payload !== null) {
        const candidate = payload as Partial<FailureEvent>;
        if (typeof candidate.code === "string" && typeof candidate.message === "string") {
            return {
                event_id: candidate.event_id ?? "",
                timestamp: candidate.timestamp ?? "",
                domain: candidate.domain ?? "chat",
                stage: candidate.stage ?? "unknown",
                code: candidate.code,
                message: candidate.message,
                retryable: Boolean(candidate.retryable),
                trace_id: candidate.trace_id ?? "",
                conversation_id: candidate.conversation_id ?? null,
                turn_id: candidate.turn_id ?? null,
                character_id: candidate.character_id ?? null,
                context: candidate.context ?? null,
            };
        }
    }

    return null;
}

export function parseLegacyChatError(payload: unknown): string {
    if (typeof payload === "string") {
        return payload;
    }

    if (payload instanceof Error) {
        return payload.message;
    }

    const failure = parseFailureEvent(payload);
    if (failure) {
        return failure.message;
    }

    return String(payload);
}

export interface ChatTurnStartEvent {
    turn_id: string;
}

export interface ChatTurnDeltaEvent {
    turn_id: string;
    delta: string;
}

export interface ChatTurnFinishEvent {
    turn_id: string;
    status: "completed" | "error";
}

export interface ChatTurnTranslationEvent {
    turn_id: string;
    translation: string;
}

export interface ToolTraceItem {
    tool: string;
    toolName?: string;
    toolId?: string;
    text: string;
    isError?: boolean;
    source?: "builtin" | "mcp";
    serverName?: string;
    needsFeedback?: boolean;
    permissionLevel?: "safe" | "elevated";
    riskTags?: Array<"read" | "write" | "external" | "sensitive">;
    denyKind?: "hook_denied" | "policy_denied" | "fail_closed" | "pending_approval" | "execution_error";
    approvalRequestId?: string;
    approvalStatus?: "requested" | "approved" | "rejected";
}

export interface ChatTurnToolEvent {
    turn_id: string;
    tool: string;
    tool_name?: string;
    tool_id?: string;
    source?: "builtin" | "mcp";
    server_name?: string;
    needs_feedback?: boolean;
    permission_level?: "safe" | "elevated";
    risk_tags?: Array<"read" | "write" | "external" | "sensitive">;
    result?: {
        message: string;
    };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}

export async function onChatTurnStart(callback: (event: ChatTurnStartEvent) => void): Promise<UnlistenFn> {
    return listen<ChatTurnStartEvent>("chat-turn-start", (event) => callback(event.payload));
}

export async function onChatTurnDelta(callback: (event: ChatTurnDeltaEvent) => void): Promise<UnlistenFn> {
    return listen<ChatTurnDeltaEvent>("chat-turn-delta", (event) => callback(event.payload));
}

export async function onChatTurnFinish(callback: (event: ChatTurnFinishEvent) => void): Promise<UnlistenFn> {
    return listen<ChatTurnFinishEvent>("chat-turn-finish", (event) => callback(event.payload));
}

export async function onChatTurnTranslation(callback: (event: ChatTurnTranslationEvent) => void): Promise<UnlistenFn> {
    return listen<ChatTurnTranslationEvent>("chat-turn-translation", (event) => callback(event.payload));
}

// ── Cue Events ─────────────────────────────────────

export interface CueEvent {
    cue: string;
    source?: string;
}

export async function onChatCue(
    callback: (data: CueEvent) => void
): Promise<UnlistenFn> {
    return listen<CueEvent>("chat-cue", (event) => callback(event.payload));
}

// ── LLM Management ──────────────────────────────────

export interface Model {
    id: string;
    object: string;
    created: number;
    owned_by: string;
}

export interface ModelListResponse {
    object: "list";
    data: Model[];
}

export async function fetchModels(endpoint: string, apiKey: string): Promise<string[]> {
    // Remove trailing slash if present
    const baseUrl = endpoint.replace(/\/+$/, "");
    // Handle cases where user provides full /v1/chat/completions URL
    // We want the base, usually ending in /v1
    const cleanUrl = baseUrl.replace(/\/chat\/completions$/, "");

    try {
        const response = await fetch(`${cleanUrl}/models`, {
            method: "GET",
            headers: {
                "Authorization": `Bearer ${apiKey}`,
                "Content-Type": "application/json"
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to fetch models: ${response.statusText}`);
        }

        const data: ModelListResponse = await response.json();
        return data.data.map(m => m.id).sort();
    } catch (error) {
        console.error("[KokoroBridge] fetchModels error:", error);
        throw error;
    }
}

// ── Mod System ──────────────────────────────────────

export async function listMods(): Promise<ModManifest[]> {
    return invoke("list_mods");
}

export async function loadMod(modId: string): Promise<ModManifest> {
    return invoke("load_mod", { modId });
}

export async function installMod(filePath: string): Promise<ModManifest> {
    return invoke("install_mod", { filePath });
}

export async function getModTheme(): Promise<ModThemeJson | null> {
    return invoke("get_mod_theme");
}

export async function getModLayout(): Promise<unknown | null> {
    return invoke("get_mod_layout");
}

// ── Mod Events ─────────────────────────────────────

export async function onModThemeOverride(
    callback: (theme: ModThemeJson) => void
): Promise<UnlistenFn> {
    return listen<ModThemeJson>("mod:theme-override", (event) => callback(event.payload));
}

export async function onModLayoutOverride(
    callback: (layout: unknown) => void
): Promise<UnlistenFn> {
    return listen<unknown>("mod:layout-override", (event) => callback(event.payload));
}

export async function onModComponentsRegister(
    callback: (components: Record<string, string>) => void
): Promise<UnlistenFn> {
    return listen<Record<string, string>>("mod:components-register", (event) => callback(event.payload));
}

export async function onModUiMessage(
    callback: (data: { component: string; payload: unknown }) => void
): Promise<UnlistenFn> {
    return listen<{ component: string; payload: unknown }>("mod:ui-message", (event) => callback(event.payload));
}

export async function dispatchModEvent(event: string, payload: unknown): Promise<void> {
    return invoke("dispatch_mod_event", { event, payload });
}

export async function unloadMod(): Promise<void> {
    return invoke("unload_mod");
}

export async function onModUnload(callback: () => void): Promise<UnlistenFn> {
    return listen<void>("mod:unload", () => callback());
}

export async function onModScriptEvent(
    callback: (data: { event: string; payload: unknown }) => void
): Promise<UnlistenFn> {
    return listen<{ event: string; payload: unknown }>("mod:script-event", (e) => callback(e.payload));
}

// ── Live2D Model Import ─────────────────────────────

export interface Live2dModelInfo {
    name: string;
    path: string;
}

export interface Live2dCueBinding {
    expression?: string | null;
    motion_group?: string | null;
    exclude_from_prompt?: boolean;
}

export const BUILTIN_LIVE2D_MODEL_PATH = "__builtin__/haru/haru_greeter_t03.model3.json";

export interface Live2dModelProfile {
    version: number;
    model_path: string;
    available_expressions: string[];
    available_motion_groups: Record<string, number>;
    available_hit_areas: string[];
    cue_map: Record<string, Live2dCueBinding>;
    semantic_cue_map: Record<string, string>;
}

export async function importLive2dZip(zipPath: string): Promise<string> {
    return invoke<string>("import_live2d_zip", { zipPath });
}

export async function importLive2dFolder(modelJsonPath: string): Promise<string> {
    return invoke<string>("import_live2d_folder", { modelJsonPath });
}

export async function exportLive2dModel(modelPath: string, exportPath: string): Promise<string> {
    return invoke<string>("export_live2d_model", { modelPath, exportPath });
}

export async function listLive2dModels(): Promise<Live2dModelInfo[]> {
    return invoke<Live2dModelInfo[]>("list_live2d_models");
}

export async function deleteLive2dModel(modelName: string): Promise<void> {
    return invoke("delete_live2d_model", { modelName });
}

export async function renameLive2dModel(modelPath: string, newName: string): Promise<string> {
    return invoke<string>("rename_live2d_model", { modelPath, newName });
}

export async function getLive2dModelProfile(modelPath: string): Promise<Live2dModelProfile> {
    return invoke<Live2dModelProfile>("get_live2d_model_profile", { modelPath });
}

export async function saveLive2dModelProfile(profile: Live2dModelProfile): Promise<Live2dModelProfile> {
    return invoke<Live2dModelProfile>("save_live2d_model_profile", { profile });
}

export async function setActiveLive2dModel(modelPath: string | null): Promise<void> {
    return invoke("set_active_live2d_model", { modelPath });
}

// ── TTS ────────────────────────────────────────────

export async function synthesize(text: string, config: TtsConfig): Promise<void> {
    return invoke("synthesize", { text, config });
}

export async function listTtsProviders(): Promise<ProviderStatus[]> {
    return invoke<ProviderStatus[]>("list_tts_providers");
}

export async function listTtsVoices(): Promise<VoiceProfile[]> {
    return invoke<VoiceProfile[]>("list_tts_voices");
}

export async function getTtsProviderStatus(providerId: string): Promise<ProviderStatus | null> {
    return invoke<ProviderStatus | null>("get_tts_provider_status", { providerId });
}

export async function clearTtsCache(): Promise<void> {
    return invoke("clear_tts_cache");
}

export async function getTtsConfig(): Promise<TtsSystemConfig> {
    return invoke<TtsSystemConfig>("get_tts_config");
}

export async function saveTtsConfig(config: TtsSystemConfig): Promise<void> {
    return invoke("save_tts_config", { config });
}

export interface GptSovitsModels {
    gpt_models: string[];
    sovits_models: string[];
}

export async function listGptSovitsModels(installPath: string): Promise<GptSovitsModels> {
    return invoke<GptSovitsModels>("list_gpt_sovits_models", { installPath });
}

// ── Image Generation ───────────────────────────────

export interface ImageGenResult {
    image_url: string;
    prompt: string;
    provider_id: string;
}

export interface ImageGenProviderConfig {
    id: string;
    provider_type: "openai" | "stable_diffusion" | string;
    enabled: boolean;
    api_key?: string;
    api_key_env?: string;
    base_url?: string;
    model?: string;
    size?: string;
    quality?: string;
    style?: string;
    negative_prompt?: string;
    extra?: Record<string, any>;
}

export interface ImageGenSystemConfig {
    default_provider?: string;
    enabled: boolean;
    providers: ImageGenProviderConfig[];
}

export async function generateImage(prompt: string, providerId?: string): Promise<ImageGenResult> {
    return invoke("generate_image", { prompt, providerId });
}

export async function getImageGenConfig(): Promise<ImageGenSystemConfig> {
    return invoke("get_imagegen_config");
}

export async function saveImageGenConfig(config: ImageGenSystemConfig): Promise<void> {
    return invoke("save_imagegen_config", { config });
}

export async function testSdConnection(baseUrl: string): Promise<string[]> {
    return invoke<string[]>("test_sd_connection", { baseUrl });
}

// ── Image Gen Events ──────────────────────────────

export interface ChatImageGenEvent {
    prompt: string;
}

export async function onChatImageGen(callback: (event: ChatImageGenEvent) => void): Promise<UnlistenFn> {
    return listen<ChatImageGenEvent>("chat-imagegen", (e) => callback(e.payload));
}

export async function onImageGenDone(callback: (event: ImageGenResult) => void): Promise<UnlistenFn> {
    return listen<ImageGenResult>("imagegen:done", (e) => callback(e.payload));
}

export async function onImageGenError(callback: (error: string) => void): Promise<UnlistenFn> {
    return listen<string>("imagegen:error", (e) => callback(e.payload));
}

// ── Vision Upload ──────────────────────────────────

export async function uploadVisionImage(fileBytes: number[], filename: string): Promise<string> {
    return invoke<string>("upload_vision_image", { fileBytes, filename });
}

// ── Vision Config & Watcher ────────────────────────

export interface VisionConfig {
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

export async function getVisionConfig(): Promise<VisionConfig> {
    return invoke<VisionConfig>("get_vision_config");
}

export async function saveVisionConfig(config: VisionConfig): Promise<void> {
    return invoke("save_vision_config", { config });
}

export async function captureScreenNow(): Promise<string> {
    return invoke<string>("capture_screen_now");
}

export async function onVisionObservation(callback: (desc: string) => void): Promise<UnlistenFn> {
    return listen<string>("vision-observation", (event) => callback(event.payload));
}

export async function onCameraObservation(callback: (desc: string) => void): Promise<UnlistenFn> {
    return listen<string>("camera-observation", (event) => callback(event.payload));
}

// ── Memory Management ──────────────────────────────

export interface MemoryRecord {
    id: number;
    content: string;
    created_at: number;
    importance: number;
    tier: string;
}

export interface ListMemoriesResponse {
    memories: MemoryRecord[];
    total: number;
}

export type MemoryUpgradeConfig = {
    readonly observability_enabled: boolean;
    readonly event_trigger_enabled: boolean;
    readonly event_cooldown_secs: number;
    readonly structured_memory_enabled: boolean;
    readonly intent_routing_enabled: boolean;
    readonly retrieval_eval_enabled: boolean;
};

export type MemoryObservabilitySummary = {
    readonly write_event_count: number;
    readonly retrieval_log_count: number;
};

export type MemoryWriteEventRecord = {
    readonly source: string;
    readonly trigger: string;
    readonly extracted_count: number;
    readonly stored_count: number;
    readonly deduplicated_count: number;
    readonly invalidated_count: number;
    readonly duration_ms: number;
};

export type MemoryRetrievalLogRecord = {
    readonly query: string;
    readonly semantic_candidates: number;
    readonly bm25_candidates: number;
    readonly fused_candidates: number;
    readonly injected_count: number;
    readonly overlap_count: number | null;
    readonly semantic_only_count: number | null;
    readonly bm25_only_count: number | null;
    readonly filtered_out_count: number | null;
};

export async function listMemories(characterId: string, limit = 50, offset = 0): Promise<ListMemoriesResponse> {
    return invoke<ListMemoriesResponse>("list_memories", {
        request: { character_id: characterId, limit, offset },
    });
}

export async function updateMemory(id: number, content: string, importance: number): Promise<void> {
    return invoke("update_memory", {
        request: { id, content, importance },
    });
}

export async function deleteMemory(id: number): Promise<void> {
    return invoke("delete_memory", {
        request: { id },
    });
}

export async function updateMemoryTier(id: number, tier: string): Promise<void> {
    return invoke("update_memory_tier", {
        request: { id, tier },
    });
}

export async function getMemoryUpgradeConfig(): Promise<MemoryUpgradeConfig> {
    return invoke<MemoryUpgradeConfig>("get_memory_upgrade_config");
}

export async function setMemoryUpgradeConfig(config: Readonly<MemoryUpgradeConfig>): Promise<void> {
    return invoke("set_memory_upgrade_config", { config });
}

export async function getMemoryObservabilitySummary(): Promise<MemoryObservabilitySummary> {
    return invoke<MemoryObservabilitySummary>("get_memory_observability_summary");
}

export async function getLatestMemoryWriteEvent(): Promise<MemoryWriteEventRecord | null> {
    return invoke<MemoryWriteEventRecord | null>("get_latest_memory_write_event");
}

export async function getLatestMemoryRetrievalLog(): Promise<MemoryRetrievalLogRecord | null> {
    return invoke<MemoryRetrievalLogRecord | null>("get_latest_memory_retrieval_log");
}

// ── STT (Speech-to-Text) ──────────────────────────────

export interface SttProviderConfig {
    id: string;
    provider_type: string;
    enabled: boolean;
    api_key?: string;
    api_key_env?: string;
    base_url?: string;
    model?: string;
    // sensevoice_local fields
    model_path?: string;
    tokens_path?: string;
    num_threads?: number;
    use_itn?: boolean;
}

export interface SttConfig {
    active_provider: string;
    language?: string;
    auto_send: boolean;
    continuous_listening: boolean;
    wake_word_enabled: boolean;
    wake_word?: string;
    providers: SttProviderConfig[];
}

export interface SenseVoiceLocalModelStatus {
    installed: boolean;
    download_instructions_url: string;
    recommended_model_id: string;
    download_url: string;
    install_dir: string;
    model_path: string;
    tokens_path: string;
}

export interface SenseVoiceLocalDownloadProgress {
    stage: "downloading" | "extracting" | "complete" | "ready" | string;
    message: string;
    downloaded_bytes: number;
    total_bytes: number | null;
}

export async function transcribeAudio(audioBytes: number[], format: string): Promise<string> {
    return invoke<string>("transcribe_audio", { audioBytes, format });
}

export async function getSttConfig(): Promise<SttConfig> {
    return invoke<SttConfig>("get_stt_config");
}

export async function saveSttConfig(config: SttConfig): Promise<void> {
    return invoke("save_stt_config", { config });
}

export async function getSenseVoiceLocalStatus(): Promise<SenseVoiceLocalModelStatus> {
    return invoke<SenseVoiceLocalModelStatus>("get_sensevoice_local_status");
}

export async function downloadSenseVoiceLocalModel(): Promise<SenseVoiceLocalModelStatus> {
    return invoke<SenseVoiceLocalModelStatus>("download_sensevoice_local_model");
}

export async function onSenseVoiceLocalProgress(
    callback: (progress: SenseVoiceLocalDownloadProgress) => void
): Promise<UnlistenFn> {
    return listen<SenseVoiceLocalDownloadProgress>(
        "stt:sensevoice-local-progress",
        (event) => callback(event.payload)
    );
}

// ── Actions (Tool Calling) ─────────────────────────────

export interface ActionInfo {
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

export interface ActionResult {
    success: boolean;
    message: string;
    data?: unknown;
}

export interface ToolCallEvent {
    tool: string;
    tool_id?: string;
    result?: ActionResult;
    error?: string;
}

export interface ToolSettings {
    max_tool_rounds: number;
    enabled_tools: Record<string, boolean>;
    max_permission_level: "safe" | "elevated";
    blocked_risk_tags: ("read" | "write" | "external" | "sensitive")[];
}

export async function listActions(): Promise<ActionInfo[]> {
    return invoke<ActionInfo[]>("list_actions");
}

export async function listBuiltinTools(): Promise<ActionInfo[]> {
    return invoke<ActionInfo[]>("list_builtin_tools");
}

export async function executeAction(name: string, args: Record<string, string>, characterId?: string): Promise<ActionResult> {
    return invoke<ActionResult>("execute_action", { name, args, characterId });
}

export async function onChatTurnTool(callback: (event: ChatTurnToolEvent) => void): Promise<UnlistenFn> {
    return listen<ChatTurnToolEvent>("chat-turn-tool", (event) => callback(event.payload));
}

export async function getToolSettings(): Promise<ToolSettings> {
    return invoke<ToolSettings>("get_tool_settings");
}

export async function saveToolSettings(settings: ToolSettings): Promise<void> {
    return invoke("save_tool_settings", { settings });
}

export async function approveToolApproval(approvalRequestId: string): Promise<void> {
    return invoke("approve_tool_approval", { approvalRequestId });
}

export async function rejectToolApproval(approvalRequestId: string, reason: string | null = null): Promise<void> {
    return invoke("reject_tool_approval", { approvalRequestId, reason });
}

// ── MCP (Model Context Protocol) ──────────────────────────

export interface McpServerConfig {
    name: string;
    /** "stdio" (default) or "streamable_http" */
    type?: string;
    command?: string;
    args?: string[];
    env?: Record<string, string>;
    /** HTTP endpoint URL (for streamable_http transport) */
    url?: string;
    enabled: boolean;
}

export interface McpServerStatus {
    name: string;
    enabled: boolean;
    connected: boolean;
    tool_count: number;
    server_version: string | null;
    status: "connected" | "connecting" | "disconnected";
    error: string | null;
}

export async function listMcpServers(): Promise<McpServerStatus[]> {
    return invoke<McpServerStatus[]>("list_mcp_servers");
}

export async function addMcpServer(config: McpServerConfig): Promise<void> {
    return invoke("add_mcp_server", { config });
}

export async function removeMcpServer(name: string): Promise<void> {
    return invoke("remove_mcp_server", { name });
}

export async function refreshMcpTools(): Promise<void> {
    return invoke("refresh_mcp_tools");
}

export async function reconnectMcpServer(name: string): Promise<void> {
    return invoke("reconnect_mcp_server", { name });
}

export async function toggleMcpServer(name: string, enabled: boolean): Promise<void> {
    return invoke("toggle_mcp_server", { name, enabled });
}

// ── Conversation History ───────────────────────────────

export interface Conversation {
    id: string;
    character_id: string;
    title: string;
    topic: string;
    pinned_state: string;
    created_at: string;
    updated_at: string;
}

export interface ConversationMessage {
    role: string;
    content: string;
    metadata?: string;
    created_at: string;
}

export interface LoadedConversation {
    topic: string;
    pinned_state: string;
    messages: ConversationMessage[];
}

export async function listConversations(characterId: string): Promise<Conversation[]> {
    return invoke<Conversation[]>("list_conversations", {
        request: { character_id: characterId },
    });
}

export async function loadConversation(id: string): Promise<LoadedConversation> {
    return invoke<LoadedConversation>("load_conversation", {
        request: { id },
    });
}

export async function updateConversationState(
    id: string,
    patch: { topic?: string; pinned_state?: string }
): Promise<void> {
    return invoke("update_conversation_state", {
        request: { id, ...patch },
    });
}

export function hasPinnedConversationState(pinnedState: string): boolean {
    const normalized = pinnedState.trim();
    return normalized !== "" && normalized !== "{}";
}

export function getConversationDisplayTitle(conversation: Conversation): string {
    return conversation.topic.trim() || conversation.title;
}

export async function deleteConversation(id: string): Promise<void> {
    return invoke("delete_conversation", {
        request: { id },
    });
}

export async function createConversation(): Promise<string> {
    return invoke<string>("create_conversation");
}

export async function renameConversation(id: string, title: string): Promise<void> {
    return invoke("rename_conversation", {
        request: { id, title },
    });
}

// ── Singing (RVC Voice Conversion) ──────────────────────

export interface RvcModelInfo {
    name: string;
    description?: string;
}

export interface SingingResult {
    output_path: string;
    duration_secs: number;
}

export interface SingingProgressEvent {
    stage: "reading" | "converting" | "done";
    progress: number;
    output_path?: string;
}

export async function checkRvcStatus(): Promise<boolean> {
    return invoke<boolean>("check_rvc_status");
}

export async function listRvcModels(): Promise<RvcModelInfo[]> {
    return invoke<RvcModelInfo[]>("list_rvc_models");
}

export async function convertSinging(
    audioPath: string,
    modelName?: string,
    pitchShift?: number,
    separateVocals?: boolean,
    // Advanced RVC params
    f0Method?: string,
    indexPath?: string,
    indexRate?: number,
): Promise<SingingResult> {
    return invoke<SingingResult>("convert_singing", {
        audioPath,
        modelName,
        pitchShift,
        separateVocals,
        f0Method,
        indexPath,
        indexRate,
    });
}

export async function onSingingProgress(callback: (event: SingingProgressEvent) => void): Promise<UnlistenFn> {
    return listen<SingingProgressEvent>("singing:progress", (event) => callback(event.payload));
}

// ── Telegram Bot ──────────────────────────────────

export interface TelegramConfig {
    enabled: boolean;
    bot_token?: string;
    bot_token_env?: string;
    allowed_chat_ids: number[];
    send_voice_reply: boolean;
    character_id?: string;
}

export interface TelegramStatus {
    running: boolean;
    enabled: boolean;
    has_token: boolean;
}

export async function setActiveCharacterId(id: string): Promise<void> {
    return invoke("set_active_character_id", { id });
}

export async function listCharacterIds(): Promise<string[]> {
    return invoke<string[]>("list_character_ids");
}

export async function getTelegramConfig(): Promise<TelegramConfig> {
    return invoke<TelegramConfig>("get_telegram_config");
}

export async function saveTelegramConfig(config: TelegramConfig): Promise<void> {
    return invoke("save_telegram_config", { config });
}

export async function startTelegramBot(): Promise<void> {
    return invoke("start_telegram_bot");
}

export async function stopTelegramBot(): Promise<void> {
    return invoke("stop_telegram_bot");
}

export async function getTelegramStatus(): Promise<TelegramStatus> {
    return invoke<TelegramStatus>("get_telegram_status");
}

export interface TelegramChatSync {
    role: string;
    text: string;
    translation?: string;
}

export async function onTelegramChatSync(callback: (data: TelegramChatSync) => void): Promise<UnlistenFn> {
    return listen<TelegramChatSync>("telegram:chat-sync", (event) => callback(event.payload));
}

// ── Backup / Restore ──────────────────────────────

export interface BackupStats {
    memories: number;
    conversations: number;
    messages: number;
    configs: number;
}

export interface ExportResult {
    path: string;
    size_bytes: number;
    stats: BackupStats;
}

export interface BackupManifest {
    version: string;
    created_at: string;
    app_version: string;
}

export interface ImportPreview {
    manifest: BackupManifest;
    has_database: boolean;
    has_configs: boolean;
    config_files: string[];
    stats: BackupStats;
}

export interface ImportOptions {
    import_database: boolean;
    import_configs: boolean;
    conflict_strategy: "skip" | "overwrite";
    target_character_id?: string;
}

export interface ImportResult {
    imported_memories: number;
    imported_conversations: number;
    imported_configs: number;
    characters_json?: string;
    debug_log?: string[];
}

export async function exportData(exportPath: string, charactersJson?: string): Promise<ExportResult> {
    return invoke<ExportResult>("export_data", { exportPath, charactersJson });
}

export async function previewImport(filePath: string): Promise<ImportPreview> {
    return invoke<ImportPreview>("preview_import", { filePath });
}

export async function importData(filePath: string, options: ImportOptions): Promise<ImportResult> {
    return invoke<ImportResult>("import_data", { filePath, options });
}

// ── Character CRUD (SQLite-backed) ────────────────

export interface CharacterRecord {
    id: string;
    name: string;
    persona: string;
    user_nickname: string;
    source_format: string;
    created_at: number;
    updated_at: number;
}

export async function listCharacters(): Promise<CharacterRecord[]> {
    return invoke<CharacterRecord[]>("list_characters");
}

export async function createCharacter(record: CharacterRecord): Promise<void> {
    return invoke("create_character", { request: record });
}

export async function updateCharacter(record: Omit<CharacterRecord, "created_at">): Promise<void> {
    return invoke("update_character", { request: record });
}

export async function deleteCharacter(id: string): Promise<void> {
    return invoke("delete_character", { id });
}

// ── Auto Backup ────────────────────────────────────

export interface AutoBackupConfig {
    enabled: boolean;
    backup_dir: string;
    interval_days: number;
    auto_cleanup: boolean;
    keep_days: number;
}

export async function getAutoBackupConfig(): Promise<AutoBackupConfig> {
    return invoke<AutoBackupConfig>("get_auto_backup_config");
}

export async function saveAutoBackupConfig(config: AutoBackupConfig): Promise<void> {
    return invoke("save_auto_backup_config", { config });
}

export async function runAutoBackupNow(): Promise<string> {
    return invoke<string>("run_auto_backup_now");
}

// ── Error Handling ──────────────────────────────────

/**
 * 结构化错误对象，对应后端 KokoroError
 */
export interface KokoroErrorObject {
    code: string;
    message: string;
    stage?: string;
    retryable?: boolean;
    trace_id?: string;
}

/**
 * 解析 Kokoro 错误，支持结构化 JSON 和裸字符串两种格式
 *
 * @param error - 来自后端的错误（可能是 JSON 字符串或裸字符串）
 * @returns 结构化错误对象或原始错误字符串
 */
export function parseKokoroError(error: unknown): KokoroErrorObject | string {
    if (typeof error !== "string") {
        if (error instanceof Error) return error.message;
        return String(error);
    }

    try {
        const parsed = JSON.parse(error);
        if (parsed.code && parsed.message) {
            return parsed as KokoroErrorObject;
        }
    } catch {
        // 不是 JSON，返回原始字符串
    }

    return error;
}

/**
 * 安全的 invoke 包装，自动处理错误解析
 *
 * @param cmd - 命令名称
 * @param args - 命令参数
 * @returns Promise，错误时拒绝并包含结构化错误信息
 */
export async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    return invoke<T>(cmd, args);
}

/**
 * 检查错误是否为特定类型（TypeScript 类型收窄谓词）
 *
 * @param error - 错误对象
 * @param code - 要检查的错误代码（限定为合法的 KokoroErrorObject["code"] 值）
 * @returns 是否匹配，匹配时 error 被收窄为 KokoroErrorObject
 */
export function isKokoroErrorCode(
    error: unknown,
    code: KokoroErrorObject["code"]
): error is KokoroErrorObject {
    if (typeof error === "object" && error !== null && "code" in error) {
        return (error as KokoroErrorObject).code === code;
    }
    return false;
}
