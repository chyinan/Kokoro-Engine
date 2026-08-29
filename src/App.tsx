// pattern: Imperative Shell

import { useState, useEffect, useMemo, useSyncExternalStore, useCallback, useRef } from "react";
import { motion } from "framer-motion";
import { Settings } from "lucide-react";
import { emit, listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { LayoutRenderer } from "./ui/layout/LayoutRenderer";
import { LayoutConfig } from "./ui/layout/types";
import { ThemeProvider } from "./ui/theme/ThemeContext";
import { defaultTheme } from "./ui/theme/default";
import { registry } from "./ui/registry/ComponentRegistry";
import { registerCoreComponents } from "./core/init";
import { ttsService } from "./core/services";
import SettingsPanel, { normalizeSettingsTabId, type SettingsTabId } from "./ui/widgets/SettingsPanel";
import BackgroundLayer from "./ui/widgets/BackgroundLayer";
import { mapBackgroundAssetUrl } from "./ui/widgets/background-asset-url";
import WindowTitleBar from "./ui/widgets/WindowTitleBar";
import OnboardingOverlay, {
  type OnboardingCharacter,
  type OnboardingLanguageCode,
} from "./ui/widgets/OnboardingOverlay";
import MemoryModelDownloadDialog from "./ui/widgets/MemoryModelDownloadDialog";
import { QQAuthorizationDialog } from "./ui/widgets/QQAuthorizationDialog";
import { CharacterCatalog, type CharacterCatalogActionDependencies } from "./ui/widgets/CharacterCatalog";
import {
  CharacterRecommendationDialog,
  type CharacterCapabilityRecommendations,
} from "./ui/widgets/CharacterRecommendationDialog";
import { useBackgroundSlideshow } from "./ui/hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "./features/live2d/Live2DViewer";
import { mapLive2dModelUrl, type Live2dModelSource } from "./features/live2d/live2d-model-url";
import { live2dUrl } from "./lib/utils";
import { MEMORY_MODEL_DIALOG_EVENT } from "./lib/memory-model-gate";
import { characterDb } from "./lib/db";
import {
  APP_SETTING_KEYS,
  type AppSettingKey,
  dispatchRuntimeSettingsChanged,
  readBooleanSetting,
  readNumberSetting,
  readStringSetting,
  removeSetting,
  writeBooleanSetting,
  writeJsonSetting,
  writeStringSetting,
} from "./lib/app-settings";
import {
  createCharacterActivationService,
  type CharacterActivationService,
} from "./features/characters/character-activation";
import type {
  FrontendAssetSource,
  FrontendRuntimeState,
} from "./features/characters/character-runtime-profile";
import { resolveFrontendAssetSource } from "./features/characters/character-runtime-profile";
import { parseCharacterCueProfile } from "./features/characters/character-cue-profile";
import { applyCharacterCapabilityRecommendations } from "./features/characters/character-capability-recommendations";
import { createCharacterRuntimeOverrideService } from "./features/characters/character-runtime-override-service";
import {
  selectCharacterForEditing,
  type CharacterRuntimeOverrides,
} from "./features/characters/character-runtime-overrides";
import {
  createOnboardingDraft,
  deserializeOnboardingDraft,
  onboardingFlowReducer,
  serializeOnboardingDraft,
  type OnboardingDraft,
  type OnboardingFlowEvent,
} from "./features/onboarding/onboarding-flow";
import {
  providerToSetup,
  saveProviderSetup,
  testProviderSetup,
  type ProviderSetup,
} from "./features/onboarding/provider-setup";
import {
  createModActionDispatcher,
  getModActionFromEvent,
  type ModActionEnvelope,
} from "./core/mod-actions/dispatcher";

// Register components synchronously before first render
registerCoreComponents();

const CHAT_PANEL_MIN_WIDTH = 350;
const CHAT_PANEL_RESIZE_GUTTER = 160;
const CHAT_PANEL_WIDTH_CSS_VAR = "--kokoro-chat-panel-width";
const HOME_LIVE2D_HORIZONTAL_OFFSET_STORAGE_KEY = "kokoro_live2d_home_horizontal_offset";

function clampChatPanelWidth(width: number): number {
  const roundedWidth = Math.round(width);
  if (typeof window === "undefined") {
    return Math.max(CHAT_PANEL_MIN_WIDTH, roundedWidth);
  }
  const viewportMax = Math.max(CHAT_PANEL_MIN_WIDTH, window.innerWidth - CHAT_PANEL_RESIZE_GUTTER);
  return Math.min(Math.max(CHAT_PANEL_MIN_WIDTH, roundedWidth), viewportMax);
}

function applyChatPanelWidth(width: number): number {
  const nextWidth = clampChatPanelWidth(width);
  if (typeof document !== "undefined") {
    document.documentElement.style.setProperty(CHAT_PANEL_WIDTH_CSS_VAR, `${nextWidth}px`);
  }
  return nextWidth;
}

// Build layout config as a function of displayMode
function createLayout(options: {
  mode: Live2DDisplayMode;
  modelUrl: string;
  modelPath: string | null;
  modelSource: Live2dModelSource;
  gazeTracking: boolean;
  renderFps: number;
  chatPanelWidth: number;
  chatPanelInteractive: boolean;
  onChatPanelWidthPreview: (width: number) => number;
  onChatPanelWidthChange: (width: number) => void;
}): LayoutConfig {
  return {
    root: {
      id: "root-layer",
      type: "layer",
      children: [
        {
          id: "stage",
          type: "component",
          component: "Live2DStage",
          zIndex: 0,
          props: {
            modelUrl: options.modelUrl,
            modelPath: options.modelPath,
            modelSource: options.modelSource,
            displayMode: options.mode,
            gazeTracking: options.gazeTracking,
            maxFps: options.renderFps,
            enableHorizontalDrag: true,
            horizontalOffsetStorageKey: HOME_LIVE2D_HORIZONTAL_OFFSET_STORAGE_KEY,
          }
        },
        {
          id: "ui-grid",
          type: "grid",
          zIndex: 10,
          style: {
            gridTemplateColumns: `var(${CHAT_PANEL_WIDTH_CSS_VAR}, ${CHAT_PANEL_MIN_WIDTH}px) minmax(0, 1fr)`,
            gridTemplateRows: "1fr",
            gridTemplateAreas: `
                        "highlight main"
                    `,
            pointerEvents: "none",
            position: "absolute",
            inset: "0"
          },
          children: [
            {
              id: "chat-panel",
              type: "component",
              component: "ChatPanel",
              area: "highlight",
              props: {
                minWidth: CHAT_PANEL_MIN_WIDTH,
                width: options.chatPanelWidth,
                onWidthPreview: options.onChatPanelWidthPreview,
                onWidthChange: options.onChatPanelWidthChange,
                interactionDisabled: !options.chatPanelInteractive,
              },
              style: { pointerEvents: options.chatPanelInteractive ? "auto" : "none", margin: "48px 0 20px 20px", padding: "0" },
              motion: "panelEntry"
            }
          ]
        }
      ]
    }
  };
}

function parseMcpJson(raw: string): McpServerConfig[] {
  let trimmed = raw.trim().replace(/,\s*$/, "");
  if (trimmed.startsWith('"') && !trimmed.startsWith("{")) {
    trimmed = `{${trimmed}}`;
  }

  const parsed = JSON.parse(trimmed);
  const servers = parsed.mcpServers || parsed;
  const configs: McpServerConfig[] = [];

  for (const [name, entry] of Object.entries(servers)) {
    const server = entry as {
      type?: string;
      transportType?: string;
      command?: string;
      args?: string[];
      env?: Record<string, string>;
      url?: string;
      disabled?: boolean;
    };
    let type = server.type || server.transportType || "stdio";
    if (type === "stdio" && !server.command && server.url) {
      type = server.url.replace(/\/+$/, "").endsWith("/sse") ? "sse" : "streamable_http";
    }
    configs.push({
      name,
      type,
      command: server.command || "",
      args: server.args || [],
      env: server.env || {},
      url: server.url,
      enabled: server.disabled === true ? false : true,
    });
  }

  return configs;
}

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import {
  onImageGenDone,
  onModThemeOverride,
  onModComponentsRegister,
  onModUiMessage,
  onModScriptEvent,
  onModUnload,
  onChatTurnStart,
  onChatTurnDelta,
  onChatTurnFinish,
  onChatCue,
  streamChat,
  dispatchModEvent,
  unloadMod,
  loadMod,
  installMod,
  listLive2dModels,
  getTtsConfig,
  prepareCharacterActivation,
  commitCharacterActivation,
  getCommittedCharacterRuntime,
  listCharacters,
  listCharacterTemplates,
  instantiateCharacterTemplate,
  duplicateCharacter,
  restoreCharacterDefaults,
  reconcileCharacterTemplate,
  applyCharacterTemplateReconciliation,
  createCharacter,
  createCharacterWithAvatar,
  updateCharacter,
  setUserName,
  setUserPersona,
  getJailbreakPrompt,
  setJailbreakPrompt,
  getProactiveEnabled,
  getUserProfileSettings,
  getMemoryEmbeddingModelStatus,
  setMemoryEnabled,
  // Config Getters
  getLlmConfig,
  getImageGenConfig,
  getTelegramConfig,
  getTelegramStatus,
  getBotConfig,
  getBotStatus,
  getAutoBackupConfig,
  getVisionConfig,
  getSttConfig,
  listMcpServers,
  listMods,
  listTtsProviders,
  listTtsVoices,
  // Actions
  fetchModels,
  listAnthropicModels,
  listOllamaModels,
  getLlamaCppStatus,
  listGptSovitsModels,
  // Config Setters
  saveLlmConfig,
  saveTtsConfig,
  saveImageGenConfig,
  saveVisionConfig,
  saveSttConfig,
  saveTelegramConfig,
  saveBotConfig,
  respondQQAuthorization,
  saveAutoBackupConfig,
  runAutoBackupNow,
  exportData,
  previewImport,
  importData,
  startTelegramBot,
  stopTelegramBot,
  startBotPlatform,
  stopBotPlatform,
  // New: MCP Management
  addMcpServer,
  removeMcpServer,
  reconnectMcpServer,
  refreshMcpTools,
  toggleMcpServer,
  // New: Memory
  listMemories,
  updateMemory,
  updateMemoryTier,
  deleteMemory,
  downloadMemoryEmbeddingModel,
  // New: ImageGen
  testSdConnection,
  setWindowSize,
  onChatImageGen,
  generateImage,
  synthesize,
  // New: Vision
  captureScreenNow,
  // New: Live2D
  deleteLive2dModel,
  importLive2dZip,
  importLive2dFolder,
  exportLive2dModel,
  renameLive2dModel,
  setActiveLive2dModel,
  BUILTIN_LIVE2D_MODEL_PATH,
  // New: Context
  setUserLanguage,
  // Types
  type ImageGenResult,
  type ModThemeJson,
  type Live2dModelInfo,
  type TtsSystemConfig,
  type LlmConfig,
  type LlmConnectionTestResult,
  type SttConfig,
  type VisionConfig,
  type ImageGenSystemConfig,
  type ModManifest,
  type McpServerConfig,
  type McpServerStatus,
  type ProviderStatus,
  type VoiceProfile,
  type GptSovitsModels,
  type MemoryRecord,
  type TelegramConfig,
  type TelegramStatus,
  type BotConfig,
  type BotPlatformId,
  type BotStatus,
  type AutoBackupConfig,
  type ImportPreview,
  type CharacterRecord,
  type CharacterTemplateManifest,
  type MemoryEmbeddingModelStatus,
  type MemoryEmbeddingModelDownloadProgress,
  type QQAuthorizationRequest,
  type CharacterActivationToken,
  getKokoroErrorMessage,
  onMemoryEmbeddingModelProgress,
} from "./lib/kokoro-bridge";
import type { ThemeConfig } from "./ui/layout/types";
import { modMessageBus } from "./ui/mods/ModMessageBus";
import { CameraWatcher } from "./features/camera/CameraWatcher";
import { mapCharacterAvatarUrl } from "./ui/widgets/character-avatar-url";
import { shouldEnableChatPanel } from "./ui/layout/layout-interaction";
import { isOnboardingTurnEvent } from "./features/onboarding/onboarding-turn-correlation";

let _regSnap = 0;
const _subscribeFn = (cb: () => void) => {
  return registry.subscribe(() => { _regSnap++; cb(); });
};
const _getSnap = () => _regSnap;

interface PetConfig {
  render_fps?: number;
}

type ModActionData = Record<string, any>;
type LegacyModActionEnvelope = ModActionEnvelope & {
  data?: ModActionData;
};

const ONBOARDING_STATUS_KEY = "kokoro_onboarding_status";
const ONBOARDING_DRAFT_KEY = "kokoro_onboarding_draft";

const ONBOARDING_LANGUAGE_NAMES: Record<OnboardingLanguageCode, string> = {
  en: "English",
  zh: "中文",
  "zh-TW": "繁體中文",
  ja: "日本語",
  ko: "한국어",
  ru: "Русский",
};

function App() {
  const { i18n, t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [activeSettingsTab, setActiveSettingsTab] = useState<SettingsTabId>(() => {
    const saved = readStringSetting(APP_SETTING_KEYS.settingsActiveTab, "");
    return normalizeSettingsTabId(saved);
  });
  const [onboardingDraft, setOnboardingDraft] = useState<OnboardingDraft>(() =>
    deserializeOnboardingDraft(localStorage.getItem(ONBOARDING_DRAFT_KEY)) ?? createOnboardingDraft()
  );
  const [onboardingOpen, setOnboardingOpen] = useState<boolean>(() => {
    const status = localStorage.getItem(ONBOARDING_STATUS_KEY);
    const draft = deserializeOnboardingDraft(localStorage.getItem(ONBOARDING_DRAFT_KEY));
    return status !== "completed" && draft?.completed !== true;
  });
  const [displayMode, setDisplayMode] = useState<Live2DDisplayMode>(
    () => readStringSetting(APP_SETTING_KEYS.displayMode, "full") as Live2DDisplayMode
  );
  const bgSlideshow = useBackgroundSlideshow();
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);

  // Subscribe to registry changes so SettingsPanel slot picks up mod overrides.
  useSyncExternalStore(_subscribeFn, _getSnap);

  const [customModelPath, setCustomModelPath] = useState<string | null>(
    () => readStringSetting(APP_SETTING_KEYS.customModelPath, "") || null
  );
  const [characterBackground, setCharacterBackground] = useState<string | null>(
    () => readStringSetting(APP_SETTING_KEYS.characterBackground, "") || null
  );
  const [activeCharacterId, setActiveCharacterIdState] = useState(
    () => readStringSetting(APP_SETTING_KEYS.activeCharacterId, ""),
  );
  const [runtimeReady, setRuntimeReady] = useState(false);
  const [onboardingProviderSetup, setOnboardingProviderSetup] = useState<ProviderSetup>({
    providerType: "ollama",
    presetId: null,
    endpoint: "http://localhost:11434",
    apiKey: null,
    model: "llama3",
  });
  const [onboardingConnectionResult, setOnboardingConnectionResult] = useState<LlmConnectionTestResult | null>(null);
  const [onboardingTestingConnection, setOnboardingTestingConnection] = useState(false);
  const [onboardingSavingProvider, setOnboardingSavingProvider] = useState(false);
  const [onboardingProviderError, setOnboardingProviderError] = useState<string | null>(null);
  const [onboardingCharacterError, setOnboardingCharacterError] = useState<string | null>(null);
  const [onboardingSubmittingChat, setOnboardingSubmittingChat] = useState(false);
  const onboardingChatPendingRef = useRef<{
    clientRequestId: string;
    turnId: string | null;
    reply: string;
    resolve: (reply: string) => void;
    reject: (error: Error) => void;
  } | null>(null);
  const pendingOnboardingResponseLanguageRef = useRef<string | null>(null);

  const [gazeTracking, setGazeTracking] = useState<boolean>(
    () => readBooleanSetting(APP_SETTING_KEYS.gazeTracking, true)
  );
  const [renderFps, setRenderFps] = useState<number>(60);
  const [chatPanelWidth, setChatPanelWidth] = useState(CHAT_PANEL_MIN_WIDTH);
  const activeLive2dModelPath = customModelPath ?? BUILTIN_LIVE2D_MODEL_PATH;

  const handleChatPanelWidthPreview = useCallback((width: number) => {
    return applyChatPanelWidth(width);
  }, []);

  const handleChatPanelWidthChange = useCallback((width: number) => {
    setChatPanelWidth(applyChatPanelWidth(width));
  }, []);

  const handleGazeTrackingChange = (enabled: boolean) => {
    setGazeTracking(enabled);
    writeBooleanSetting(APP_SETTING_KEYS.gazeTracking, enabled);
  };

  useEffect(() => {
    applyChatPanelWidth(chatPanelWidth);
    const handleResize = () => {
      setChatPanelWidth(prev => applyChatPanelWidth(prev));
    };
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [chatPanelWidth]);

  // ── Global Settings State ──
  const [availableModels, setAvailableModels] = useState<Live2dModelInfo[]>([]);
  const [persona, setPersonaState] = useState(() => readStringSetting(APP_SETTING_KEYS.persona, ""));
  const [responseLanguage, setResponseLanguageState] = useState(() =>
    readStringSetting(APP_SETTING_KEYS.responseLanguage, "zh")
  );

  // Full Config State
  const [ttsConfig, setTtsConfig] = useState<TtsSystemConfig | undefined>(undefined);
  const [llmConfig, setLlmConfig] = useState<LlmConfig | undefined>(undefined);
  const [sttConfig, setSttConfig] = useState<SttConfig | undefined>(undefined);
  const [visionConfig, setVisionConfig] = useState<VisionConfig | undefined>(undefined);
  const [imageGenConfig, setImageGenConfig] = useState<ImageGenSystemConfig | undefined>(undefined);
  const [telegramConfig, setTelegramConfig] = useState<TelegramConfig | undefined>(undefined);
  const [telegramStatus, setTelegramStatus] = useState<TelegramStatus | undefined>(undefined);
  const [botConfig, setBotConfig] = useState<BotConfig | undefined>(undefined);
  const [botStatus, setBotStatus] = useState<BotStatus | undefined>(undefined);
  const [autoBackupConfig, setAutoBackupConfig] = useState<AutoBackupConfig | undefined>(undefined);
  const [backupStatus, setBackupStatus] = useState<{
    phase: "idle" | "exporting" | "exported" | "preview" | "importing" | "imported" | "auto-running" | "auto-ran" | "error";
    message?: string;
    error?: string;
    preview?: ImportPreview;
    importFilePath?: string;
  }>({ phase: "idle" });

  // Lists
  const [mcpServers, setMcpServers] = useState<McpServerStatus[]>([]);
  const [modList, setModList] = useState<ModManifest[]>([]);
  const [ttsProviders, setTtsProviders] = useState<ProviderStatus[]>([]);
  const [ttsVoices, setTtsVoices] = useState<VoiceProfile[]>([]);

  // Character list for mod settings
  const [characters, setCharacters] = useState<CharacterRecord[]>([]);
  const [characterTemplates, setCharacterTemplates] = useState<Array<CharacterTemplateManifest>>([]);
  const [recommendedCapabilities, setRecommendedCapabilities] = useState<{
    readonly characterName: string;
    readonly recommendations: CharacterCapabilityRecommendations;
  } | null>(null);
  const [characterToEditId, setCharacterToEditId] = useState<string | null>(null);

  // Mod-specific state exposed via props
  const [voiceInterrupt, setVoiceInterrupt] = useState(false);
  const [fetchedLlmModels, setFetchedLlmModels] = useState<string[]>([]);
  const [scannedTtsModels, setScannedTtsModels] = useState<Record<string, GptSovitsModels>>({});
  // New: Memory, MCP, Vision, ImageGen dynamic state for mods
  const [memoryList, setMemoryList] = useState<MemoryRecord[]>([]);
  const [memoryTotal, setMemoryTotal] = useState(0);
  const [sdModels, setSdModels] = useState<string[]>([]);
  const [capturedScreenUrl, setCapturedScreenUrl] = useState<string | null>(null);
  const [userLanguage, setUserLanguageState] = useState(() =>
    readStringSetting(APP_SETTING_KEYS.userLanguage, "zh")
  );
  const [proactiveEnabled, setProactiveEnabledState] = useState(() => {
    return readBooleanSetting(APP_SETTING_KEYS.proactiveEnabled, true);
  });
  const [memoryModelStatus, setMemoryModelStatus] = useState<MemoryEmbeddingModelStatus | null>(null);
  const [memoryModelProgress, setMemoryModelProgress] = useState<MemoryEmbeddingModelDownloadProgress | null>(null);
  const [memoryModelDialogOpen, setMemoryModelDialogOpen] = useState(false);
  const [memoryModelDownloading, setMemoryModelDownloading] = useState(false);
  const [memoryModelError, setMemoryModelError] = useState<string | null>(null);
  const memoryModelDownloadInFlightRef = useRef(false);
  const [qqAuthorizationQueue, setQQAuthorizationQueue] = useState<Array<QQAuthorizationRequest>>([]);
  const [qqAuthorizationBusy, setQQAuthorizationBusy] = useState(false);
  const [qqAuthorizationError, setQQAuthorizationError] = useState<string | null>(null);
  const currentQQAuthorization = qqAuthorizationQueue[0] ?? null;

  function readCharacterFrontendRuntime(): FrontendRuntimeState {
    const providerId = readStringSetting(APP_SETTING_KEYS.ttsProvider, "") || null;
    const isTtsEnabled = readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false);
    const readAssetSource = (key: AppSettingKey, value: string | null): FrontendAssetSource =>
      resolveFrontendAssetSource(readStringSetting(key, "") || undefined, value);
    const live2dModel = readStringSetting(APP_SETTING_KEYS.customModelPath, "") || null;
    const background = readStringSetting(APP_SETTING_KEYS.characterBackground, "") || null;
    const cueProfile = readStringSetting(APP_SETTING_KEYS.characterCueProfile, "") || null;
    return {
      activeCharacterId: readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || null,
      live2dModel,
      live2dModelSource: readAssetSource(APP_SETTING_KEYS.characterLive2dSource, live2dModel),
      background,
      backgroundSource: readAssetSource(APP_SETTING_KEYS.characterBackgroundSource, background),
      tts: {
        enabled: isTtsEnabled,
        mode: !isTtsEnabled
          ? "text_only"
          : providerId === "browser"
            ? "browser"
            : "configured_provider",
        providerId,
        voice: readStringSetting(APP_SETTING_KEYS.ttsVoice, "") || null,
        speed: readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1),
        pitch: readNumberSetting(APP_SETTING_KEYS.ttsPitch, 1),
      },
      cueProfile,
      cueProfileSource: readAssetSource(APP_SETTING_KEYS.characterCueProfileSource, cueProfile),
    };
  }

  async function applyCharacterFrontendRuntime(
    runtime: Readonly<FrontendRuntimeState>,
  ): Promise<void> {
    if (runtime.activeCharacterId !== null) {
      writeStringSetting(APP_SETTING_KEYS.activeCharacterId, runtime.activeCharacterId);
      setActiveCharacterIdState(runtime.activeCharacterId);
    }
    if (runtime.live2dModel === null) {
      removeSetting(APP_SETTING_KEYS.customModelPath);
    } else {
      writeStringSetting(APP_SETTING_KEYS.customModelPath, runtime.live2dModel);
    }
    writeStringSetting(
      APP_SETTING_KEYS.characterLive2dSource,
      runtime.live2dModelSource ?? (runtime.live2dModel === null ? "none" : "user"),
    );
    if (runtime.background === null) {
      removeSetting(APP_SETTING_KEYS.characterBackground);
    } else {
      writeStringSetting(APP_SETTING_KEYS.characterBackground, runtime.background);
    }
    writeStringSetting(
      APP_SETTING_KEYS.characterBackgroundSource,
      runtime.backgroundSource ?? (runtime.background === null ? "none" : "user"),
    );
    if (runtime.cueProfile === null) {
      removeSetting(APP_SETTING_KEYS.characterCueProfile);
      removeSetting(APP_SETTING_KEYS.characterCueProfileCache);
      window.dispatchEvent(new CustomEvent("kokoro-character-cue-profile-changed", {
        detail: { cueMap: {}, semanticCueMap: {} },
      }));
    } else {
      writeStringSetting(APP_SETTING_KEYS.characterCueProfile, runtime.cueProfile);
      try {
        const response = await fetch(convertFileSrc(runtime.cueProfile));
        if (!response.ok) throw new Error(`cue profile returned ${response.status}`);
        const applied = parseCharacterCueProfile(await response.json());
        writeJsonSetting(APP_SETTING_KEYS.characterCueProfileCache, applied);
        window.dispatchEvent(new CustomEvent("kokoro-character-cue-profile-changed", {
          detail: applied,
        }));
      } catch (error) {
        console.warn("[CharacterActivation] failed to apply optional cue profile", error);
        removeSetting(APP_SETTING_KEYS.characterCueProfileCache);
      }
    }
    writeStringSetting(
      APP_SETTING_KEYS.characterCueProfileSource,
      runtime.cueProfileSource ?? (runtime.cueProfile === null ? "none" : "user"),
    );
    writeBooleanSetting(APP_SETTING_KEYS.ttsEnabled, runtime.tts.enabled);
    writeStringSetting(APP_SETTING_KEYS.ttsProvider, runtime.tts.providerId ?? "");
    writeStringSetting(APP_SETTING_KEYS.ttsVoice, runtime.tts.voice ?? "");
    writeStringSetting(APP_SETTING_KEYS.ttsSpeed, String(runtime.tts.speed));
    writeStringSetting(APP_SETTING_KEYS.ttsPitch, String(runtime.tts.pitch));
    setCustomModelPath(runtime.live2dModel);
    setCharacterBackground(runtime.background);
  }

  const activationServiceRef = useRef<CharacterActivationService | null>(null);
  if (activationServiceRef.current === null) {
    activationServiceRef.current = createCharacterActivationService({
      prepareCharacterActivation,
      commitCharacterActivation,
      getCommittedCharacterRuntime,
      readFrontendRuntime: readCharacterFrontendRuntime,
      applyFrontendRuntime: applyCharacterFrontendRuntime,
      restoreFrontendRuntime: applyCharacterFrontendRuntime,
      writeRuntimeCache: (runtime) => {
        writeJsonSetting(APP_SETTING_KEYS.characterRuntimeCache, runtime);
      },
      dispatchRuntimeChanged: (runtime) => {
        writeStringSetting(APP_SETTING_KEYS.persona, runtime.runtime.system_prompt);
        writeStringSetting(APP_SETTING_KEYS.responseLanguage, runtime.runtime.response_language);
        writeBooleanSetting(APP_SETTING_KEYS.proactiveEnabled, runtime.runtime.proactive_enabled);
        setPersonaState(runtime.runtime.system_prompt);
        setResponseLanguageState(runtime.runtime.response_language);
        setProactiveEnabledState(runtime.runtime.proactive_enabled);
        window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
          detail: runtime,
        }));
        void emit("character-runtime-committed", runtime).catch((error) => {
          console.warn("[CharacterActivation] failed to notify auxiliary windows", error);
        });
      },
      probeLocalTtsPreset: async (endpoint) => {
        try {
          const response = await fetch(endpoint, { method: "GET" });
          return response.ok;
        } catch {
          return false;
        }
      },
      presentLocalTtsPresetProbe: ({ endpoint, available }) => {
        window.alert(t(
          available
            ? "characterCatalog.ttsProbe.available"
            : "characterCatalog.ttsProbe.unavailable",
          { endpoint },
        ));
      },
      confirmLocalTtsPresetSave: async ({ endpoint, available }) => {
        if (!available) return false;
        return window.confirm(t("characterCatalog.ttsProbe.confirm", { endpoint }));
      },
      saveConfirmedLocalTtsPreset: async (token: Readonly<CharacterActivationToken>) => {
        const resolved = token.resolved_runtime.tts;
        const id = resolved.local_preset;
        const providerType = resolved.provider_type;
        const endpoint = resolved.endpoint;
        if (id === null || providerType === null || endpoint === null) return;
        const config = await getTtsConfig();
        const existing = config.providers.find((provider) => provider.id === id);
        const nextConfig: TtsSystemConfig = {
          ...config,
          default_provider: id,
          providers: existing
            ? config.providers.map((provider) => provider.id === id
              ? {
                ...provider,
                provider_type: providerType,
                enabled: true,
                // The allowlisted preset owns its conventional loopback
                // endpoint; never retain a stale custom/remote endpoint.
                endpoint,
                base_url: endpoint,
                default_voice: provider.default_voice ?? resolved.voice,
              }
              : provider)
            : [
              ...config.providers,
              {
                id,
                provider_type: providerType,
                enabled: true,
                endpoint,
                base_url: endpoint,
                default_voice: resolved.voice,
                extra: {},
              },
            ],
        };
        await saveTtsConfig(nextConfig);
        setTtsConfig(nextConfig);
      },
    });
  }
  const characterActivation = activationServiceRef.current;
  const characterRuntimeOverrides = createCharacterRuntimeOverrideService({
    getCharacter: async (characterId) => {
      const all = await listCharacters();
      return selectCharacterForEditing(all, characterId);
    },
    updateCharacter: async (character) => {
      const { created_at: _createdAt, ...request } = character;
      await updateCharacter(request);
    },
    activateCharacter: (characterId) => characterActivation.activateCharacter(characterId),
    now: Date.now,
  });

  async function updateActiveCharacterRuntime(
    overrides: Readonly<CharacterRuntimeOverrides>,
  ): Promise<void> {
    const characterId = readStringSetting(APP_SETTING_KEYS.activeCharacterId, "");
    if (!characterId) throw new Error("active character is unavailable");
    await characterRuntimeOverrides.update(characterId, overrides);
    setCharacters(await listCharacters());
  }

  function readActiveCharacterTtsRuntime(): NonNullable<CharacterRuntimeOverrides["tts"]> {
    const providerId = readStringSetting(APP_SETTING_KEYS.ttsProvider, "") || null;
    const provider = ttsConfig?.providers.find((candidate) => candidate.id === providerId);
    return {
      enabled: readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false),
      providerId,
      providerType: provider?.provider_type ?? null,
      voice: readStringSetting(APP_SETTING_KEYS.ttsVoice, "") || null,
      speed: readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1),
      pitch: readNumberSetting(APP_SETTING_KEYS.ttsPitch, 1),
    };
  }

  const activeModelSource: Live2dModelSource = (() => {
    const rawSource = readStringSetting(APP_SETTING_KEYS.characterLive2dSource, "builtin");
    return rawSource === "package" || rawSource === "user" ? rawSource : "builtin";
  })();
  const modelUrl = useMemo(() => {
    const path = customModelPath ?? BUILTIN_LIVE2D_MODEL_PATH;
    return mapLive2dModelUrl(path, activeModelSource, convertFileSrc, live2dUrl);
  }, [customModelPath, activeModelSource]);

  useEffect(() => {
    const isAbsolutePath = /^[A-Za-z]:[\\/]/.test(activeLive2dModelPath)
      || activeLive2dModelPath.startsWith("\\\\")
      || activeLive2dModelPath.startsWith("/");
    const backendModelPath = isAbsolutePath ? BUILTIN_LIVE2D_MODEL_PATH : activeLive2dModelPath;
    setActiveLive2dModel(backendModelPath).catch((err) => {
      console.error("[App] Failed to sync active Live2D model:", err);
    });
    emit("live2d-model-selection-updated", {
      modelPath: activeLive2dModelPath,
      customModelPath,
      modelUrl,
      modelSource: activeModelSource,
    }).catch((err) => {
      console.error("[App] Failed to broadcast Live2D model selection:", err);
    });
  }, [activeLive2dModelPath, customModelPath, modelUrl, activeModelSource]);

  const layout = useMemo(
    () => createLayout({
      mode: displayMode,
      modelUrl,
      modelPath: activeLive2dModelPath,
      modelSource: activeModelSource,
      gazeTracking,
      renderFps,
      chatPanelWidth,
      chatPanelInteractive: shouldEnableChatPanel(onboardingOpen),
      onChatPanelWidthPreview: handleChatPanelWidthPreview,
      onChatPanelWidthChange: handleChatPanelWidthChange,
    }),
    [displayMode, modelUrl, activeLive2dModelPath, activeModelSource, gazeTracking, renderFps, chatPanelWidth, onboardingOpen, handleChatPanelWidthPreview, handleChatPanelWidthChange]
  );

  const handleDisplayModeChange = (mode: Live2DDisplayMode) => {
    setDisplayMode(mode);
    writeStringSetting(APP_SETTING_KEYS.displayMode, mode);
  };

  const handleCustomModelChange = (path: string | null) => {
    void updateActiveCharacterRuntime({ live2dModel: path }).catch((error) => {
      console.error("[App] Failed to update active character model:", error);
    });
  };

  const handleRenderFpsChange = async (fps: number) => {
    setRenderFps(fps);

    try {
      const cfg = await invoke<PetConfig>("get_pet_config");
      const nextConfig = { ...cfg, render_fps: fps };
      await invoke("save_pet_config", { config: nextConfig });
      await emit("pet-config-updated", nextConfig);
    } catch (e) {
      console.error("[App] Failed to persist render FPS:", e);
    }
  };

  const applyOnboardingLanguage = (language: OnboardingLanguageCode) => {
    const label = ONBOARDING_LANGUAGE_NAMES[language];
    i18n.changeLanguage(language);
    writeStringSetting(APP_SETTING_KEYS.appLanguage, language);
    writeStringSetting(APP_SETTING_KEYS.userLanguage, label);
    setResponseLanguageState(label);
    setUserLanguageState(label);
    if (runtimeReady) {
      void updateActiveCharacterRuntime({ responseLanguage: label }).catch(console.error);
    } else {
      // Apply through the activation owner once the initial committed runtime
      // has been recovered; never mutate the character response cache here.
      pendingOnboardingResponseLanguageRef.current = label;
    }
    setUserLanguage(label).catch(console.error);
  };

  function dispatchOnboardingEvent(event: OnboardingFlowEvent): void {
    setOnboardingDraft((previous) => {
      const next = onboardingFlowReducer(previous, event);
      localStorage.setItem(ONBOARDING_DRAFT_KEY, serializeOnboardingDraft(next));
      return next;
    });
  }

  const previewOnboardingLanguage = (language: OnboardingLanguageCode) => {
    dispatchOnboardingEvent({ type: "select-language", language });
    applyOnboardingLanguage(language);
  };

  const handleOnboardingCharacterSelect = async (characterId: string): Promise<void> => {
    setOnboardingCharacterError(null);
    try {
      await characterActivation.activateCharacter(characterId);
      dispatchOnboardingEvent({ type: "select-character", characterId });
    } catch {
      const message = t("onboarding.workflow.errors.character_activate", {
        defaultValue: "We couldn't activate this character. Check the character package and retry.",
      });
      setOnboardingCharacterError(message);
      throw new Error(message);
    }
  };

  const handleOnboardingProviderSave = async (): Promise<void> => {
    setOnboardingSavingProvider(true);
    setOnboardingProviderError(null);
    try {
      if (!llmConfig) throw new Error("provider configuration is still loading");
      const saved = await saveProviderSetup(llmConfig, onboardingProviderSetup);
      setLlmConfig(saved);
      dispatchOnboardingEvent({ type: "configure-provider", providerId: saved.active_provider });
    } catch {
      const message = t("onboarding.workflow.errors.provider_save", {
        defaultValue: "We couldn't save this provider. Check the endpoint, model, and key, then retry.",
      });
      setOnboardingProviderError(message);
      throw new Error(message);
    } finally {
      setOnboardingSavingProvider(false);
    }
  };

  const handleOnboardingConnectionTest = async (): Promise<void> => {
    dispatchOnboardingEvent({ type: "connection-test-started" });
    setOnboardingTestingConnection(true);
    setOnboardingConnectionResult(null);
    try {
      if (!llmConfig) throw new Error("provider configuration is still loading");
      const result = await testProviderSetup(llmConfig, onboardingProviderSetup);
      setOnboardingConnectionResult(result);
      dispatchOnboardingEvent({ type: "connection-test-succeeded" });
    } catch {
      dispatchOnboardingEvent({
        type: "connection-test-failed",
        error: t("onboarding.workflow.errors.connection_failed", {
          defaultValue: "We couldn't reach this provider. Check the endpoint, model, and key, then retry.",
        }),
      });
    } finally {
      setOnboardingTestingConnection(false);
    }
  };

  const handleOnboardingChatSubmit = async (message: string): Promise<string> => {
    if (onboardingChatPendingRef.current) throw new Error("a chat turn is already in progress");
    dispatchOnboardingEvent({ type: "chat-started" });
    setOnboardingSubmittingChat(true);
    return new Promise<string>((resolve, reject) => {
      const clientRequestId = crypto.randomUUID();
      onboardingChatPendingRef.current = { clientRequestId, turnId: null, reply: "", resolve, reject };
      void streamChat({
        message,
        character_id: readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || undefined,
        client_request_id: clientRequestId,
      }).catch((error) => {
        const pending = onboardingChatPendingRef.current;
        onboardingChatPendingRef.current = null;
        setOnboardingSubmittingChat(false);
        pending?.reject(error instanceof Error ? error : new Error(getKokoroErrorMessage(error)));
      });
    });
  };

  const handleOnboardingFirstReplySucceeded = (reply: string): void => {
    dispatchOnboardingEvent({ type: "first-reply-succeeded", reply });
    localStorage.setItem(ONBOARDING_STATUS_KEY, "completed");
    setOnboardingOpen(false);
  };

  const cancelOnboardingChat = (): void => {
    const pending = onboardingChatPendingRef.current;
    onboardingChatPendingRef.current = null;
    setOnboardingSubmittingChat(false);
    // Resolve with an empty reply so the overlay cannot accidentally mark the
    // flow complete after the user dismissed the pending turn.
    pending?.resolve("");
  };

  const refreshMemoryModelStatus = useCallback(async () => {
    const status = await getMemoryEmbeddingModelStatus();
    setMemoryModelStatus(status);
    if (status.installed) {
      setMemoryModelError(null);
    }
    return status;
  }, []);

  const startMemoryModelDownload = useCallback(async () => {
    if (memoryModelDownloadInFlightRef.current) {
      return;
    }

    memoryModelDownloadInFlightRef.current = true;
    setMemoryModelDialogOpen(true);
    setMemoryModelDownloading(true);
    setMemoryModelError(null);
    setMemoryModelProgress({
      stage: "checking",
      message: "",
      current_file: "",
      file_index: 0,
      file_count: 0,
      downloaded_bytes: 0,
      total_bytes: null,
    });

    try {
      const status = await downloadMemoryEmbeddingModel();
      setMemoryModelStatus(status);
    } catch (error) {
      setMemoryModelError(getKokoroErrorMessage(error));
    } finally {
      memoryModelDownloadInFlightRef.current = false;
      setMemoryModelDownloading(false);
      refreshMemoryModelStatus().catch((err) => {
        console.error("[App] Failed to refresh memory model status:", err);
      });
    }
  }, [refreshMemoryModelStatus]);

  const openMemoryModelDialog = useCallback(async () => {
    setMemoryModelDialogOpen(true);

    try {
      const status = memoryModelStatus ?? await refreshMemoryModelStatus();
      if (!status.installed && !memoryModelDownloadInFlightRef.current && !memoryModelError) {
        void startMemoryModelDownload();
      }
    } catch (error) {
      setMemoryModelError(getKokoroErrorMessage(error));
    }
  }, [memoryModelError, memoryModelStatus, refreshMemoryModelStatus, startMemoryModelDownload]);

  const closeOnboarding = (status: "completed" | "dismissed") => {
    if (status === "dismissed") {
      cancelOnboardingChat();
      dispatchOnboardingEvent({ type: "dismiss" });
      setOnboardingOpen(false);
      return;
    }
    if (!onboardingDraft.completed) return;
    localStorage.setItem(ONBOARDING_STATUS_KEY, status);
    setOnboardingOpen(false);
  };

  useEffect(() => {
    refreshMemoryModelStatus().catch((err) => {
      console.error("[App] Failed to load memory model status:", err);
      setMemoryModelError(getKokoroErrorMessage(err));
    });
  }, [refreshMemoryModelStatus]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    onMemoryEmbeddingModelProgress((progress) => {
      setMemoryModelProgress(progress);
      if (progress.stage === "ready") {
        setMemoryModelDownloading(false);
        setMemoryModelError(null);
      }
    }).then((fn) => {
      unlisten = fn;
    }).catch((err) => {
      console.error("[App] Failed to listen for memory model progress:", err);
    });

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (onboardingOpen && onboardingDraft.step === "chat" && memoryModelStatus && !memoryModelStatus.installed) {
      void openMemoryModelDialog();
    }
  }, [memoryModelStatus, onboardingDraft.step, onboardingOpen, openMemoryModelDialog]);

  useEffect(() => {
    const handleRequireDialog = () => {
      void openMemoryModelDialog();
    };

    window.addEventListener(MEMORY_MODEL_DIALOG_EVENT, handleRequireDialog);
    return () => window.removeEventListener(MEMORY_MODEL_DIALOG_EVENT, handleRequireDialog);
  }, [openMemoryModelDialog]);

  useEffect(() => {
    const sync = () => {
      setWindowSize(window.innerWidth, window.innerHeight).catch(console.error);
    };
    sync();
    window.addEventListener('resize', sync);
    return () => window.removeEventListener('resize', sync);
  }, []);

  useEffect(() => {
    let aborted = false;
    let cleanup: (() => void) | null = null;
    void listen<QQAuthorizationRequest>("qq-authorization-expired", event => {
      if (aborted) return;
      setQQAuthorizationQueue(current => current.filter(
        request => request.request_id !== event.payload.request_id,
      ));
      setQQAuthorizationError(null);
    }).then(unlisten => {
      if (aborted) {
        unlisten();
        return;
      }
      cleanup = unlisten;
    });
    return () => {
      aborted = true;
      cleanup?.();
    };
  }, []);

  useEffect(() => {
    let aborted = false;
    let cleanup: (() => void) | null = null;
    void listen<QQAuthorizationRequest>("qq-authorization-request", event => {
      if (aborted) return;
      setQQAuthorizationError(null);
      setQQAuthorizationQueue(current => {
        if (current.some(request => request.request_id === event.payload.request_id)) {
          return current;
        }
        return [...current, event.payload];
      });
    }).then(unlisten => {
      if (aborted) {
        unlisten();
        return;
      }
      cleanup = unlisten;
    });
    return () => {
      aborted = true;
      cleanup?.();
    };
  }, []);

  const respondToCurrentQQAuthorization = useCallback(async (approved: boolean) => {
    if (!currentQQAuthorization || qqAuthorizationBusy) return;
    setQQAuthorizationBusy(true);
    setQQAuthorizationError(null);
    try {
      await respondQQAuthorization(currentQQAuthorization, approved);
      setQQAuthorizationQueue(current => current.filter(
        request => request.request_id !== currentQQAuthorization.request_id,
      ));
    } catch (error) {
      const message = getKokoroErrorMessage(error);
      if (message.includes("no longer pending")) {
        setQQAuthorizationQueue(current => current.filter(
          request => request.request_id !== currentQQAuthorization.request_id,
        ));
      } else {
        setQQAuthorizationError(message);
      }
    } finally {
      setQQAuthorizationBusy(false);
    }
  }, [currentQQAuthorization, qqAuthorizationBusy]);

  // Listen for pet window requesting main window to show
  useEffect(() => {
    import("@tauri-apps/api/event").then(({ listen }) => {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        const unlisten = listen("show-main-window", async () => {
          const win = getCurrentWindow();
          await win.unminimize().catch(console.error);
          await win.setFocus().catch(console.error);
        });
        return unlisten;
      });
    });
  }, []);

  useEffect(() => {
    ttsService.init();

    invoke<PetConfig>("get_pet_config")
      .then((cfg) => setRenderFps(typeof cfg.render_fps === "number" ? cfg.render_fps : 60))
      .catch(err => console.error("[App] Failed to load pet config:", err));

    // Fetch initial data for settings
    // Fetch initial data — split into fast (configs) and slow (scans) batches
    // so core settings reach the mod iframe faster.
    Promise.all([
      getTtsConfig(),
      getLlmConfig(),
      getSttConfig(),
      getVisionConfig(),
      getImageGenConfig(),
      listMcpServers(),
      listMods(),
      getProactiveEnabled(),
      getTelegramConfig(),
      getTelegramStatus(),
      getBotConfig(),
      getBotStatus(),
      getAutoBackupConfig(),
    ]).then(([tts, llm, stt, vision, imageGen, mcp, mods, proactive, telegram, telegramStatus, botConfig, botStatus, autoBackup]) => {
      setTtsConfig(tts);
      setLlmConfig(llm);
      const activeProvider = llm.providers.find((provider) => provider.id === llm.active_provider) ?? llm.providers[0];
      if (activeProvider) setOnboardingProviderSetup(providerToSetup(activeProvider));
      setSttConfig(stt);
      setVisionConfig(vision);
      setImageGenConfig(imageGen);
      setMcpServers(mcp);
      setModList(mods);
      setProactiveEnabledState(proactive);
      writeBooleanSetting(APP_SETTING_KEYS.proactiveEnabled, proactive);
      setTelegramConfig(telegram);
      setTelegramStatus(telegramStatus);
      setBotConfig(botConfig);
      setBotStatus(botStatus);
      setAutoBackupConfig(autoBackup);
    }).catch(err => console.error("[App] Failed to fetch initial configs:", err));

    // User language is application-wide. Character response language is
    // restored by the committed activation runtime below.
    const savedUserLang = readStringSetting(APP_SETTING_KEYS.userLanguage, "");
    if (savedUserLang) setUserLanguage(savedUserLang).catch(console.error);

    const userProfileHydration = getUserProfileSettings()
      .then((profile) => {
        if (!profile) return;
        writeStringSetting(APP_SETTING_KEYS.userName, profile.user_name);
        writeStringSetting(APP_SETTING_KEYS.userPersona, profile.user_persona);
      })
      .catch(err => console.error("[App] Failed to restore user profile:", err));

    // These may be slower (file system scans, network)
    listLive2dModels()
      .then(models => setAvailableModels(models))
      .catch(err => console.error("[App] Failed to list Live2D models:", err));
    listTtsProviders()
      .then(prov => setTtsProviders(prov))
      .catch(err => console.error("[App] Failed to list TTS providers:", err));
    listTtsVoices()
      .then(voices => setTtsVoices(voices))
      .catch(err => console.error("[App] Failed to list TTS voices:", err));

    // Backend committed runtime is authoritative for startup and window recreation.
    userProfileHydration.finally(async () => {
      try {
        const [all, templates] = await Promise.all([listCharacters(), listCharacterTemplates()]);
        setCharacters(all);
        setCharacterTemplates(templates);
        const recovered = await characterActivation.recoverCommittedRuntime();
        if (recovered === null) {
          const savedId = readStringSetting(APP_SETTING_KEYS.activeCharacterId, "");
          const fallback = savedId ? all.find((character) => character.id === savedId) ?? all[0] : all[0];
          if (fallback) {
            await characterActivation.activateCharacter(fallback.id);
          }
        }
        const pendingLanguage = pendingOnboardingResponseLanguageRef.current;
        if (pendingLanguage !== null) {
          pendingOnboardingResponseLanguageRef.current = null;
          await updateActiveCharacterRuntime({ responseLanguage: pendingLanguage });
        }
      } catch (e) {
        console.error("[App] Failed to restore committed character runtime:", e);
      } finally {
        setRuntimeReady(true);
      }
    });

    // Listen for generated images
    const unlistenImageGen = onImageGenDone((result: ImageGenResult) => {
      const assetUrl = convertFileSrc(result.image_url);
      console.log("[App] Received generated image:", assetUrl);
      setGeneratedImage(assetUrl);
    });

    // Listen for chat-triggered image generation requests
    const unlistenChatImageGen = onChatImageGen(({ prompt }) => {
      console.log("[App] chat-imagegen triggered, prompt:", prompt);
      generateImage(prompt).then(result => {
        const assetUrl = convertFileSrc(result.image_url);
        setGeneratedImage(assetUrl);
        bgSlideshow.setConfig({ mode: "generated" });
      }).catch(err => console.error("[App] chat-imagegen generation failed:", err));
    });

    // ── MOD System: Theme override ──
    const unlistenModTheme = onModThemeOverride((modTheme: ModThemeJson) => {
      console.log("[App] Mod theme override received:", modTheme.name || modTheme.id);
      // Convert ModThemeJson to ThemeConfig for ThemeProvider
      const themeConfig: ThemeConfig = {
        id: modTheme.id || "mod-theme",
        name: modTheme.name || "Mod Theme",
        variables: modTheme.variables,
        assets: modTheme.assets ? {
          fonts: modTheme.assets.fonts,
          background: modTheme.assets.background,
          noise_texture: modTheme.assets.noise_texture,
        } : undefined,
        animations: modTheme.animations,
      };
      // Apply the theme (ThemeProvider listens for setTheme calls)
      // We need to access setTheme from context — handled via event
      document.dispatchEvent(new CustomEvent("kokoro:mod-theme", { detail: themeConfig }));
    });

    // ── MOD System: Component registration ──
    const unlistenModComponents = onModComponentsRegister((components) => {
      console.log("[App] Mod components registered:", Object.keys(components));
      for (const [slot, src] of Object.entries(components)) {
        // Extract modId from the mod:// URL: mod://modId/path
        const modId = src.replace("mod://", "").split("/")[0];
        registry.registerModComponent(slot, modId, src);
      }
      // registry.notify() fires automatically from registerModComponent,
      // which triggers useSyncExternalStore subscribers in both
      // App (for SettingsPanel slot) and LayoutRenderer (for ChatPanel slot).
    });

    // ── MOD System: UI message forwarding (QuickJS → iframe) ──
    const unlistenModUiMessage = onModUiMessage(({ component, payload }) => {
      console.log(`[App] Forwarding ui-message to component '${component}'`);
      modMessageBus.send(component, {
        type: 'event',
        payload: { name: 'script-data', data: payload },
      });
    });

    // ── MOD System: Engine event bridge → broadcast to iframes + forward to QuickJS ──
    const unlistenOnboardingChatStart = onChatTurnStart(({ turn_id, client_request_id }) => {
      const onboardingPending = onboardingChatPendingRef.current;
      if (onboardingPending && isOnboardingTurnEvent(onboardingPending.clientRequestId, client_request_id)) {
        onboardingPending.turnId = turn_id;
      }
    });

    const unlistenModChatDelta = onChatTurnDelta(({ turn_id, delta, client_request_id }) => {
      const onboardingPending = onboardingChatPendingRef.current;
      if (onboardingPending
        && isOnboardingTurnEvent(onboardingPending.clientRequestId, client_request_id)
        && onboardingPending.turnId === turn_id) {
        onboardingPending.reply += delta;
      }
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-delta', delta, turn_id },
      });
      // Forward to QuickJS scripts so Kokoro.on('chat', ...) works
      dispatchModEvent('chat', { delta, turn_id }).catch(() => { });
    });

    const unlistenModCue = onChatCue((data) => {
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-cue', ...data },
      });
      dispatchModEvent('cue', data).catch(() => { });
    });

    const unlistenModChatDone = onChatTurnFinish(({ turn_id, status, client_request_id }) => {
      const onboardingPending = onboardingChatPendingRef.current;
      if (onboardingPending
        && isOnboardingTurnEvent(onboardingPending.clientRequestId, client_request_id)
        && onboardingPending.turnId === turn_id) {
        onboardingChatPendingRef.current = null;
        setOnboardingSubmittingChat(false);
        if (status === "completed") {
          onboardingPending.resolve(onboardingPending.reply);
        } else {
          onboardingPending.reject(new Error(`chat turn ${status}`));
        }
      }
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-done', turn_id, status },
      });
      dispatchModEvent('chat-done', { turn_id, status }).catch(() => { });
    });

    // ── MOD System: Script events → broadcast to iframes ──
    const unlistenModScriptEvent = onModScriptEvent(({ event, payload }) => {
      console.log(`[App] Script event '${event}' → broadcasting to iframes`);
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: `script:${event}`, data: payload },
      });
    });

    // ── MOD System: Unload — reset to native mode ──
    const unlistenModUnload = onModUnload(() => {
      console.log("[App] Mod unloaded, restoring native mode");
      // 清除所有 mod 注册的组件
      registry.clearAllModComponents();
      // 重新注册核心组件
      registerCoreComponents();
      // 重置主题：通知 ThemeProvider 恢复默认
      document.dispatchEvent(new CustomEvent("kokoro:mod-theme", { detail: null }));
    });

    return () => {
      ttsService.cleanup();
      unlistenImageGen.then(unlisten => unlisten());
      unlistenChatImageGen.then(unlisten => unlisten());
      unlistenModTheme.then(unlisten => unlisten());
      unlistenModComponents.then(unlisten => unlisten());
      unlistenOnboardingChatStart.then(unlisten => unlisten());
      unlistenModUiMessage.then(unlisten => unlisten());
      unlistenModChatDelta.then(unlisten => unlisten());
      unlistenModCue.then(unlisten => unlisten());
      unlistenModChatDone.then(unlisten => unlisten());
      unlistenModScriptEvent.then(unlisten => unlisten());
      unlistenModUnload.then(unlisten => unlisten());
    };
  }, []);

  const formatBytes = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const handleExportBackup = async () => {
    setBackupStatus({ phase: "exporting", message: "正在导出备份..." });
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const filePath = await save({
        defaultPath: `kokoro-backup-${new Date().toISOString().slice(0, 10)}.kokoro`,
        filters: [{ name: 'Kokoro Backup', extensions: ['kokoro'] }],
      });
      if (!filePath) {
        setBackupStatus({ phase: "idle" });
        return;
      }
      const result = await exportData(filePath, { include_character_resources: false });
      setBackupStatus({
        phase: "exported",
        message: `已导出 ${formatBytes(result.size_bytes)} · 记忆 ${result.stats.memories} · 对话 ${result.stats.conversations} · 配置 ${result.stats.configs}`,
      });
    } catch (error) {
      setBackupStatus({ phase: "error", error: getKokoroErrorMessage(error) });
    }
  };

  const handleSelectImportBackup = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Kokoro Backup', extensions: ['kokoro'] }],
      });
      if (!selected) return;
      const filePath = Array.isArray(selected) ? selected[0] : selected;
      const preview = await previewImport(filePath);
      setBackupStatus({ phase: "preview", preview, importFilePath: filePath });
    } catch (error) {
      setBackupStatus({ phase: "error", error: getKokoroErrorMessage(error) });
    }
  };

  const handleConfirmImportBackup = async (options: {
    import_database?: boolean;
    import_configs?: boolean;
    conflict_strategy?: "skip" | "overwrite";
  }) => {
    const filePath = backupStatus.importFilePath;
    if (!filePath) return;
    setBackupStatus(prev => ({ ...prev, phase: "importing", message: "正在导入备份..." }));
    try {
      let payload: any = null;
      let targetCharacterId: string | undefined;
      const importDb = options.import_database ?? true;
      const importConfigs = options.import_configs ?? true;
      const conflictStrategy = options.conflict_strategy ?? "overwrite";

      const firstPass = await importData(filePath, {
        import_database: false,
        import_configs: false,
        conflict_strategy: conflictStrategy,
      });

      if (firstPass.characters_json && importDb) {
        payload = JSON.parse(firstPass.characters_json);
        const chars = payload.characters ?? payload;

        if (payload.userName != null) {
          writeStringSetting(APP_SETTING_KEYS.userName, payload.userName);
          await setUserName(payload.userName);
        }
        if (payload.userPersona != null) {
          writeStringSetting(APP_SETTING_KEYS.userPersona, payload.userPersona);
          await setUserPersona(payload.userPersona);
        }
        if (payload.userLanguage != null) writeStringSetting(APP_SETTING_KEYS.userLanguage, payload.userLanguage);
        if (payload.voiceInterrupt != null) writeStringSetting(APP_SETTING_KEYS.voiceInterrupt, payload.voiceInterrupt);

        const newIds: number[] = [];
        for (const char of chars) {
          let avatarBlob: Blob | undefined;
          if (char.avatarB64) {
            const bytes = Uint8Array.from(atob(char.avatarB64), c => c.charCodeAt(0));
            avatarBlob = new Blob([bytes]);
          }
          const { avatarB64: _avatarB64, id: _oldId, ...rest } = char;
          const newId = await characterDb.add({ ...rest, avatarBlob });
          newIds.push(newId);
        }
        const existing = await characterDb.getAll();
        for (const char of existing) {
          if (char.id !== undefined && !newIds.includes(char.id)) await characterDb.remove(char.id);
        }

        targetCharacterId = payload.activeCharacterId || chars[0]?.stableId;
      }

      if (!targetCharacterId) {
        targetCharacterId = readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || undefined;
      }

      const result = await importData(filePath, {
        import_database: importDb,
        import_configs: importConfigs,
        conflict_strategy: conflictStrategy,
        target_character_id: targetCharacterId,
      });

      if (payload?.userName != null) {
        writeStringSetting(APP_SETTING_KEYS.userName, payload.userName);
        await setUserName(payload.userName);
      }
      if (payload?.userPersona != null) {
        writeStringSetting(APP_SETTING_KEYS.userPersona, payload.userPersona);
        await setUserPersona(payload.userPersona);
      }

      setBackupStatus({
        phase: "imported",
        message: `导入完成 · 记忆 ${result.imported_memories} · 对话 ${result.imported_conversations} · 配置 ${result.imported_configs}`,
      });
      window.setTimeout(() => window.location.reload(), 1500);
    } catch (error) {
      setBackupStatus(prev => ({ ...prev, phase: "error", importFilePath: prev.importFilePath, preview: prev.preview, error: getKokoroErrorMessage(error) }));
    }
  };

  const handlePickAutoBackupDir = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      const next = {
        ...(autoBackupConfig ?? { enabled: false, backup_dir: "", interval_days: 1, auto_cleanup: false, keep_days: 30 }),
        backup_dir: selected,
      };
      setAutoBackupConfig(next);
      await saveAutoBackupConfig(next);
    } catch (error) {
      setBackupStatus({ phase: "error", error: getKokoroErrorMessage(error) });
    }
  };

  // ── MOD System: Action listener for UI components ──
  const dispatchSimpleModAction = createModActionDispatcher({
    close_settings: () => {
      setSettingsOpen(false);
    },
    set_display_mode: ({ data }) => {
      if (data && typeof data === "object") {
        const mode = (data as { mode?: unknown }).mode;
        if (mode) {
          handleDisplayModeChange(String(mode) as Live2DDisplayMode);
        }
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

  const handleModAction = (e: Event) => {
    const action = getModActionFromEvent(e);
    if (!action) return;

    void dispatchSimpleModAction(action).then((handled) => {
      if (handled) return;
      handleLegacyModAction(action as LegacyModActionEnvelope);
    }).catch((err) => {
      console.error("[App] Mod action dispatch failed:", err);
    });
  };

  const handleLegacyModAction = (detail: LegacyModActionEnvelope) => {
    if (detail.action === 'send_message' && detail.data?.message) {
      const message = detail.data.message;
      void (async () => {
        try {
          const status = memoryModelStatus ?? await refreshMemoryModelStatus();
          if (!status.installed) {
            await openMemoryModelDialog();
            return;
          }
          await streamChat({
            message,
            character_id: readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || undefined,
          });
        } catch (err) {
          console.error("[App] Mod send_message failed:", err);
        }
      })();
    }
    // New settings actions
    if (detail.action === 'set_model' && detail.data?.model) {
      const model = detail.data.model;
      const target = availableModels.find(m => m.name === model || m.path === model);
      if (target) void updateActiveCharacterRuntime({ live2dModel: target.path }).catch(console.error);
    }
    if (detail.action === 'set_persona' && detail.data?.persona) {
      void updateActiveCharacterRuntime({ persona: detail.data.persona }).catch(console.error);
    }
    if (detail.action === 'set_language' && detail.data?.language) {
      void updateActiveCharacterRuntime({ responseLanguage: detail.data.language }).catch(console.error);
    }
    // Full Config Save Handlers
    if (detail.action === 'save_llm_config' && detail.data?.config) {
      setLlmConfig(detail.data.config);
      saveLlmConfig(detail.data.config).catch(console.error);
    }
    if (detail.action === 'save_tts_config' && detail.data?.config) {
      const config = detail.data.config as TtsSystemConfig;
      setTtsConfig(config);
      saveTtsConfig(config).then(() => {
        // Refresh providers & voices after save
        Promise.all([listTtsProviders(), listTtsVoices()]).then(([p, v]) => {
          setTtsProviders(p);
          setTtsVoices(v);
        }).catch(err => console.error("[App] Failed to refresh TTS lists:", err));
        const provider = config.providers.find((candidate) => candidate.id === config.default_provider) ?? null;
        return updateActiveCharacterRuntime({
          tts: {
            enabled: provider?.enabled === true,
            providerId: provider?.id ?? null,
            providerType: provider?.provider_type ?? null,
            voice: provider?.default_voice ?? null,
            speed: 1,
            pitch: 1,
          },
        });
      }).catch(console.error);
    }
    if (detail.action === 'save_stt_config' && detail.data?.config) {
      setSttConfig(detail.data.config);
      saveSttConfig(detail.data.config).catch(console.error);
    }
    if (detail.action === 'save_image_gen_config' && detail.data?.config) {
      setImageGenConfig(detail.data.config);
      saveImageGenConfig(detail.data.config).catch(console.error);
    }
    if (detail.action === 'save_vision_config' && detail.data?.config) {
      setVisionConfig(detail.data.config);
      saveVisionConfig(detail.data.config).catch(console.error);
    }
    if (detail.action === 'save_telegram_config' && detail.data?.config) {
      setTelegramConfig(detail.data.config);
      saveTelegramConfig(detail.data.config)
        .then(() => getTelegramStatus())
        .then(setTelegramStatus)
        .catch(console.error);
    }
    if (detail.action === 'save_bot_config' && detail.data?.config) {
      setBotConfig(detail.data.config);
      saveBotConfig(detail.data.config)
        .then(savedConfig => {
          setBotConfig(savedConfig);
          return Promise.all([getBotStatus(), getTelegramConfig(), getTelegramStatus()]);
        })
        .then(([status, telegram, telegramStatus]) => {
          setBotStatus(status);
          setTelegramConfig(telegram);
          setTelegramStatus(telegramStatus);
        })
        .catch(console.error);
    }
    if (detail.action === 'refresh_bot_status') {
      Promise.all([getBotConfig(), getBotStatus(), getTelegramStatus()])
        .then(([config, status, telegramStatus]) => {
          setBotConfig(config);
          setBotStatus(status);
          setTelegramStatus(telegramStatus);
        })
        .catch(console.error);
    }
    if (detail.action === 'start_bot_platform' && detail.data?.platform) {
      const platform = detail.data.platform as BotPlatformId;
      const start = platform === "telegram"
        ? startTelegramBot()
        : startBotPlatform(platform as Exclude<BotPlatformId, "telegram">);
      start
        .then(() => Promise.all([getBotStatus(), getTelegramStatus()]))
        .then(([status, telegramStatus]) => {
          setBotStatus(status);
          setTelegramStatus(telegramStatus);
        })
        .catch(console.error);
    }
    if (detail.action === 'stop_bot_platform' && detail.data?.platform) {
      const platform = detail.data.platform as BotPlatformId;
      const stop = platform === "telegram"
        ? stopTelegramBot()
        : stopBotPlatform(platform as Exclude<BotPlatformId, "telegram">);
      stop
        .then(() => Promise.all([getBotStatus(), getTelegramStatus()]))
        .then(([status, telegramStatus]) => {
          setBotStatus(status);
          setTelegramStatus(telegramStatus);
        })
        .catch(console.error);
    }
    if (detail.action === 'export_backup') {
      void handleExportBackup();
    }
    if (detail.action === 'select_import_backup') {
      void handleSelectImportBackup();
    }
    if (detail.action === 'confirm_import_backup') {
      void handleConfirmImportBackup(detail.data ?? {});
    }
    if (detail.action === 'save_auto_backup_config' && detail.data?.config) {
      setAutoBackupConfig(detail.data.config);
      saveAutoBackupConfig(detail.data.config)
        .then(() => setBackupStatus({ phase: "idle", message: "自动备份设置已保存" }))
        .catch(error => setBackupStatus({ phase: "error", error: getKokoroErrorMessage(error) }));
    }
    if (detail.action === 'pick_auto_backup_dir') {
      void handlePickAutoBackupDir();
    }
    if (detail.action === 'run_auto_backup_now') {
      setBackupStatus({ phase: "auto-running", message: "正在执行自动备份..." });
      runAutoBackupNow()
        .then(path => setBackupStatus({ phase: "auto-ran", message: `自动备份完成：${path}` }))
        .catch(error => setBackupStatus({ phase: "error", error: getKokoroErrorMessage(error) }));
    }

    // New Actions for Mod Settings
    if (detail.action === 'fetch_llm_models' && detail.data) {
      // detail.data should contain { providerType, baseUrl, apiKey }
      const { providerType, baseUrl, apiKey } = detail.data;
      if (providerType === 'ollama') {
        listOllamaModels(baseUrl || "http://localhost:11434")
          .then(models => setFetchedLlmModels(models.map(m => m.name)))
          .catch(err => console.error("[App] Failed to list Ollama models:", err));
      } else if (providerType === 'anthropic') {
        listAnthropicModels(baseUrl || "https://api.anthropic.com/v1", apiKey || "")
          .then(models => setFetchedLlmModels(models))
          .catch(err => console.error("[App] Failed to list Anthropic models:", err));
      } else if (providerType === 'llama_cpp') {
        getLlamaCppStatus(baseUrl || "http://127.0.0.1:8080")
          .then(status => setFetchedLlmModels(status.available_models))
          .catch(err => console.error("[App] Failed to inspect llama.cpp server:", err));
      } else {
        fetchModels(baseUrl || "https://api.openai.com/v1", apiKey || "")
          .then(models => setFetchedLlmModels(models))
          .catch(err => console.error("[App] Failed to fetch LLM models:", err));
      }
    }

    if (detail.action === 'scan_gpt_sovits_models' && detail.data) {
      const { providerId, installPath } = detail.data;
      if (installPath) {
        listGptSovitsModels(installPath)
          .then(models => setScannedTtsModels(prev => ({ ...prev, [providerId]: models })))
          .catch(err => console.error("[App] Failed to scan GPT-SoVITS:", err));
      }
    }

    if (detail.action === 'export_jailbreak_prompt') {
      void (async () => {
        try {
          const [{ save }, { writeTextFile }, prompt] = await Promise.all([
            import('@tauri-apps/plugin-dialog'),
            import('@tauri-apps/plugin-fs'),
            getJailbreakPrompt(),
          ]);
          const filePath = await save({
            defaultPath: 'jailbreak_prompt.txt',
            filters: [{ name: 'Text', extensions: ['txt'] }],
          });
          if (filePath) await writeTextFile(filePath, prompt);
        } catch (err) {
          console.error('[App] Jailbreak export failed:', err);
        }
      })();
    }
    if (detail.action === 'import_jailbreak_prompt') {
      void (async () => {
        try {
          const [{ open }, { readTextFile }] = await Promise.all([
            import('@tauri-apps/plugin-dialog'),
            import('@tauri-apps/plugin-fs'),
          ]);
          const selected = await open({
            multiple: false,
            filters: [{ name: 'Text', extensions: ['txt'] }],
          });
          if (selected && typeof selected === 'string') {
            const content = await readTextFile(selected);
            await setJailbreakPrompt(content);
          }
        } catch (err) {
          console.error('[App] Jailbreak import failed:', err);
        }
      })();
    }

    if (detail.action === 'set_proactive_enabled') {
      const enabled = !!detail.data?.enabled;
      void updateActiveCharacterRuntime({ proactiveEnabled: enabled }).catch(console.error);
    }

    // ── Background Config Actions ────────────────────
    if (detail.action === 'set_bg_config' && detail.data) {
      bgSlideshow.setConfig(detail.data);
    }
    if (detail.action === 'remove_bg_image' && detail.data?.index !== undefined) {
      bgSlideshow.removeImage(detail.data.index);
    }
    if (detail.action === 'clear_bg_images') {
      bgSlideshow.clearImages();
    }
    if (detail.action === 'import_bg_images') {
      import('@tauri-apps/plugin-dialog').then(async ({ open }) => {
        const selected = await open({
          multiple: true,
          filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'] }],
        });
        if (!selected) return;
        const paths = Array.isArray(selected) ? selected : [selected];
        const { readFile } = await import('@tauri-apps/plugin-fs');
        const files: File[] = [];
        for (const p of paths) {
          try {
            const data = await readFile(p);
            const ext = p.split('.').pop()?.toLowerCase() || 'png';
            const mimeMap: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif', bmp: 'image/bmp' };
            const name = p.split(/[\\/]/).pop() || 'image';
            files.push(new File([data], name, { type: mimeMap[ext] || 'image/png' }));
          } catch (e) { console.error('[App] Failed to read bg image:', p, e); }
        }
        if (files.length > 0) {
          const dt = new DataTransfer();
          files.forEach(f => dt.items.add(f));
          await bgSlideshow.importFiles(dt.files);
        }
      }).catch(err => console.error('[App] import_bg_images failed:', err));
    }
    if (detail.action === 'import_bg_folder') {
      import('@tauri-apps/plugin-dialog').then(async ({ open }) => {
        const selected = await open({ directory: true, multiple: false });
        if (!selected || Array.isArray(selected)) return;
        const { readDir, readFile } = await import('@tauri-apps/plugin-fs');
        const imageExts = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp']);
        const mimeMap: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif', bmp: 'image/bmp' };
        const files: File[] = [];
        const joinPath = (base: string, name: string) => `${base.replace(/[\\/]+$/, '')}\\${name}`;
        const walk = async (dir: string) => {
          const entries = await readDir(dir);
          for (const entry of entries) {
            const childPath = joinPath(dir, entry.name);
            if (entry.isDirectory) {
              await walk(childPath);
              continue;
            }
            const ext = entry.name.split('.').pop()?.toLowerCase() || '';
            if (!imageExts.has(ext)) continue;
            try {
              const data = await readFile(childPath);
              files.push(new File([data], entry.name, { type: mimeMap[ext] || 'image/png' }));
            } catch (e) {
              console.error('[App] Failed to read bg folder image:', childPath, e);
            }
          }
        };
        await walk(selected);
        if (files.length > 0) {
          const dt = new DataTransfer();
          files.forEach(f => dt.items.add(f));
          await bgSlideshow.importFiles(dt.files);
        }
      }).catch(err => console.error('[App] import_bg_folder failed:', err));
    }

    // ── TTS Playback Actions ────────────────────────
    if (detail.action === 'set_tts_enabled') {
      void updateActiveCharacterRuntime({
        tts: { ...readActiveCharacterTtsRuntime(), enabled: !!detail.data?.enabled },
      }).catch(console.error);
    }
    if (detail.action === 'set_tts_speed' && detail.data?.speed !== undefined) {
      void updateActiveCharacterRuntime({
        tts: { ...readActiveCharacterTtsRuntime(), speed: Number(detail.data.speed) || 1 },
      }).catch(console.error);
    }
    if (detail.action === 'set_tts_pitch' && detail.data?.pitch !== undefined) {
      void updateActiveCharacterRuntime({
        tts: { ...readActiveCharacterTtsRuntime(), pitch: Number(detail.data.pitch) || 1 },
      }).catch(console.error);
    }
    if (detail.action === 'test_tts') {
      synthesize("Hello! This is a test of the TTS system.", {
        provider_id: readStringSetting(APP_SETTING_KEYS.ttsProvider, "") || undefined,
        voice: readStringSetting(APP_SETTING_KEYS.ttsVoice, "") || undefined,
        speed: readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1.0),
        pitch: readNumberSetting(APP_SETTING_KEYS.ttsPitch, 1.0),
      }).catch(err => console.error('[App] TTS test failed:', err));
    }
    if (detail.action === 'set_tts_playback' && detail.data) {
      const { speed, pitch, voice, provider } = detail.data;
      const current = readActiveCharacterTtsRuntime();
      const providerId = provider === undefined ? current.providerId : String(provider);
      const providerConfig = ttsConfig?.providers.find((candidate) => candidate.id === providerId);
      void updateActiveCharacterRuntime({
        tts: {
          ...current,
          providerId,
          providerType: providerConfig?.provider_type ?? current.providerType,
          speed: speed === undefined ? current.speed : Number(speed) || 1,
          pitch: pitch === undefined ? current.pitch : Number(pitch) || 1,
          voice: voice === undefined ? current.voice : String(voice),
        },
      }).catch(console.error);
    }

    // ── MCP Actions ────────────────────────────────
    if (detail.action === 'list_mcp_servers') {
      listMcpServers()
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP list failed:', err));
    }
    if (detail.action === 'add_mcp_server' && (detail.data?.config || detail.data?.json)) {
      let configs: McpServerConfig[];
      try {
        configs = detail.data?.config ? [detail.data.config as McpServerConfig] : parseMcpJson(String(detail.data.json));
      } catch (err) {
        console.error('[App] MCP JSON parse failed:', err);
        return;
      }
      Promise.all(configs.map(config => addMcpServer(config)))
        .then(() => listMcpServers())
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP add failed:', err));
    }
    if (detail.action === 'remove_mcp_server' && detail.data?.name) {
      removeMcpServer(detail.data.name)
        .then(() => listMcpServers())
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP remove failed:', err));
    }
    if (detail.action === 'reconnect_mcp_server' && detail.data?.name) {
      reconnectMcpServer(detail.data.name)
        .then(() => listMcpServers())
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP reconnect failed:', err));
    }
    if (detail.action === 'toggle_mcp_server' && detail.data?.name) {
      toggleMcpServer(detail.data.name, !!detail.data.enabled)
        .then(() => listMcpServers())
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP toggle failed:', err));
    }
    if (detail.action === 'refresh_mcp_tools') {
      refreshMcpTools()
        .then(() => listMcpServers())
        .then(servers => setMcpServers(servers))
        .catch(err => console.error('[App] MCP refresh failed:', err));
    }

    // ── Mod Unload Action ─────────────────────────────
    if (detail.action === 'unload_mod') {
      unloadMod().catch(err => console.error('[App] Mod unload failed:', err));
    }
    if (detail.action === 'refresh_mods') {
      listMods()
        .then(mods => setModList(mods))
        .catch(err => console.error('[App] Mod list failed:', err));
    }
    if (detail.action === 'load_mod' && detail.data?.id) {
      loadMod(detail.data.id)
        .then(() => listMods())
        .then(mods => setModList(mods))
        .catch(err => console.error('[App] Mod load failed:', err));
    }
    if (detail.action === 'import_mod_archive') {
      void (async () => {
        try {
          const { open } = await import('@tauri-apps/plugin-dialog');
          const selected = await open({
            multiple: false,
            filters: [{ name: 'Mod Archive', extensions: ['zip'] }],
          });
          if (!selected || typeof selected !== 'string') return;
          await installMod(selected);
          const mods = await listMods();
          setModList(mods);
        } catch (err) {
          console.error('[App] Mod import failed:', err);
        }
      })();
    }

    // ── Memory Actions ─────────────────────────────
    if (detail.action === 'list_memories' && detail.data?.characterId) {
      const { characterId, limit, offset } = detail.data;
      listMemories(characterId, limit || 50, offset || 0)
        .then(res => { setMemoryList(res.memories); setMemoryTotal(res.total); })
        .catch(err => console.error('[App] Memory list failed:', err));
    }
    if (detail.action === 'update_memory' && detail.data) {
      const { id, content, importance } = detail.data;
      updateMemory(id, content, importance)
        .then(() => {
          if (detail.data?.tier) return updateMemoryTier(id, detail.data.tier);
        })
        .catch(err => console.error('[App] Memory update failed:', err));
    }
    if (detail.action === 'delete_memory' && detail.data?.id !== undefined) {
      deleteMemory(detail.data.id)
        .catch(err => console.error('[App] Memory delete failed:', err));
    }

    // ── ImageGen Actions ───────────────────────────
    if (detail.action === 'test_sd_connection' && detail.data?.baseUrl) {
      testSdConnection(detail.data.baseUrl)
        .then(models => setSdModels(models))
        .catch(err => console.error('[App] SD connection test failed:', err));
    }
    if (detail.action === 'test_image_gen') {
      generateImage("A cute chibi anime character, white background, high quality", detail.data?.providerId)
        .then(result => console.log('[App] Image generation test completed:', result.image_url))
        .catch(err => console.error('[App] Image generation test failed:', err));
    }

    // ── Vision Actions ─────────────────────────────
    if (detail.action === 'capture_screen') {
      captureScreenNow()
        .then(url => setCapturedScreenUrl(url))
        .catch(err => console.error('[App] Screen capture failed:', err));
    }

    // ── Live2D Model Actions ──────────────────────
    if (detail.action === 'delete_live2d_model' && detail.data?.modelName) {
      deleteLive2dModel(detail.data.modelName)
        .then(() => listLive2dModels())
        .then(models => setAvailableModels(models))
        .catch(err => console.error('[App] Live2D delete failed:', err));
    }
    if (detail.action === 'refresh_live2d_models') {
      listLive2dModels()
        .then(models => setAvailableModels(models))
        .catch(err => console.error('[App] Live2D refresh failed:', err));
    }
    // Alias for mod convenience
    if (detail.action === 'refresh_models') {
      listLive2dModels()
        .then(models => setAvailableModels(models))
        .catch(err => console.error('[App] Live2D refresh failed:', err));
    }
    if (detail.action === 'set_custom_model') {
      const newPath = detail.data?.path ?? null;
      void updateActiveCharacterRuntime({ live2dModel: newPath }).catch(console.error);
    }
    if (detail.action === 'import_model') {
      import('@tauri-apps/plugin-dialog').then(({ open }) => {
        open({
          multiple: false,
          filters: [
            { name: 'Live2D Package (zip)', extensions: ['zip'] },
            { name: 'Live2D Model', extensions: ['model3.json'] },
          ]
        }).then(async (selected) => {
          if (selected && typeof selected === 'string') {
            if (selected.toLowerCase().endsWith('.zip')) {
              try {
                const modelPath = await importLive2dZip(selected);
                await updateActiveCharacterRuntime({ live2dModel: modelPath });
                const models = await listLive2dModels();
                setAvailableModels(models);
              } catch (e) { console.error('[App] import zip failed:', e); }
            } else {
              try {
                const modelPath = await importLive2dFolder(selected);
                await updateActiveCharacterRuntime({ live2dModel: modelPath });
                const models = await listLive2dModels();
                setAvailableModels(models);
              } catch (e) { console.error('[App] import folder failed:', e); }
            }
          }
        });
      });
    }
    if (detail.action === 'export_live2d_model') {
      void (async () => {
        try {
          const modelPath = detail.data?.path || customModelPath;
          const selectedModel = availableModels.find(m => m.path === modelPath);
          if (!modelPath || !selectedModel) return;
          const { save } = await import('@tauri-apps/plugin-dialog');
          const filePath = await save({
            defaultPath: `${selectedModel.name}.zip`,
            filters: [{ name: 'Live2D Package', extensions: ['zip'] }],
          });
          if (!filePath) return;
          await exportLive2dModel(modelPath, filePath);
        } catch (err) {
          console.error('[App] Live2D export failed:', err);
        }
      })();
    }
    if (detail.action === 'rename_live2d_model' && detail.data?.path && detail.data?.newName) {
      const modelPath = detail.data.path;
      const newName = detail.data.newName;
      renameLive2dModel(modelPath, newName)
        .then(async nextPath => {
          // A renamed model only belongs to the active character when its
          // persisted selection still points at the old path. Keep unrelated
          // character selections untouched.
          if (readStringSetting(APP_SETTING_KEYS.customModelPath, "") === modelPath) {
            await updateActiveCharacterRuntime({ live2dModel: nextPath });
          }
          return listLive2dModels();
        })
        .then(models => setAvailableModels(models))
        .catch(err => console.error('[App] Live2D rename failed:', err));
    }

    // ── Language Actions ───────────────────────────
    if (detail.action === 'set_user_language' && detail.data?.language) {
      setUserLanguageState(detail.data.language);
      writeStringSetting(APP_SETTING_KEYS.userLanguage, detail.data.language);
      setUserLanguage(detail.data.language).catch(console.error);
    }

    // ── User Profile Actions ───────────────────────
    if (detail.action === 'set_user_name' && detail.data?.name) {
      writeStringSetting(APP_SETTING_KEYS.userName, detail.data.name);
      setUserName(detail.data.name).catch(console.error);
    }
    if (detail.action === 'set_user_persona' && detail.data?.persona) {
      writeStringSetting(APP_SETTING_KEYS.userPersona, detail.data.persona);
      setUserPersona(detail.data.persona).catch(console.error);
    }

    // ── Character Actions ─────────────────────────
    if (detail.action === 'list_characters') {
      listCharacters().then((all) => {
        setCharacters(all);
      }).catch(console.error);
    }
    if (detail.action === 'select_character' && detail.data?.id != null) {
      const characterId = String(detail.data.id);
      void (async () => {
        const all = await listCharacters();
        if (!all.some((character) => character.id === characterId)) return;
        await characterActivation.activateCharacter(characterId);
        setCharacters(all);
      })().catch(console.error);
    }
    if (detail.action === 'create_character') {
      void (async () => {
        const id = crypto.randomUUID();
        const now = Date.now();
        await createCharacter({ id, name: 'New Character', persona: '', user_nickname: 'User', source_format: 'manual', created_at: now, updated_at: now });
        const all = await listCharacters();
        setCharacters(all);
        if (all.some((character) => character.id === id)) {
          await characterActivation.activateCharacter(id);
        }
      })().catch(console.error);
    }
    if (detail.action === 'import_character') {
      // Trigger file input from host context
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.json,.png';
      input.onchange = async (e) => {
        const file = (e.target as HTMLInputElement).files?.[0];
        if (!file) return;
        try {
          const { parseCharacterCard } = await import('./lib/character-card-parser');
          const profile = await parseCharacterCard(file);
          const id = crypto.randomUUID();
          const now = Date.now();
          const avatarPath = profile.avatar_bytes
            ? `character-instance-resource://${id}/avatar.png`
            : profile.avatar_path;
          const record = {
            id,
            name: profile.name,
            description: profile.description,
            persona: profile.persona,
            avatar_path: avatarPath,
            greeting: profile.greeting,
            example_dialogue: profile.example_dialogue,
            user_nickname: profile.user_nickname,
            source_format: profile.source_format,
            created_at: now,
            updated_at: now,
          };
          if (profile.avatar_bytes) {
            await createCharacterWithAvatar(record, profile.avatar_bytes);
          } else {
            await createCharacter(record);
          }
          const all = await listCharacters();
          setCharacters(all);
          if (all.some((character) => character.id === id)) {
            await characterActivation.activateCharacter(id);
          }
        } catch (err) {
          console.error('[App] import character failed:', err);
        }
      };
      input.click();
    }
  };

  useEffect(() => {
    document.addEventListener('kokoro:mod-action', handleModAction);
    return () => document.removeEventListener('kokoro:mod-action', handleModAction);
  });

  const characterCatalogActions: CharacterCatalogActionDependencies = {
    activateCharacter: async (catalogId) => {
      let targetId = catalogId;
      let template = characterTemplates.find((candidate) => candidate.id === catalogId) ?? null;
      if (catalogId.startsWith("template:")) {
        const templateId = catalogId.slice("template:".length);
        template = characterTemplates.find((candidate) => candidate.id === templateId) ?? null;
        if (template === null) {
          throw new Error(`character template '${templateId}' is unavailable`);
        }
        const existing = characters.find((candidate) => candidate.template_id === templateId);
        if (existing) {
          targetId = existing.id;
        } else {
          targetId = characters.some((candidate) => candidate.id === templateId)
            ? `${templateId}-${crypto.randomUUID()}`
            : templateId;
          const now = Date.now();
          await instantiateCharacterTemplate({
            template_id: template.id,
            template_version: template.version,
            instance_id: targetId,
            user_nickname: readStringSetting(APP_SETTING_KEYS.userName, "") || "User",
            created_at: now,
            updated_at: now,
          });
        }
      } else if (template === null) {
        const instance = characters.find((candidate) => candidate.id === targetId);
        template = instance?.template_id
          ? characterTemplates.find((candidate) => candidate.id === instance.template_id) ?? null
          : null;
      }

      const activation = await characterActivation.activateCharacter(targetId);
      setCharacters(await listCharacters());

      const raw = activation.recommendations;
      const recommendations: CharacterCapabilityRecommendations = {
        vision: raw.vision === true,
        memory: raw.memory === true,
        mcpServers: raw.mcp_servers,
        botPlatforms: raw.bot_platforms,
      };
      return recommendations.vision
        || recommendations.memory
        || recommendations.mcpServers.length > 0
        || recommendations.botPlatforms.length > 0
        ? recommendations
        : null;
    },
    importCharacter: async () => {
      document.dispatchEvent(new CustomEvent("kokoro:mod-action", {
        detail: { action: "import_character" },
      }));
    },
    editCharacter: async (characterId) => {
      selectCharacterForEditing(characters, characterId);
      setCharacterToEditId(characterId);
      setActiveSettingsTab("persona");
      setSettingsOpen(true);
    },
    duplicateCharacter: async (characterId) => {
      const source = characters.find((candidate) => candidate.id === characterId);
      if (!source) throw new Error(`character '${characterId}' is unavailable`);
      const now = Date.now();
      await duplicateCharacter({
        source_id: characterId,
        new_id: crypto.randomUUID(),
        new_name: `${source.name} ${t("characterCatalog.copySuffix")}`,
        created_at: now,
        updated_at: now,
      });
      setCharacters(await listCharacters());
    },
    restoreCharacterDefaults: async (characterId) => {
      await restoreCharacterDefaults({ id: characterId, updated_at: Date.now() });
      setCharacters(await listCharacters());
      if (characterId === activeCharacterId) {
        await characterActivation.activateCharacter(characterId);
      }
    },
    resolveTemplateConflict: async (characterId) => {
      const character = characters.find((candidate) => candidate.id === characterId);
      if (!character?.template_id) throw new Error("character has no template origin");
      const availableTemplates = characterTemplates
        .filter((candidate) => candidate.id === character.template_id)
        .sort((left, right) => left.version.localeCompare(right.version));
      const available = availableTemplates[availableTemplates.length - 1];
      if (!available) throw new Error(`template '${character.template_id}' is unavailable`);
      const preview = await reconcileCharacterTemplate({
        instance_id: characterId,
        template_version: available.version,
      });
      await applyCharacterTemplateReconciliation({
        instance_id: characterId,
        expected_current_template_version: preview.current_template_version,
        expected_new_template_version: preview.available_template_version,
        selected: preview.merged,
        updated_at: Date.now(),
      });
      setCharacters(await listCharacters());
      if (characterId === activeCharacterId) {
        await characterActivation.activateCharacter(characterId);
      }
    },
  };

  const enableRecommendedCapabilities = async (
    recommendations: Readonly<CharacterCapabilityRecommendations>,
  ): Promise<void> => {
    await applyCharacterCapabilityRecommendations({
      recommendations,
      dependencies: {
        enableVisionBackend: async () => {
          const currentVision = visionConfig ?? await getVisionConfig();
          const nextVision = { ...currentVision, vlm_enabled: true };
          await saveVisionConfig(nextVision);
          setVisionConfig(nextVision);
        },
        cacheVisionEnabled: (enabled) => writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, enabled),
        dispatchVisionChanged: () => dispatchRuntimeSettingsChanged("vision"),
        setMemoryEnabled,
        listMcpServerNames: () => mcpServers.map((server) => server.name),
        toggleMcpServer,
        refreshMcpServers: async () => setMcpServers(await listMcpServers()),
        enableBotPlatforms: async (platforms) => {
          const current = botConfig ?? await getBotConfig();
          let next = current;
          for (const platform of platforms) {
            switch (platform) {
              case "telegram": next = { ...next, telegram: { ...next.telegram, enabled: true } }; break;
              case "qq": next = { ...next, qq: { ...next.qq, enabled: true } }; break;
              case "discord": next = { ...next, discord: { ...next.discord, enabled: true } }; break;
              case "line": next = { ...next, line: { ...next.line, enabled: true } }; break;
              case "webhook": next = { ...next, webhook: { ...next.webhook, enabled: true } }; break;
            }
          }
          const saved = await saveBotConfig(next);
          setBotConfig(saved);
        },
      },
    });
  };

  // Determine active background based on mode
  let activeBackgroundUrl = characterBackground ?? bgSlideshow.currentUrl;

  if (bgSlideshow.config.mode === "generated" && generatedImage) {
    activeBackgroundUrl = generatedImage;
  } else if (bgSlideshow.config.mode === "static" && characterBackground === null) {
    // For static, we might just use the first image in the list, or the current selected one?
    // Since 'static' usually implies 'user selected one image', but we don't have a specific UI for that yet
    // beyond the 'slideshow' list. 
    // Let's assume 'static' just means 'no rotation' which is handled by background hook if we set interval to 0?
    // Actually, useBackgroundSlideshow doesn't fully support 'static' mode in the hook logic itself cleanly
    // aside from 'slideshow' mode.
    // But based on our update, 'useBackgroundSlideshow' now has 'mode' in config.
    // If config.mode is 'static', existing hook might just pause?
    // Let's just use currentUrl from hook, assuming hook handles 'static' logic (or we treat it as slideshow paused)
    activeBackgroundUrl = bgSlideshow.currentUrl;
  }

  // If in 'generated' mode but no generated image yet, fallback to current slideshow image (or blank?)
  // Better to fallback to slideshow image so it's not empty.
  // Code above does this: default is bgSlideshow.currentUrl, override if generated & mode is generated.

  const renderedBackgroundUrl = mapBackgroundAssetUrl(activeBackgroundUrl, convertFileSrc);

  if (!runtimeReady) {
    return <div className="h-screen w-screen bg-black" aria-label="Loading character runtime" />;
  }

  return (
    <ThemeProvider initialTheme={defaultTheme}>
      {/* Background image rendered inside LayoutRenderer, behind Live2D */}
      <LayoutRenderer
        config={layout}
        transparent={!!renderedBackgroundUrl}
        backgroundLayer={
          <BackgroundLayer
            imageUrl={renderedBackgroundUrl}
            blur={bgSlideshow.config.blur}
            blurAmount={bgSlideshow.config.blurAmount}
          />
        }
        overlayLayer={
          <div className="absolute right-[92px] top-[52px]">
            <CharacterCatalog
              characters={characters}
              templates={characterTemplates}
              activeCharacterId={activeCharacterId}
              actions={characterCatalogActions}
              resolveAvatarUrl={(path) => mapCharacterAvatarUrl(path, convertFileSrc)}
              onRecommendations={(characterName, recommendations) => {
                setRecommendedCapabilities({ characterName, recommendations });
              }}
            />
          </div>
        }
      />

      <CharacterRecommendationDialog
        open={recommendedCapabilities !== null}
        characterName={recommendedCapabilities?.characterName ?? ""}
        recommendations={recommendedCapabilities?.recommendations ?? null}
        onConfirm={enableRecommendedCapabilities}
        onDismiss={() => setRecommendedCapabilities(null)}
      />

      {/* Floating settings gear — top-right corner */}
      <WindowTitleBar />

      <motion.button
        initial={false}
        whileHover="hover"
        whileTap={{ scale: 0.97 }}
        transition={{ type: "spring", stiffness: 360, damping: 26 }}
        onClick={() => setSettingsOpen(true)}
        data-onboarding-id="settings-button"
        className="fixed top-[52px] right-[35px] z-50 p-3 rounded-full bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)] border border-[var(--color-border)] text-[var(--color-text-secondary)] shadow-lg transition-[color,border-color,box-shadow] duration-200 ease-out hover:border-[var(--color-border-accent)] hover:text-[var(--color-accent)] hover:shadow-[0_0_18px_rgba(0,240,255,0.18)]"
        aria-label="Open settings"
      >
        <motion.span
          variants={{ hover: { rotate: 18, scale: 1.04 } }}
          transition={{ type: "spring", stiffness: 420, damping: 24 }}
          className="flex items-center justify-center"
        >
          <Settings size={20} strokeWidth={1.5} />
        </motion.span>
      </motion.button>

      {/* SettingsPanel is retrieved from registry to allow mod overrides */}
      {(() => {
        const SettingsComponent = registry.get("SettingsPanel") || SettingsPanel;
        const isMod = registry.isModComponent("SettingsPanel");
        const component = (
          <SettingsComponent
            isOpen={settingsOpen}
            onClose={() => setSettingsOpen(false)}
            activeTab={activeSettingsTab}
            onActiveTabChange={setActiveSettingsTab}
            backgroundControls={{
              config: bgSlideshow.config,
              setConfig: bgSlideshow.setConfig,
              images: bgSlideshow.images,
              importFiles: bgSlideshow.importFiles,
              removeImage: bgSlideshow.removeImage,
              clearImages: bgSlideshow.clearImages,
              imageCount: bgSlideshow.imageCount,
            }}
            displayMode={displayMode}
            onDisplayModeChange={handleDisplayModeChange}
            customModelPath={customModelPath}
            onCustomModelChange={handleCustomModelChange}
            gazeTracking={gazeTracking}
            onGazeTrackingChange={handleGazeTrackingChange}
            renderFps={renderFps}
            onRenderFpsChange={handleRenderFpsChange}
            // External state for Mod
            availableModels={availableModels}
            persona={persona}
            responseLanguage={responseLanguage}
            ttsConfig={ttsConfig}
            llmConfig={llmConfig}
            onLlmConfigSaved={setLlmConfig}
            sttConfig={sttConfig}
            visionConfig={visionConfig}
            onVisionConfigChange={setVisionConfig}
            imageGenConfig={imageGenConfig}
            telegramConfig={telegramConfig}
            botConfig={botConfig}
            botStatus={botStatus}
            autoBackupConfig={autoBackupConfig}
            backupStatus={backupStatus}
            mcpServers={mcpServers}
            modList={modList}
            ttsProviders={ttsProviders}
            ttsVoices={ttsVoices}
            // Dynamic State
            fetchedLlmModels={fetchedLlmModels}
            scannedTtsModels={scannedTtsModels}
            voiceInterrupt={voiceInterrupt}
            onVoiceInterruptChange={(v: boolean) => setVoiceInterrupt(v)}
            // New: Full Parity Props
            memoryList={memoryList}
            memoryTotal={memoryTotal}
            sdModels={sdModels}
            capturedScreenUrl={capturedScreenUrl}
            userLanguage={userLanguage}
            activeCharacterId={readStringSetting(APP_SETTING_KEYS.activeCharacterId, "default") || "default"}
            characterToEditId={characterToEditId}
            onActivateCharacter={async (characterId: string) => {
              await characterActivation.activateCharacter(characterId);
            }}
            onCharacterRuntimeChange={updateActiveCharacterRuntime}
            characters={characters}
            // User Profile (from localStorage)
            userName={readStringSetting(APP_SETTING_KEYS.userName, "")}
            userPersona={readStringSetting(APP_SETTING_KEYS.userPersona, "")}
            proactiveEnabled={proactiveEnabled}
            initialTelegramStatus={telegramStatus}
          />
        );

        if (isMod) {
          if (!settingsOpen) return null;
          return (
            <div style={{
              position: "fixed",
              inset: 0,
              zIndex: 100,
              pointerEvents: "auto",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}>
              {component}
            </div>
          );
        }

        return component;
      })()}

      {onboardingDraft.dismissed && !onboardingOpen && (
        <button
          type="button"
          onClick={() => setOnboardingOpen(true)}
          className="pointer-events-auto fixed bottom-5 right-5 z-[130] rounded-full border border-[var(--color-border-accent)] bg-[var(--color-bg-surface)]/95 px-4 py-2 text-xs font-semibold uppercase tracking-[0.14em] text-[var(--color-accent)] shadow-lg backdrop-blur-xl transition hover:bg-[var(--color-accent)]/10"
        >
          {t("onboarding.workflow.resume", { defaultValue: "Resume setup" })}
        </button>
      )}
      {onboardingOpen && (
        <OnboardingOverlay
          draft={onboardingDraft}
          characters={characters.map<OnboardingCharacter>((character) => ({
            id: character.id,
            name: character.name,
            description: character.description?.trim() || character.persona,
            avatarPath: character.avatar_path
              ? mapCharacterAvatarUrl(character.avatar_path, convertFileSrc)
              : null,
          }))}
          providerSetup={onboardingProviderSetup}
          connectionResult={onboardingConnectionResult}
          isTestingConnection={onboardingTestingConnection}
          isSavingProvider={onboardingSavingProvider}
          providerError={onboardingProviderError}
          characterError={onboardingCharacterError}
          isSubmittingChat={onboardingSubmittingChat}
          onEvent={dispatchOnboardingEvent}
          onLanguageSelect={previewOnboardingLanguage}
          onCharacterSelect={handleOnboardingCharacterSelect}
          onProviderChange={setOnboardingProviderSetup}
          onProviderSave={handleOnboardingProviderSave}
          onTestConnection={() => void handleOnboardingConnectionTest()}
          onChatSubmit={handleOnboardingChatSubmit}
          onFirstReplySucceeded={handleOnboardingFirstReplySucceeded}
          onDismiss={() => closeOnboarding("dismissed")}
          onResume={() => setOnboardingOpen(true)}
        />
      )}

      <MemoryModelDownloadDialog
        open={memoryModelDialogOpen}
        status={memoryModelStatus}
        progress={memoryModelProgress}
        downloading={memoryModelDownloading}
        error={memoryModelError}
        onClose={() => setMemoryModelDialogOpen(false)}
        onDownload={() => {
          if (memoryModelStatus?.installed) {
            setMemoryModelDialogOpen(false);
            return;
          }
          void startMemoryModelDownload();
        }}
      />

      <QQAuthorizationDialog
        request={currentQQAuthorization}
        busy={qqAuthorizationBusy}
        error={qqAuthorizationError}
        onAllow={() => { void respondToCurrentQQAuthorization(true); }}
        onReject={() => { void respondToCurrentQQAuthorization(false); }}
      />

      {/* Camera watcher — lives at app root so it persists when settings panel closes */}
      <CameraWatcher
        enabled={visionConfig?.camera_enabled ?? false}
        deviceId={visionConfig?.camera_device_id ?? undefined}
      />
    </ThemeProvider>
  );
}

export default App;
