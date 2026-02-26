/**
 * Kokoro Engine — IPC Bridge
 * 
 * Typed wrapper around Tauri's invoke API.
 * All backend commands are accessed through this module.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ModManifest, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson } from "../core/types/mod";
export type { ModManifest, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson };

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
    current_expression: string;
    mood: number;
    is_speaking: boolean;
}

export interface ChatResponse {
    text: string;
    expression: string;
    mood_delta: number;
}

// ── System Commands ────────────────────────────────

export async function getEngineInfo(): Promise<EngineInfo> {
    return invoke<EngineInfo>("get_engine_info");
}

export async function getSystemStatus(): Promise<SystemStatus> {
    return invoke<SystemStatus>("get_system_status");
}

// ── Character Commands ─────────────────────────────

export async function getCharacterState(): Promise<CharacterState> {
    return invoke<CharacterState>("get_character_state");
}

export async function setExpression(expression: string): Promise<CharacterState> {
    return invoke<CharacterState>("set_expression", { expression });
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

export async function setResponseLanguage(language: string): Promise<void> {
    return invoke("set_response_language", { language });
}

export async function setUserLanguage(language: string): Promise<void> {
    return invoke("set_user_language", { language });
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

// ── LLM Config Management ──────────────────────────

export interface LlmProviderConfig {
    id: string;
    provider_type: string;
    enabled: boolean;
    api_key?: string;
    api_key_env?: string;
    base_url?: string;
    model?: string;
    extra?: Record<string, unknown>;
}

export interface LlmConfig {
    active_provider: string;
    system_provider?: string;
    system_model?: string;
    providers: LlmProviderConfig[];
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

export interface OllamaPullProgress {
    status: string;
    digest?: string;
    total?: number;
    completed?: number;
}

export async function pullOllamaModel(baseUrl: string, model: string): Promise<void> {
    return invoke("pull_ollama_model", { baseUrl, model });
}

export async function onOllamaPullProgress(callback: (p: OllamaPullProgress) => void): Promise<UnlistenFn> {
    return listen<OllamaPullProgress>("ollama:pull-progress", (event) => callback(event.payload));
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
    messages?: any[];
    /** If true, neither user message nor response is saved to chat history */
    hidden?: boolean;
}

export async function streamChat(request: ChatRequest): Promise<void> {
    return invoke("stream_chat", { request });
}

export async function onChatDelta(callback: (delta: string) => void): Promise<UnlistenFn> {
    return listen<string>("chat-delta", (event) => callback(event.payload));
}

export async function onChatError(callback: (error: string) => void): Promise<UnlistenFn> {
    return listen<string>("chat-error", (event) => callback(event.payload));
}

export async function onChatDone(callback: () => void): Promise<UnlistenFn> {
    return listen<void>("chat-done", () => callback());
}

// ── Expression Events ──────────────────────────────

export interface ExpressionEvent {
    expression: string;
    mood: number;
}

export async function onChatExpression(
    callback: (data: ExpressionEvent) => void
): Promise<UnlistenFn> {
    return listen<ExpressionEvent>("chat-expression", (event) => callback(event.payload));
}

// ── Action/Motion Events ───────────────────────────

export interface ActionEvent {
    action: string;
}

export async function onChatAction(
    callback: (data: ActionEvent) => void
): Promise<UnlistenFn> {
    return listen<ActionEvent>("chat-action", (event) => callback(event.payload));
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

export async function importLive2dZip(zipPath: string): Promise<string> {
    return invoke<string>("import_live2d_zip", { zipPath });
}

export async function listLive2dModels(): Promise<Live2dModelInfo[]> {
    return invoke<Live2dModelInfo[]>("list_live2d_models");
}

export async function deleteLive2dModel(modelName: string): Promise<void> {
    return invoke("delete_live2d_model", { modelName });
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

// ── STT (Speech-to-Text) ──────────────────────────────

export interface SttProviderConfig {
    id: string;
    provider_type: string;
    enabled: boolean;
    api_key?: string;
    api_key_env?: string;
    base_url?: string;
    model?: string;
}

export interface SttConfig {
    active_provider: string;
    language?: string;
    auto_send: boolean;
    providers: SttProviderConfig[];
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

// ── Actions (Tool Calling) ─────────────────────────────

export interface ActionInfo {
    name: string;
    description: string;
    parameters: { name: string; description: string; required: boolean }[];
}

export interface ActionResult {
    success: boolean;
    message: string;
    data?: unknown;
}

export interface ToolCallEvent {
    tool: string;
    result?: ActionResult;
    error?: string;
}

export async function listActions(): Promise<ActionInfo[]> {
    return invoke<ActionInfo[]>("list_actions");
}

export async function executeAction(name: string, args: Record<string, string>, characterId?: string): Promise<ActionResult> {
    return invoke<ActionResult>("execute_action", { name, args, characterId });
}

export async function onToolCallResult(callback: (event: ToolCallEvent) => void): Promise<UnlistenFn> {
    return listen<ToolCallEvent>("chat-tool-result", (event) => callback(event.payload));
}

// ── MCP (Model Context Protocol) ──────────────────────────

export interface McpServerConfig {
    name: string;
    command: string;
    args: string[];
    env: Record<string, string>;
    enabled: boolean;
}

export interface McpServerStatus {
    name: string;
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

// ── Conversation History ───────────────────────────────

export interface Conversation {
    id: string;
    character_id: string;
    title: string;
    created_at: string;
    updated_at: string;
}

export interface ConversationMessage {
    role: string;
    content: string;
    metadata?: string;
    created_at: string;
}

export async function listConversations(characterId: string): Promise<Conversation[]> {
    return invoke<Conversation[]>("list_conversations", {
        request: { character_id: characterId },
    });
}

export async function loadConversation(id: string): Promise<ConversationMessage[]> {
    return invoke<ConversationMessage[]>("load_conversation", {
        request: { id },
    });
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
