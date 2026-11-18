// pattern: Imperative Shell

/**
 * Kokoro Engine — IPC Bridge
 * 
 * Application adapters for the generated Tauri IPC contract.
 */
// Reason: This boundary module only declares IPC contracts and delegates side effects to Tauri.
import { commands, events, type JsonValue } from "@/lib/generated/ipc";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import type { ModManifest, TtsAudioFormat, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson } from "../core/types/mod";
export type { ModManifest, TtsAudioFormat, TtsConfig, ProviderStatus, VoiceProfile, TtsSystemConfig, ModThemeJson };

type IpcResponse<Command extends keyof typeof commands> = Awaited<ReturnType<(typeof commands)[Command]>>;
type IpcEventPayload<Event> = Event extends { listen(callback: EventCallback<infer Payload>): Promise<UnlistenFn> }
    ? Payload
    : never;

type PresentFields<Value, Key extends keyof Value> = Value & { [Field in Key]-?: NonNullable<Value[Field]> };

async function withKokoroError<T>(result: Promise<T>): Promise<T> {
    try {
        return await result;
    } catch (error) {
        // Preserve framework exceptions and normalize Rust and legacy rejection payloads.
        throw error instanceof Error ? error : parseKokoroError(error);
    }
}

// ── Types ──────────────────────────────────────────

export type EngineInfo = IpcResponse<"getEngineInfo">;

export type SystemStatus = IpcResponse<"getSystemStatus">;

export type CharacterState = IpcResponse<"getCharacterState">;

export type ChatResponse = IpcResponse<"sendMessage">;

// ── System Commands ────────────────────────────────

export async function getEngineInfo(): Promise<EngineInfo> {
    return withKokoroError(commands.getEngineInfo());
}

export async function getSystemStatus(): Promise<SystemStatus> {
    return withKokoroError(commands.getSystemStatus());
}

export async function setWindowSize(width: number, height: number): Promise<void> {
    await withKokoroError(commands.setWindowSize(width, height));
}

// ── Character Commands ─────────────────────────────

export async function getCharacterState(): Promise<CharacterState> {
    return withKokoroError(commands.getCharacterState());
}

export async function playCue(cue: string): Promise<CharacterState> {
    return withKokoroError(commands.playCue(cue));
}

// ── Database Commands ──────────────────────────────

export type DbTestResult = IpcResponse<"testVectorStore">;

export async function initDb(): Promise<string> {
    return withKokoroError(commands.initDb());
}

export async function testVectorStore(): Promise<DbTestResult> {
    return withKokoroError(commands.testVectorStore());
}

export async function sendMessage(message: string): Promise<ChatResponse> {
    return withKokoroError(commands.sendMessage(message));
}

// ── Context Management ─────────────────────────────

/** @deprecated Use character runtime overrides and serialized activation. */
export async function setPersona(prompt: string): Promise<void> {
    await withKokoroError(commands.setPersona(prompt));
}

/** @deprecated Partial character updates are rejected by the backend. */
export async function setCharacterName(name: string): Promise<void> {
    await withKokoroError(commands.setCharacterName(name));
}

export async function setUserName(name: string): Promise<void> {
    await withKokoroError(commands.setUserName(name));
}

export async function setUserPersona(persona: string): Promise<void> {
    await withKokoroError(commands.setUserPersona(persona));
}

/** @deprecated Partial character updates are rejected by the backend. */
export async function setResponseLanguage(language: string): Promise<void> {
    await withKokoroError(commands.setResponseLanguage(language));
}

export async function setUserLanguage(language: string): Promise<void> {
    await withKokoroError(commands.setUserLanguage(language));
}

export type UserProfileSettings = NonNullable<IpcResponse<"getUserProfileSettings">>;

export async function getUserProfileSettings(): Promise<UserProfileSettings | null> {
    return withKokoroError(commands.getUserProfileSettings());
}

export async function setJailbreakPrompt(prompt: string): Promise<void> {
    await withKokoroError(commands.setJailbreakPrompt(prompt));
}

export async function getJailbreakPrompt(): Promise<string> {
    return withKokoroError(commands.getJailbreakPrompt());
}

export async function setProactiveEnabled(enabled: boolean): Promise<void> {
    await withKokoroError(commands.setProactiveEnabled(enabled));
}

export async function getProactiveEnabled(): Promise<boolean> {
    return withKokoroError(commands.getProactiveEnabled());
}

export async function clearHistory(): Promise<void> {
    await withKokoroError(commands.clearHistory());
}

export async function setMemoryEnabled(enabled: boolean): Promise<void> {
    await withKokoroError(commands.setMemoryEnabled(enabled));
}

export async function getMemoryEnabled(): Promise<boolean> {
    return withKokoroError(commands.getMemoryEnabled());
}

// ── Context Settings ───────────────────────────────

export type ContextSettings = IpcResponse<"getContextSettings">;

export async function getContextSettings(): Promise<ContextSettings> {
    return withKokoroError(commands.getContextSettings());
}

export async function setContextSettings(settings: ContextSettings): Promise<void> {
    await withKokoroError(commands.setContextSettings(settings));
}

export async function deleteLastMessages(
    count: number,
    expectedConversationId?: string | null,
    expectedTailMessageId?: number | null,
    expectedHistory?: { generation: number; historyRevision: number; keepThroughId: number },
): Promise<void> {
    await withKokoroError(commands.deleteLastMessages(count, expectedConversationId ?? null, expectedTailMessageId ?? null, expectedHistory ?? null));
}

// ── LLM Config Management ──────────────────────────

type WireLlmConfig = Parameters<typeof commands.saveLlmConfig>[0];
type WireLlmProviderConfig = NonNullable<WireLlmConfig["providers"]>[number];

export type LlmProviderConfig = PresentFields<WireLlmProviderConfig, "enabled" | "supports_native_tools">;

type WireLlmPreset = NonNullable<WireLlmConfig["presets"]>[number];
export type LlmPreset = Omit<WireLlmPreset, "providers"> & { providers: LlmProviderConfig[] };

export type LlmConfig = Omit<PresentFields<WireLlmConfig, "active_provider">, "providers" | "presets"> & {
    providers: LlmProviderConfig[];
    presets?: LlmPreset[];
};

export type LlmConnectionTestedTarget = LlmConnectionTestResult["tested_targets"][number];

export type LlmConnectionTestResult = IpcResponse<"testLlmConnection">;

export type OllamaModelInfo = IpcResponse<"listOllamaModels">[number];

export type LlamaCppStatus = IpcResponse<"getLlamaCppStatus">;

export type CodexRuntimeInfo = IpcResponse<"getCodexRuntimeStatus">;

export async function getLlmConfig(): Promise<LlmConfig> {
    return withKokoroError(commands.getLlmConfig());
}

export async function saveLlmConfig(config: LlmConfig): Promise<void> {
    await withKokoroError(commands.saveLlmConfig(config));
}

export async function testLlmConnection(config: LlmConfig): Promise<LlmConnectionTestResult> {
    return withKokoroError(commands.testLlmConnection(config));
}

export async function listOllamaModels(baseUrl: string): Promise<OllamaModelInfo[]> {
    return withKokoroError(commands.listOllamaModels(baseUrl));
}

export async function listAnthropicModels(baseUrl: string, apiKey: string): Promise<string[]> {
    const models = await withKokoroError(commands.listAnthropicModels(baseUrl, apiKey));
    return models.map((model) => model.id).sort();
}

export async function getLlamaCppStatus(baseUrl: string): Promise<LlamaCppStatus> {
    return withKokoroError(commands.getLlamaCppStatus(baseUrl));
}

export async function getCodexRuntimeStatus(): Promise<CodexRuntimeInfo> {
    return withKokoroError(commands.getCodexRuntimeStatus());
}

export async function listCodexRuntimeModels(): Promise<string[]> {
    return withKokoroError(commands.listCodexRuntimeModels());
}

// ── LLM Streaming ──────────────────────────────────

export type ChatRequest = NonNullable<Parameters<typeof commands.streamChat>[0]>;

export type StreamChatResponse = IpcResponse<"streamChat">;

export async function streamChat(request: ChatRequest): Promise<StreamChatResponse> {
    return withKokoroError(commands.streamChat(request));
}

export function retryChatPersistence(clientRequestId: string): Promise<StreamChatResponse> {
    return withKokoroError(commands.retryChatPersistence(clientRequestId));
}

export type ChatTurnSnapshot = IpcResponse<"getChatRuntimeSnapshot">[number];
export type ChatViewVersion = NonNullable<Parameters<typeof commands.getChatView>[0]>;
export type ChatViewSnapshot = NonNullable<IpcResponse<"getChatView">>;
export function getChatRuntimeSnapshot(): Promise<ChatTurnSnapshot[]> {
    return withKokoroError(commands.getChatRuntimeSnapshot());
}
export function getChatView(): Promise<ChatViewSnapshot>;
export function getChatView(knownVersion: ChatViewVersion): Promise<ChatViewSnapshot | null>;
export function getChatView(knownVersion?: ChatViewVersion): Promise<ChatViewSnapshot | null> {
    return withKokoroError(commands.getChatView(knownVersion ?? null));
}
export type ChatDraftSubmissionStatus = IpcResponse<"recoverChatDraftSubmission">;

export function recoverChatDraftSubmission(characterId: string, clientRequestId: string): Promise<ChatDraftSubmissionStatus> {
    return withKokoroError(commands.recoverChatDraftSubmission(characterId, clientRequestId));
}
export function onChatRuntimeChanged(callback: (snapshot: ChatTurnSnapshot) => void): Promise<UnlistenFn> {
    return events.chatRuntimeChanged.listen(event => callback(event.payload));
}

export function finishChatPlayback(requestId: string, error?: string): Promise<void> {
    return withKokoroError(commands.finishChatPlayback(requestId, error ?? null));
}

export async function cancelChatTurn(turnId: string, reason?: string): Promise<void> {
    await withKokoroError(commands.cancelChatTurn(turnId, reason ?? null));
}

export async function isChatBusy(): Promise<boolean> {
    return withKokoroError(commands.isChatBusy());
}

export function startNativeMic(autoStopOnSilence: boolean, onlyWhenChatIdle: boolean): Promise<boolean> {
    return withKokoroError(commands.startNativeMic(autoStopOnSilence, onlyWhenChatIdle));
}

export function isChatTurnBusyError(error: unknown): boolean {
    const msg = getKokoroErrorMessage(error);
    return typeof msg === "string" && msg.includes("chat_turn_busy");
}

export async function onChatError(callback: (error: string) => void): Promise<UnlistenFn> {
    return events.chatError.listen((event) => callback(parseLegacyChatError(event.payload)));
}

export type ChatWarningEvent = IpcEventPayload<typeof events.chatWarning>;
export async function onChatWarning(callback: (warning: ChatWarningEvent) => void): Promise<UnlistenFn> {
    return events.chatWarning.listen((event) => callback(event.payload));
}

export interface FailureEventContext {
    deny_kind?: "hook_denied" | "policy_denied" | "fail_closed" | "pending_approval" | "execution_error";
    approval_status?: "requested" | "approved" | "rejected";
    [key: string]: unknown;
}

export type FailureEvent = Omit<IpcEventPayload<typeof events.chatFailure>, "context"> & {
    context?: FailureEventContext | null;
};

export async function onChatFailure(callback: (event: FailureEvent) => void): Promise<UnlistenFn> {
    return events.chatFailure.listen((event) => {
        callback(parseFailureEvent(event.payload) ?? createFallbackFailureEvent(parseLegacyChatError(event.payload)));
    });
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null;
}

function getStringField(record: Record<string, unknown>, key: string): string | undefined {
    const value = record[key];
    return typeof value === "string" ? value : undefined;
}

function stringifyUnknown(value: unknown): string {
    if (typeof value === "string") {
        return value;
    }

    if (value instanceof Error) {
        return value.message;
    }

    if (isRecord(value)) {
        try {
            return JSON.stringify(value);
        } catch {
            // Fall through to String for unusual host objects.
        }
    }

    return String(value);
}

function parseJsonPayload(payload: string): unknown | null {
    try {
        return JSON.parse(payload);
    } catch {
        return null;
    }
}

function createFallbackFailureEvent(message: string): FailureEvent {
    return {
        event_id: "",
        timestamp: "",
        domain: "chat",
        stage: "unknown",
        code: "CHAT_FAILURE",
        message,
        retryable: false,
        trace_id: "",
        conversation_id: null,
        turn_id: null,
        character_id: null,
    };
}

export function parseFailureEvent(payload: unknown): FailureEvent | null {
    if (typeof payload === "string") {
        const parsed = parseJsonPayload(payload);
        return parsed === null ? null : parseFailureEvent(parsed);
    }

    if (isRecord(payload)) {
        const code = getStringField(payload, "code");
        const message = getStringField(payload, "message");
        if (code && message) {
            return {
                event_id: getStringField(payload, "event_id") ?? "",
                timestamp: getStringField(payload, "timestamp") ?? "",
                domain: getStringField(payload, "domain") ?? "chat",
                stage: getStringField(payload, "stage") ?? "unknown",
                code,
                message,
                retryable: Boolean(payload.retryable),
                trace_id: getStringField(payload, "trace_id") ?? "",
                conversation_id: getStringField(payload, "conversation_id") ?? null,
                turn_id: getStringField(payload, "turn_id") ?? null,
                character_id: getStringField(payload, "character_id") ?? null,
                context: isRecord(payload.context) ? payload.context : null,
            };
        }
    }

    return null;
}

export function parseLegacyChatError(payload: unknown): string {
    if (typeof payload === "string") {
        return parseFailureEvent(payload)?.message ?? payload;
    }

    if (payload instanceof Error) {
        return payload.message;
    }

    const failure = parseFailureEvent(payload);
    if (failure) {
        return failure.message;
    }

    if (isRecord(payload)) {
        const message = getStringField(payload, "message") ?? getStringField(payload, "error");
        if (message) {
            return message;
        }
    }

    return stringifyUnknown(payload);
}

export type ChatTurnAcknowledgedEvent = IpcEventPayload<typeof events.chatTurnAcknowledged>;

export type ChatTurnStartEvent = IpcEventPayload<typeof events.chatTurnStart>;

export type ChatTurnDeltaEvent = IpcEventPayload<typeof events.chatTurnDelta>;

export type ChatTurnFinishEvent = IpcEventPayload<typeof events.chatTurnFinish>;

export type ChatTurnTranslationEvent = IpcEventPayload<typeof events.chatTurnTranslation>;

export type ChatTurnTextCompleteEvent = IpcEventPayload<typeof events.chatTurnTextComplete>;

export interface ToolTraceItem {
    tool: string;
    toolName?: string;
    toolId?: string;
    toolTraceId?: string;
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

export type ChatTurnToolEvent = IpcEventPayload<typeof events.chatTurnTool>;

export async function onChatTurnAcknowledged(callback: (event: ChatTurnAcknowledgedEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnAcknowledged.listen((event) => callback(event.payload));
}

export async function onChatTurnStart(callback: (event: ChatTurnStartEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnStart.listen((event) => callback(event.payload));
}

export async function onChatTurnDelta(callback: (event: ChatTurnDeltaEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnDelta.listen((event) => callback(event.payload));
}

export async function onChatTurnFinish(callback: (event: ChatTurnFinishEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnFinish.listen((event) => callback(event.payload));
}

export async function onChatTurnTextComplete(callback: (event: ChatTurnTextCompleteEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnTextComplete.listen((event) => callback(event.payload));
}

export async function onChatTurnTranslation(callback: (event: ChatTurnTranslationEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnTranslation.listen((event) => callback(event.payload));
}

// ── Cue Events ─────────────────────────────────────

export type CueEvent = IpcEventPayload<typeof events.chatCue>;

export async function onChatCue(
    callback: (data: CueEvent) => void
): Promise<UnlistenFn> {
    return events.chatCue.listen((event) => callback(event.payload));
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
    // We want the base, usually ending in /v1.
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
    return withKokoroError(commands.listMods());
}

export async function loadMod(modId: string): Promise<void> {
    await withKokoroError(commands.loadMod(modId));
}

export async function installMod(filePath: string): Promise<ModManifest> {
    return withKokoroError(commands.installMod(filePath));
}

export async function installModFromUrl(url: string, confirmUntrustedCode: boolean): Promise<ModManifest> {
    return withKokoroError(commands.installModFromUrl(url, confirmUntrustedCode));
}

export async function removeMod(modId: string): Promise<void> {
    await withKokoroError(commands.removeMod(modId));
}

export type RegistryModInstallRequest = {
    readonly entry: Readonly<RegistryEntry>;
    readonly registryUrl?: string | null;
    readonly permissionConfirmed: boolean;
};

export async function installModFromRegistry(
    request: Readonly<RegistryModInstallRequest>,
): Promise<ModManifest> {
    return withKokoroError(commands.installModFromRegistry(request.entry, request.registryUrl ?? null, request.permissionConfirmed));
}

export async function getModTheme(): Promise<ModThemeJson | null> {
    return withKokoroError(commands.getModTheme());
}

export async function getModLayout(): Promise<unknown | null> {
    return withKokoroError(commands.getModLayout());
}

// ── Mod Events ─────────────────────────────────────

export async function onModThemeOverride(
    callback: (theme: ModThemeJson) => void
): Promise<UnlistenFn> {
    return events.modThemeOverride.listen((event) => callback(event.payload));
}

export async function onModLayoutOverride(
    callback: (layout: unknown) => void
): Promise<UnlistenFn> {
    return events.modLayoutOverride.listen((event) => callback(event.payload));
}

export async function onModComponentsRegister(
    callback: (components: Record<string, string>) => void
): Promise<UnlistenFn> {
    return events.modComponentsRegister.listen((event) => callback(event.payload));
}

export async function onModUiMessage(
    callback: (data: { component: string; payload: unknown }) => void
): Promise<UnlistenFn> {
    return events.modUiMessage.listen((event) => callback(event.payload));
}

export async function dispatchModEvent(event: string, payload: unknown): Promise<void> {
    await withKokoroError(commands.dispatchModEvent(event, toIpcJson(payload)));
}

export function toIpcJson(value: unknown): JsonValue {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) throw new TypeError("IPC requires a JSON payload");
    const decoded: unknown = JSON.parse(encoded);
    if (!isJsonValue(decoded)) throw new TypeError("IPC requires a JSON payload");
    return decoded;
}

function isJsonValue(value: unknown): value is JsonValue {
    if (value === null || typeof value === "string" || typeof value === "boolean" || typeof value === "number") return true;
    if (Array.isArray(value)) return value.every(isJsonValue);
    return isRecord(value) && Object.values(value).every(isJsonValue);
}

export async function unloadMod(): Promise<void> {
    await withKokoroError(commands.unloadMod());
}

export async function onModUnload(callback: () => void): Promise<UnlistenFn> {
    return events.modUnload.listen(() => callback());
}

export async function onModScriptEvent(
    callback: (data: { event: string; payload: unknown }) => void
): Promise<UnlistenFn> {
    return events.modScriptEvent.listen((e) => callback(e.payload));
}

// ── Live2D Model Import ─────────────────────────────

export type Live2dModelInfo = IpcResponse<"listLive2dModels">[number];

export type Live2dCueBinding = NonNullable<Live2dModelProfile["cue_map"][string]>;

export const BUILTIN_LIVE2D_MODEL_PATH = "__builtin__/hiyori/hiyori_pro_t11.model3.json";

export type Live2dModelProfile = IpcResponse<"getLive2dModelProfile">;

export async function importLive2dZip(zipPath: string): Promise<string> {
    return withKokoroError(commands.importLive2dZip(zipPath));
}

export async function importLive2dFolder(modelJsonPath: string): Promise<string> {
    return withKokoroError(commands.importLive2dFolder(modelJsonPath));
}

export async function exportLive2dModel(modelPath: string, exportPath: string): Promise<string> {
    return withKokoroError(commands.exportLive2dModel(modelPath, exportPath));
}

export async function listLive2dModels(): Promise<Live2dModelInfo[]> {
    return withKokoroError(commands.listLive2dModels());
}

export async function deleteLive2dModel(modelName: string): Promise<void> {
    await withKokoroError(commands.deleteLive2dModel(modelName));
}

export async function renameLive2dModel(modelPath: string, newName: string): Promise<string> {
    return withKokoroError(commands.renameLive2dModel(modelPath, newName));
}

export async function getLive2dModelProfile(modelPath: string): Promise<Live2dModelProfile> {
    return withKokoroError(commands.getLive2dModelProfile(modelPath));
}

export async function saveLive2dModelProfile(profile: Live2dModelProfile): Promise<Live2dModelProfile> {
    return withKokoroError(commands.saveLive2dModelProfile(profile));
}

export async function setActiveLive2dModel(modelPath: string | null): Promise<void> {
    await withKokoroError(commands.setActiveLive2dModel(modelPath));
}

// ── TTS ────────────────────────────────────────────

let latestTtsRequestId: string | null = null;

export async function synthesize(text: string, config: TtsConfig): Promise<void> {
    const requestId = config.request_id || createTtsRequestId();
    const replacedRequestId = latestTtsRequestId;
    latestTtsRequestId = requestId;
    if (typeof window !== "undefined") {
        window.dispatchEvent(new CustomEvent("kokoro-tts-replacing", {
            detail: { request_id: requestId, replaced_request_id: replacedRequestId },
        }));
    }
    await withKokoroError(commands.synthesize(text, { ...config, request_id: requestId }));
}

export function getLatestTtsRequestId(): string | null {
    return latestTtsRequestId;
}

function createTtsRequestId(): string {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
        return crypto.randomUUID();
    }
    return `tts-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export async function cancelTts(requestId?: string | null): Promise<void> {
    await withKokoroError(commands.cancelTts(requestId ?? null));
}

/** Automatic speech follows this view until selection changes. Turn identity stays immutable. */
export type ChatPlaybackScope = NonNullable<IpcEventPayload<typeof events.ttsStart>["chat_scope"]>;

export type TtsPlaybackErrorEvent = IpcEventPayload<typeof events.ttsPlaybackError>;

export async function onTtsPlaybackError(
    callback: (event: TtsPlaybackErrorEvent) => void,
): Promise<UnlistenFn> {
    return events.ttsPlaybackError.listen(event => callback(event.payload));
}

export async function listTtsProviders(): Promise<ProviderStatus[]> {
    return withKokoroError(commands.listTtsProviders());
}

export async function listTtsVoices(): Promise<VoiceProfile[]> {
    return withKokoroError(commands.listTtsVoices());
}

export async function listOpenAITtsModels(
    config: TtsSystemConfig["providers"][number],
): Promise<Array<string>> {
    return withKokoroError(commands.listOpenaiTtsModels(config));
}

export async function getTtsProviderStatus(providerId: string): Promise<ProviderStatus | null> {
    return withKokoroError(commands.getTtsProviderStatus(providerId));
}

export async function clearTtsCache(): Promise<void> {
    await withKokoroError(commands.clearTtsCache());
}

export async function getTtsConfig(): Promise<TtsSystemConfig> {
    return withKokoroError(commands.getTtsConfig());
}

export async function saveTtsConfig(config: TtsSystemConfig): Promise<void> {
    await withKokoroError(commands.saveTtsConfig(config));
}

export type GptSovitsModels = IpcResponse<"listGptSovitsModels">;

export async function listGptSovitsModels(installPath: string): Promise<GptSovitsModels> {
    return withKokoroError(commands.listGptSovitsModels(installPath));
}

// ── Image Generation ───────────────────────────────

export type ImageGenResult = IpcResponse<"generateImage">;
export type ImageGenDone = IpcEventPayload<typeof events.imagegenDone>;

type WireImageGenSystemConfig = Parameters<typeof commands.saveImagegenConfig>[0];
type WireImageGenProviderConfig = NonNullable<WireImageGenSystemConfig["providers"]>[number];

export type ImageGenProviderConfig = PresentFields<WireImageGenProviderConfig, "enabled">;
export type ImageGenSystemConfig = Omit<PresentFields<WireImageGenSystemConfig, "enabled">, "providers"> & {
    providers: ImageGenProviderConfig[];
};

export async function generateImage(prompt: string, providerId?: string): Promise<ImageGenResult> {
    return withKokoroError(commands.generateImage(prompt, providerId ?? null, null));
}

export async function getImageGenConfig(): Promise<ImageGenSystemConfig> {
    return withKokoroError(commands.getImagegenConfig());
}

export async function saveImageGenConfig(config: ImageGenSystemConfig): Promise<void> {
    await withKokoroError(commands.saveImagegenConfig(config));
}

export async function testSdConnection(baseUrl: string): Promise<string[]> {
    return withKokoroError(commands.testSdConnection(baseUrl));
}

// ── Image Gen Events ──────────────────────────────

export type ChatImageGenEvent = IpcEventPayload<typeof events.chatImagegen>;

export async function onChatImageGen(callback: (event: ChatImageGenEvent) => void): Promise<UnlistenFn> {
    return events.chatImagegen.listen((e) => callback(e.payload));
}

export async function onImageGenDone(callback: (event: ImageGenDone) => void): Promise<UnlistenFn> {
    return events.imagegenDone.listen((e) => callback(e.payload));
}

export type ChatImageGenError = IpcEventPayload<typeof events.chatImagegenError>;
export async function onChatImageGenError(callback: (event: ChatImageGenError) => void): Promise<UnlistenFn> {
    return events.chatImagegenError.listen(event => callback(event.payload));
}

export async function onImageGenError(callback: (error: string) => void): Promise<UnlistenFn> {
    return events.imagegenError.listen((e) => callback(e.payload));
}

// ── Vision Upload ──────────────────────────────────

export async function uploadVisionImage(fileBytes: number[], filename: string): Promise<string> {
    return withKokoroError(commands.uploadVisionImage(fileBytes, filename));
}

// ── Vision Config & Watcher ────────────────────────

export type VisionConfig = IpcResponse<"getVisionConfig">;

export type VisionScreenInfo = IpcResponse<"listVisionScreens">[number];

export async function getVisionConfig(): Promise<VisionConfig> {
    return withKokoroError(commands.getVisionConfig());
}

export async function listVisionScreens(): Promise<VisionScreenInfo[]> {
    return withKokoroError(commands.listVisionScreens());
}

export async function saveVisionConfig(config: VisionConfig): Promise<void> {
    await withKokoroError(commands.saveVisionConfig(config));
}

export async function setVisionTextInputFocused(focused: boolean): Promise<void> {
    await withKokoroError(commands.setVisionTextInputFocused(focused));
}

export async function captureScreenNow(): Promise<string> {
    return withKokoroError(commands.captureScreenNow());
}

type VisionObservationPayload = IpcEventPayload<typeof events.visionObservation>;
export type VisionObservationEvent = Pick<VisionObservationPayload, "summary">
    & Partial<Omit<VisionObservationPayload, "summary">>;

export async function onVisionObservation(callback: (event: VisionObservationEvent) => void): Promise<UnlistenFn> {
    return events.visionObservation.listen((event) => {
        if (typeof event.payload === "string") {
            callback({ summary: event.payload });
        } else {
            callback(event.payload);
        }
    });
}

export async function onCameraObservation(callback: (desc: string) => void): Promise<UnlistenFn> {
    return events.cameraObservation.listen((event) => callback(event.payload));
}

// ── Memory Management ──────────────────────────────

export type MemoryRecord = ListMemoriesResponse["memories"][number];

export type ListMemoriesResponse = IpcResponse<"listMemories">;

export type MemoryUpgradeConfig = IpcResponse<"getMemoryUpgradeConfig">;

export type MemoryObservabilitySummary = IpcResponse<"getMemoryObservabilitySummary">;

export type MemoryWriteEventRecord = NonNullable<IpcResponse<"getLatestMemoryWriteEvent">>;

export type MemoryRetrievalLogRecord = NonNullable<IpcResponse<"getLatestMemoryRetrievalLog">>;

export type MemoryRetrievalEvalSummary = NonNullable<IpcResponse<"getLatestMemoryRetrievalEvalSummary">>;

export type MemoryDreamJobRecord = IpcResponse<"listDreamJobs">[number];

export type MemoryDreamSourceRecord = MemoryDreamProposalRecord["source_memories"][number];

export type MemoryDreamProposalRecord = IpcResponse<"listDreamProposals">[number];

export type MemoryDreamingSummary = IpcResponse<"getDreamingSummary">;

export type MemoryDreamRunResult = IpcResponse<"runDreamNow">;

export type MemoryEmbeddingModelStatus = IpcResponse<"getMemoryEmbeddingModelStatus">;

export type MemoryEmbeddingModelDownloadProgress = IpcEventPayload<typeof events.memoryEmbeddingModelProgress>;

export async function getMemoryEmbeddingModelStatus(): Promise<MemoryEmbeddingModelStatus> {
    return withKokoroError(commands.getMemoryEmbeddingModelStatus());
}

export async function downloadMemoryEmbeddingModel(): Promise<MemoryEmbeddingModelStatus> {
    return withKokoroError(commands.downloadMemoryEmbeddingModel());
}

export async function onMemoryEmbeddingModelProgress(
    callback: (progress: MemoryEmbeddingModelDownloadProgress) => void
): Promise<UnlistenFn> {
    return events.memoryEmbeddingModelProgress.listen((event) => callback(event.payload));
}

export async function listMemories(characterId: string, limit = 50, offset = 0): Promise<ListMemoriesResponse> {
    return withKokoroError(commands.listMemories({ character_id: characterId, limit, offset }));
}

export async function updateMemory(characterId: string, id: number, content: string, importance: number): Promise<void> {
    await withKokoroError(commands.updateMemory({ character_id: characterId, id, content, importance }));
}

export async function deleteMemory(characterId: string, id: number): Promise<void> {
    await withKokoroError(commands.deleteMemory({ character_id: characterId, id }));
}

export async function updateMemoryTier(characterId: string, id: number, tier: string): Promise<void> {
    await withKokoroError(commands.updateMemoryTier({ character_id: characterId, id, tier }));
}

export async function getMemoryUpgradeConfig(): Promise<MemoryUpgradeConfig> {
    return withKokoroError(commands.getMemoryUpgradeConfig());
}

export async function setMemoryUpgradeConfig(config: Readonly<MemoryUpgradeConfig>): Promise<void> {
    await withKokoroError(commands.setMemoryUpgradeConfig(config));
}

export async function getMemoryObservabilitySummary(): Promise<MemoryObservabilitySummary> {
    return withKokoroError(commands.getMemoryObservabilitySummary());
}

export async function getLatestMemoryWriteEvent(): Promise<MemoryWriteEventRecord | null> {
    return withKokoroError(commands.getLatestMemoryWriteEvent());
}

export async function getLatestMemoryRetrievalLog(): Promise<MemoryRetrievalLogRecord | null> {
    return withKokoroError(commands.getLatestMemoryRetrievalLog());
}

export async function getLatestMemoryRetrievalEvalSummary(): Promise<MemoryRetrievalEvalSummary | null> {
    return withKokoroError(commands.getLatestMemoryRetrievalEvalSummary());
}

export async function runDreamNow(characterId: string): Promise<MemoryDreamRunResult> {
    return withKokoroError(commands.runDreamNow({ character_id: characterId }));
}

export async function getDreamingSummary(characterId: string): Promise<MemoryDreamingSummary> {
    return withKokoroError(commands.getDreamingSummary({ character_id: characterId }));
}

export async function listDreamJobs(characterId: string, limit = 20): Promise<MemoryDreamJobRecord[]> {
    return withKokoroError(commands.listDreamJobs({ character_id: characterId, limit }));
}

export async function listDreamProposals(
    characterId: string,
    status = "pending",
    limit = 50,
): Promise<MemoryDreamProposalRecord[]> {
    return withKokoroError(commands.listDreamProposals({ character_id: characterId, status, limit }));
}

export async function approveDreamProposal(id: number): Promise<void> {
    await withKokoroError(commands.approveDreamProposal({ id }));
}

export async function rejectDreamProposal(id: number): Promise<void> {
    await withKokoroError(commands.rejectDreamProposal({ id }));
}

// ── STT (Speech-to-Text) ──────────────────────────────

type WireSttConfig = Parameters<typeof commands.saveSttConfig>[0];
type WireSttProviderConfig = NonNullable<WireSttConfig["providers"]>[number];
export type SttProviderConfig = PresentFields<WireSttProviderConfig, "enabled">;
export type SttConfig = Omit<PresentFields<WireSttConfig, "active_provider" | "auto_send" | "wake_word_enabled" | "continuous_listening">, "providers"> & {
    providers: SttProviderConfig[];
};

export type SenseVoiceLocalModelStatus = IpcResponse<"getSensevoiceLocalStatus">;

export type SenseVoiceLocalDownloadProgress = IpcEventPayload<typeof events.sttSensevoiceLocalProgress>;

export async function transcribeAudio(audioBytes: number[], format: string): Promise<string> {
    return withKokoroError(commands.transcribeAudio(audioBytes, format));
}

export async function getSttConfig(): Promise<SttConfig> {
    return withKokoroError(commands.getSttConfig());
}

export async function saveSttConfig(config: SttConfig): Promise<void> {
    await withKokoroError(commands.saveSttConfig(config));
}

export async function getSenseVoiceLocalStatus(): Promise<SenseVoiceLocalModelStatus> {
    return withKokoroError(commands.getSensevoiceLocalStatus());
}

export async function downloadSenseVoiceLocalModel(): Promise<SenseVoiceLocalModelStatus> {
    return withKokoroError(commands.downloadSensevoiceLocalModel());
}

export async function onSenseVoiceLocalProgress(
    callback: (progress: SenseVoiceLocalDownloadProgress) => void
): Promise<UnlistenFn> {
    return events.sttSensevoiceLocalProgress.listen((event) => callback(event.payload));
}

// ── Actions (Tool Calling) ─────────────────────────────

export type ActionInfo = IpcResponse<"listActions">[number];

export type ActionResult = IpcResponse<"executeAction">;

export type ToolSettings = IpcResponse<"getToolSettings">;

export async function listActions(): Promise<ActionInfo[]> {
    return withKokoroError(commands.listActions());
}

export async function listBuiltinTools(): Promise<ActionInfo[]> {
    return withKokoroError(commands.listBuiltinTools());
}

export async function executeAction(name: string, args: Record<string, string>, characterId?: string): Promise<ActionResult> {
    return withKokoroError(commands.executeAction(name, args, characterId ?? null));
}

export async function onChatTurnTool(callback: (event: ChatTurnToolEvent) => void): Promise<UnlistenFn> {
    return events.chatTurnTool.listen((event) => callback(event.payload));
}

export async function getToolSettings(): Promise<ToolSettings> {
    return withKokoroError(commands.getToolSettings());
}

export async function saveToolSettings(settings: ToolSettings): Promise<void> {
    await withKokoroError(commands.saveToolSettings(settings));
}

export async function approveToolApproval(approvalRequestId: string): Promise<void> {
    await withKokoroError(commands.approveToolApproval(approvalRequestId));
}

export async function rejectToolApproval(approvalRequestId: string, reason: string | null = null): Promise<void> {
    await withKokoroError(commands.rejectToolApproval(approvalRequestId, reason));
}

// ── MCP (Model Context Protocol) ──────────────────────────

export type McpServerConfig = NonNullable<Parameters<typeof commands.addMcpServer>[0]>;

export type McpServerStatus = IpcResponse<"listMcpServers">[number];

export async function listMcpServers(): Promise<McpServerStatus[]> {
    return withKokoroError(commands.listMcpServers());
}

export async function addMcpServer(config: McpServerConfig): Promise<void> {
    await withKokoroError(commands.addMcpServer(config));
}

export async function removeMcpServer(name: string): Promise<void> {
    await withKokoroError(commands.removeMcpServer(name));
}

export async function refreshMcpTools(): Promise<void> {
    await withKokoroError(commands.refreshMcpTools());
}

export async function reconnectMcpServer(name: string): Promise<void> {
    await withKokoroError(commands.reconnectMcpServer(name));
}

export async function toggleMcpServer(name: string, enabled: boolean): Promise<void> {
    await withKokoroError(commands.toggleMcpServer(name, enabled));
}

// ── Conversation History ───────────────────────────────

export type Conversation = IpcResponse<"listConversations">[number];

export type ConversationMessage = LoadedConversation["messages"][number];

export type LoadedConversation = IpcResponse<"loadConversation">;

export type EditConversationMessageRequest = NonNullable<Parameters<typeof commands.editConversationMessage>[0]>;

export type EditConversationMessageResponse = IpcResponse<"editConversationMessage">;

export async function editConversationMessage(
    request: EditConversationMessageRequest,
): Promise<EditConversationMessageResponse> {
    return withKokoroError(commands.editConversationMessage(request));
}

export async function listConversations(characterId: string): Promise<Conversation[]> {
    return withKokoroError(commands.listConversations({ character_id: characterId }));
}

export async function loadConversation(id: string): Promise<LoadedConversation> {
    return withKokoroError(commands.loadConversation({ id }));
}

export async function updateConversationState(
    id: string,
    patch: { topic?: string; pinned_state?: string }
): Promise<void> {
    await withKokoroError(commands.updateConversationState({ id, ...patch }));
}

export function hasPinnedConversationState(pinnedState: string): boolean {
    const normalized = pinnedState.trim();
    if (!normalized || normalized === "{}") return false;
    try {
        const parsed = JSON.parse(normalized);
        if (typeof parsed === "object" && parsed !== null) {
            if ("pinned" in parsed) {
                return Boolean(parsed.pinned);
            }
            return false;
        }
        return false;
    } catch {
        return false;
    }
}

export function getConversationDisplayTitle(conversation: Conversation): string {
    return conversation.topic.trim() || conversation.title;
}

export async function deleteConversation(id: string): Promise<void> {
    await withKokoroError(commands.deleteConversation({ id }));
}

export async function createConversation(): Promise<string> {
    return withKokoroError(commands.createConversation());
}

export async function renameConversation(id: string, title: string): Promise<void> {
    await withKokoroError(commands.renameConversation({ id, title }));
}

// ── Bot Integrations ───────────────────────────────

export type BotPlatformId = keyof BotStatus;

type WireBotConfig = Parameters<typeof commands.saveBotConfig>[0];

export type QQBotConfig = PresentFields<NonNullable<WireBotConfig["qq"]>,
    "enabled" | "allow_c2c" | "allowed_user_openids" | "allowed_group_openids">;

export type QQAuthorizationRequest = IpcEventPayload<typeof events.qqAuthorizationRequest>;

export type QQAuthorizationResponse = IpcResponse<"respondQqAuthorization">;

export type DiscordBotConfig = PresentFields<NonNullable<WireBotConfig["discord"]>,
    "enabled" | "allowed_channel_ids" | "allow_direct_messages" | "send_voice_reply">;

export type LineBotConfig = PresentFields<NonNullable<WireBotConfig["line"]>, "enabled" | "webhook_path" | "allowed_user_ids">;

export type WebhookBotConfig = PresentFields<NonNullable<WireBotConfig["webhook"]>,
    "enabled" | "bind_host" | "port" | "endpoint_path" | "send_voice_reply">;

export type BotConfig = PresentFields<WireBotConfig, "revision" | "selected_platform"> & {
    telegram: TelegramConfig;
    qq: QQBotConfig;
    discord: DiscordBotConfig;
    line: LineBotConfig;
    webhook: WebhookBotConfig;
};

export type BotPlatformStatus = BotStatus["qq"];

export type BotStatus = IpcResponse<"getBotStatus">;

function withFixedBotEnv(config: BotConfig): BotConfig {
    return {
        ...config,
        telegram: {
            ...config.telegram,
            bot_token_env: "TELEGRAM_BOT_TOKEN",
        },
        qq: {
            ...config.qq,
            app_id_env: "QQBOT_APP_ID",
            app_secret_env: "QQBOT_APP_SECRET",
        },
        discord: {
            ...config.discord,
            bot_token_env: "DISCORD_BOT_TOKEN",
        },
        line: {
            ...config.line,
            channel_access_token_env: "LINE_CHANNEL_ACCESS_TOKEN",
            channel_secret_env: "LINE_CHANNEL_SECRET",
        },
        webhook: {
            ...config.webhook,
            bearer_token_env: "KOKORO_WEBHOOK_TOKEN",
        },
    };
}

export async function getBotConfig(): Promise<BotConfig> {
    return withKokoroError(commands.getBotConfig());
}

export async function saveBotConfig(config: BotConfig): Promise<BotConfig> {
    return withKokoroError(commands.saveBotConfig(withFixedBotEnv(config)));
}

export async function respondQQAuthorization(
    request: Readonly<QQAuthorizationRequest>,
    approved: boolean,
): Promise<QQAuthorizationResponse> {
    return withKokoroError(commands.respondQqAuthorization(request.request_id, request.user_openid, request.group_openid ?? null, approved));
}

export async function startBotPlatform(platform: Exclude<BotPlatformId, "telegram">): Promise<void> {
    await withKokoroError(commands.startBotPlatform(platform));
}

export async function stopBotPlatform(platform: Exclude<BotPlatformId, "telegram">): Promise<void> {
    await withKokoroError(commands.stopBotPlatform(platform));
}

export async function getBotStatus(): Promise<BotStatus> {
    return withKokoroError(commands.getBotStatus());
}

// ── Telegram Bot ──────────────────────────────────

type WireTelegramConfig = Parameters<typeof commands.saveTelegramConfig>[0];
export type TelegramConfig = PresentFields<WireTelegramConfig, "enabled" | "allowed_chat_ids" | "send_voice_reply">;

export type TelegramStatus = IpcResponse<"getTelegramStatus">;

/** @deprecated Partial character updates are rejected by the backend. */
export async function setActiveCharacterId(id: string): Promise<void> {
    await withKokoroError(commands.setActiveCharacterId(id));
}

export async function listCharacterIds(): Promise<string[]> {
    return withKokoroError(commands.listCharacterIds());
}

export async function getTelegramConfig(): Promise<TelegramConfig> {
    return withKokoroError(commands.getTelegramConfig());
}

export async function saveTelegramConfig(config: TelegramConfig): Promise<void> {
    await withKokoroError(commands.saveTelegramConfig(config));
}

export async function startTelegramBot(): Promise<void> {
    await withKokoroError(commands.startTelegramBot());
}

export async function stopTelegramBot(): Promise<void> {
    await withKokoroError(commands.stopTelegramBot());
}

export async function getTelegramStatus(): Promise<TelegramStatus> {
    return withKokoroError(commands.getTelegramStatus());
}

export type TelegramChatSync = IpcEventPayload<typeof events.telegramChatSync>;

export async function onTelegramChatSync(callback: (data: TelegramChatSync) => void): Promise<UnlistenFn> {
    return events.telegramChatSync.listen((event) => callback(event.payload));
}

// ── Backup / Restore ──────────────────────────────

export type BackupStats = ExportResult["stats"];

export type ExportResult = IpcResponse<"exportData">;

export type BackupManifest = ImportPreview["manifest"];

/** One character instance stored inside a backup. */
export type BackupCharacterSummary = ImportPreview["characters"][number];

export type ImportPreview = IpcResponse<"previewImport">;

/** Routes the rows of one imported character into an existing local character. */
export type CharacterMerge = NonNullable<ImportOptions["character_merges"]>[number];

export type ImportOptions = NonNullable<Parameters<typeof commands.importData>[1]>;

export type ImportResult = IpcResponse<"importData">;

export type ManualBackupExportOptions = NonNullable<Parameters<typeof commands.exportData>[2]>;

export async function exportData(
    exportPath: string,
    options: Readonly<ManualBackupExportOptions>,
): Promise<ExportResult> {
    return withKokoroError(commands.exportData(exportPath, null, options));
}

export async function previewImport(filePath: string): Promise<ImportPreview> {
    return withKokoroError(commands.previewImport(filePath));
}

export async function importData(filePath: string, options: ImportOptions): Promise<ImportResult> {
    return withKokoroError(commands.importData(filePath, options));
}

// ── Character CRUD (SQLite-backed) ────────────────

export type CharacterRecord = IpcResponse<"listCharacters">[number];

export type CharacterRuntimeProfile = CharacterTemplateManifest["runtime"];

export type ResolvedTtsMode = ResolvedCharacterTts["mode"];

export type ResolvedCharacterTts = BackendCharacterRuntime["tts"];

export type PackageAssetReference = NonNullable<BackendCharacterRuntime["live2d_model"]>;

export type BackendCharacterRuntime = CommittedCharacterRuntime["runtime"];

export type CharacterActivationToken = IpcResponse<"prepareCharacterActivation">;

export type CommittedCharacterRuntime = IpcResponse<"commitCharacterActivation">;

export type PrepareCharacterActivationOptions = {
    readonly allowLocalPreset: boolean;
};

export type CharacterTemplateManifest = IpcResponse<"listCharacterTemplates">[number];

export type DuplicateCharacterRequest = NonNullable<Parameters<typeof commands.duplicateCharacter>[0]>;

export type RestoreCharacterDefaultsRequest = NonNullable<Parameters<typeof commands.restoreCharacterDefaults>[0]>;

export type InstantiateCharacterTemplateRequest = NonNullable<Parameters<typeof commands.instantiateCharacterTemplate>[0]>;

export type ReconcileCharacterTemplateRequest = NonNullable<Parameters<typeof commands.reconcileCharacterTemplate>[0]>;

export type ReconcileConflict = ReconcileCharacterTemplatePreview["conflicts"][number];

export type CharacterTemplateFields = ReconcileCharacterTemplatePreview["merged"];

export type ReconcileCharacterTemplatePreview = IpcResponse<"reconcileCharacterTemplate">;

export type ApplyCharacterTemplateReconciliationRequest = NonNullable<Parameters<typeof commands.applyCharacterTemplateReconciliation>[0]>;

export async function listCharacters(): Promise<CharacterRecord[]> {
    return withKokoroError(commands.listCharacters());
}

export async function createCharacter(record: Parameters<typeof commands.createCharacter>[0]): Promise<void> {
    await withKokoroError(commands.createCharacter(record));
}

export async function prepareCharacterActivation(
    characterId: string,
    options?: Readonly<PrepareCharacterActivationOptions>,
): Promise<CharacterActivationToken> {
    return withKokoroError(commands.prepareCharacterActivation(characterId, options?.allowLocalPreset ?? null));
}

export async function commitCharacterActivation(
    token: Readonly<CharacterActivationToken>,
): Promise<CommittedCharacterRuntime> {
    return withKokoroError(commands.commitCharacterActivation(token));
}

export async function getCommittedCharacterRuntime(): Promise<CommittedCharacterRuntime | null> {
    return withKokoroError(commands.getCommittedCharacterRuntime());
}

export async function createCharacterWithAvatar(
    record: Parameters<typeof commands.createCharacterWithAvatar>[0],
    bytes: Uint8Array,
): Promise<void> {
    await withKokoroError(commands.createCharacterWithAvatar(record, Array.from(bytes)));
}

export async function updateCharacterWithAvatar(
    record: Parameters<typeof commands.updateCharacterWithAvatar>[0],
    bytes: Uint8Array,
): Promise<void> {
    await withKokoroError(commands.updateCharacterWithAvatar(record, Array.from(bytes)));
}

export async function updateCharacter(record: Parameters<typeof commands.updateCharacter>[0]): Promise<void> {
    await withKokoroError(commands.updateCharacter(record));
}

export async function deleteCharacter(id: string): Promise<void> {
    await withKokoroError(commands.deleteCharacter(id));
}

export async function duplicateCharacter(request: Readonly<DuplicateCharacterRequest>): Promise<void> {
    await withKokoroError(commands.duplicateCharacter(request));
}

export async function restoreCharacterDefaults(
    request: Readonly<RestoreCharacterDefaultsRequest>,
): Promise<void> {
    await withKokoroError(commands.restoreCharacterDefaults(request));
}

export async function listCharacterTemplates(): Promise<Array<CharacterTemplateManifest>> {
    return withKokoroError(commands.listCharacterTemplates());
}

export type InstalledContentPackage = {
    readonly content_type: "character" | "mod";
    readonly id: string;
    readonly version: string;
};

export async function listInstalledContent(): Promise<Array<InstalledContentPackage>> {
    const [characters, mods] = await Promise.all([listCharacterTemplates(), listMods()]);
    return [
        ...characters.map((character) => ({
            content_type: "character" as const,
            id: character.id,
            version: character.version,
        })),
        ...mods.map((mod) => ({
            content_type: "mod" as const,
            id: mod.id,
            version: mod.version,
        })),
    ];
}

export async function instantiateCharacterTemplate(
    request: Readonly<InstantiateCharacterTemplateRequest>,
): Promise<void> {
    await withKokoroError(commands.instantiateCharacterTemplate(request));
}

export async function reconcileCharacterTemplate(
    request: Readonly<ReconcileCharacterTemplateRequest>,
): Promise<ReconcileCharacterTemplatePreview> {
    return withKokoroError(commands.reconcileCharacterTemplate(request));
}

export async function applyCharacterTemplateReconciliation(
    request: Readonly<ApplyCharacterTemplateReconciliationRequest>,
): Promise<void> {
    await withKokoroError(commands.applyCharacterTemplateReconciliation(request));
}

// ── Content Registry ───────────────────────────────────────────────────────

export type RegistryRecommendations = NonNullable<RegistryEntry["recommendations"]>;

export type RegistryEntry = RegistryIndex["entries"][number];

export type RegistryIndex = IpcResponse<"listRegistryEntries">;

export type InstalledCharacterPackage = IpcResponse<"installCharacterFromRegistry">;

export type CharacterPackageRemoval = IpcResponse<"removeCharacterPackage">;

export async function listRegistryEntries(registryUrl?: string): Promise<RegistryIndex> {
    return withKokoroError(commands.listRegistryEntries(registryUrl ?? null));
}

export async function installCharacterFromRegistry(
    characterId: string,
    version: string,
    registryUrl?: string,
): Promise<InstalledCharacterPackage> {
    return withKokoroError(commands.installCharacterFromRegistry(characterId, version, registryUrl ?? null));
}

export async function installCharacterFromUrl(url: string): Promise<InstalledCharacterPackage> {
    return withKokoroError(commands.installCharacterFromUrl(url));
}

export async function removeCharacterPackage(
    characterId: string,
    version: string,
): Promise<CharacterPackageRemoval> {
    return withKokoroError(commands.removeCharacterPackage(characterId, version));
}

// ── Auto Backup ────────────────────────────────────

export type AutoBackupConfig = IpcResponse<"getAutoBackupConfig">;

export async function getAutoBackupConfig(): Promise<AutoBackupConfig> {
    return withKokoroError(commands.getAutoBackupConfig());
}

export async function saveAutoBackupConfig(config: AutoBackupConfig): Promise<void> {
    await withKokoroError(commands.saveAutoBackupConfig(config));
}

export async function runAutoBackupNow(): Promise<string> {
    return withKokoroError(commands.runAutoBackupNow());
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

function stringifyErrorObject(error: object): string {
    try {
        return JSON.stringify(error);
    } catch {
        return Object.prototype.toString.call(error);
    }
}

function objectToKokoroError(error: Record<string, unknown>): KokoroErrorObject | null {
    const code = getStringField(error, "code");
    const message = getStringField(error, "message");
    if (!code || !message) {
        return null;
    }

    return {
        code,
        message,
        ...(typeof error.stage === "string" ? { stage: error.stage } : {}),
        ...(typeof error.retryable === "boolean" ? { retryable: error.retryable } : {}),
        ...(typeof error.trace_id === "string" ? { trace_id: error.trace_id } : {}),
    };
}

/**
 * 将 Tauri / Rust / JS 各种错误载荷转成人能读的文本。
 */
export function getKokoroErrorMessage(error: unknown): string {
    if (error instanceof Error) {
        return error.message;
    }

    if (typeof error === "string") {
        const parsed = parseJsonPayload(error);
        return parsed === null ? error : getKokoroErrorMessage(parsed);
    }

    if (isRecord(error)) {
        const structured = objectToKokoroError(error);
        if (structured) {
            return structured.message;
        }

        const message = error.message ?? error.error;
        if (typeof message === "string") {
            return message;
        }
        if (isRecord(message)) {
            return getKokoroErrorMessage(message);
        }

        const stringValue = Object.values(error).find((value): value is string => typeof value === "string");
        if (stringValue) {
            return stringValue;
        }

        const nestedObject = Object.values(error).find(isRecord);
        if (nestedObject) {
            return getKokoroErrorMessage(nestedObject);
        }

        return stringifyErrorObject(error);
    }

    return String(error);
}

/**
 * 解析 Kokoro 错误，支持结构化 JSON 和裸字符串两种格式
 *
 * @param error - 来自后端的错误（可能是 JSON 字符串或裸字符串）
 * @returns 结构化错误对象或原始错误字符串
 */
export function parseKokoroError(error: unknown): KokoroErrorObject | string {
    if (typeof error === "string") {
        const parsed = parseJsonPayload(error);
        if (isRecord(parsed)) {
            return objectToKokoroError(parsed) ?? getKokoroErrorMessage(parsed);
        }
        return error;
    }

    if (isRecord(error)) {
        return objectToKokoroError(error) ?? getKokoroErrorMessage(error);
    }

    return getKokoroErrorMessage(error);
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
    return isRecord(error) && error.code === code && typeof error.message === "string";
}
