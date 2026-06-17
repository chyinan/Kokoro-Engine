import { useState, useEffect, useMemo, useSyncExternalStore, useCallback, useRef } from "react";
import { motion } from "framer-motion";
import { Settings } from "lucide-react";
import { emit } from "@tauri-apps/api/event";
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
import WindowTitleBar from "./ui/widgets/WindowTitleBar";
import OnboardingOverlay, {
  type OnboardingLanguageCode,
  type OnboardingStep,
} from "./ui/widgets/OnboardingOverlay";
import MemoryModelDownloadDialog from "./ui/widgets/MemoryModelDownloadDialog";
import { useBackgroundSlideshow } from "./ui/hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "./features/live2d/Live2DViewer";
import { live2dUrl } from "./lib/utils";
import { MEMORY_MODEL_DIALOG_EVENT } from "./lib/memory-model-gate";
import { characterDb } from "./lib/db";
import {
  APP_SETTING_KEYS,
  dispatchRuntimeSettingsChanged,
  readBooleanSetting,
  readNumberSetting,
  readOptionalStringSetting,
  readStringSetting,
  removeSetting,
  writeBooleanSetting,
  writeStringSetting,
} from "./lib/app-settings";
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
  gazeTracking: boolean;
  renderFps: number;
  chatPanelWidth: number;
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
              },
              style: { pointerEvents: "auto", margin: "48px 0 20px 20px", padding: "0" },
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
  setPersona,
  setUserName,
  setUserPersona,
  setResponseLanguage,
  getJailbreakPrompt,
  setJailbreakPrompt,
  getProactiveEnabled,
  getUserProfileSettings,
  getMemoryEmbeddingModelStatus,
  // Config Getters
  getLlmConfig,
  getImageGenConfig,
  getTelegramConfig,
  getTelegramStatus,
  getBotConfig,
  getBotStatus,
  getAutoBackupConfig,
  listCharacters,
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
  type MemoryEmbeddingModelStatus,
  type MemoryEmbeddingModelDownloadProgress,
  getKokoroErrorMessage,
  onMemoryEmbeddingModelProgress,
} from "./lib/kokoro-bridge";
import type { ThemeConfig } from "./ui/layout/types";
import { modMessageBus } from "./ui/mods/ModMessageBus";
import { CameraWatcher } from "./features/camera/CameraWatcher";

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

const ONBOARDING_LANGUAGE_NAMES: Record<OnboardingLanguageCode, string> = {
  en: "English",
  zh: "中文",
  "zh-TW": "繁體中文",
  ja: "日本語",
  ko: "한국어",
  ru: "Русский",
};

function normalizeOnboardingLanguageCode(language: string | null | undefined): OnboardingLanguageCode {
  const normalized = language?.trim().toLowerCase();
  if (
    normalized?.startsWith("zh-tw") ||
    normalized?.startsWith("zh-hant") ||
    normalized?.startsWith("zh-hk") ||
    normalized?.startsWith("zh-mo")
  ) {
    return "zh-TW";
  }

  const base = normalized?.split("-")[0];
  switch (base) {
    case "en":
    case "zh":
    case "ja":
    case "ko":
    case "ru":
      return base;
    default:
      return "zh";
  }
}

function App() {
  const { i18n } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [activeSettingsTab, setActiveSettingsTab] = useState<SettingsTabId>(() => {
    const saved = readStringSetting(APP_SETTING_KEYS.settingsActiveTab, "");
    return normalizeSettingsTabId(saved);
  });
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(() =>
    localStorage.getItem(ONBOARDING_STATUS_KEY) ? null : "language"
  );
  const [onboardingLanguage, setOnboardingLanguage] = useState<OnboardingLanguageCode>(() =>
    normalizeOnboardingLanguageCode(
      readStringSetting(APP_SETTING_KEYS.appLanguage, "")
      || (typeof navigator !== "undefined" ? navigator.language : "zh")
    )
  );
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

  const modelUrl = useMemo(() => {
    if (customModelPath) {
      return live2dUrl(customModelPath);
    }
    return live2dUrl(BUILTIN_LIVE2D_MODEL_PATH);
  }, [customModelPath]);

  useEffect(() => {
    setActiveLive2dModel(activeLive2dModelPath).catch((err) => {
      console.error("[App] Failed to sync active Live2D model:", err);
    });
    emit("live2d-model-selection-updated", {
      modelPath: activeLive2dModelPath,
      customModelPath,
      modelUrl,
    }).catch((err) => {
      console.error("[App] Failed to broadcast Live2D model selection:", err);
    });
  }, [activeLive2dModelPath, customModelPath, modelUrl]);

  const layout = useMemo(
    () => createLayout({
      mode: displayMode,
      modelUrl,
      modelPath: activeLive2dModelPath,
      gazeTracking,
      renderFps,
      chatPanelWidth,
      onChatPanelWidthPreview: handleChatPanelWidthPreview,
      onChatPanelWidthChange: handleChatPanelWidthChange,
    }),
    [displayMode, modelUrl, activeLive2dModelPath, gazeTracking, renderFps, chatPanelWidth, handleChatPanelWidthPreview, handleChatPanelWidthChange]
  );

  const handleDisplayModeChange = (mode: Live2DDisplayMode) => {
    setDisplayMode(mode);
    writeStringSetting(APP_SETTING_KEYS.displayMode, mode);
  };

  const handleCustomModelChange = (path: string | null) => {
    setCustomModelPath(path);
    if (path) {
      writeStringSetting(APP_SETTING_KEYS.customModelPath, path);
    } else {
      removeSetting(APP_SETTING_KEYS.customModelPath);
    }
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
    setOnboardingLanguage(language);
    i18n.changeLanguage(language);
    writeStringSetting(APP_SETTING_KEYS.appLanguage, language);
    writeStringSetting(APP_SETTING_KEYS.responseLanguage, label);
    writeStringSetting(APP_SETTING_KEYS.userLanguage, label);
    setResponseLanguageState(label);
    setUserLanguageState(label);
    setResponseLanguage(label).catch(console.error);
    setUserLanguage(label).catch(console.error);
  };

  const previewOnboardingLanguage = (language: OnboardingLanguageCode) => {
    applyOnboardingLanguage(language);
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
    localStorage.setItem(ONBOARDING_STATUS_KEY, status);
    setOnboardingStep(null);
  };

  const advanceOnboarding = () => {
    switch (onboardingStep) {
      case "language":
        setOnboardingStep("open-settings");
        break;
      case "api":
        setOnboardingStep("persona");
        break;
      case "persona":
        setOnboardingStep("return-home");
        break;
      case "chat":
        closeOnboarding("completed");
        break;
      default:
        break;
    }
  };

  useEffect(() => {
    if (onboardingStep === "open-settings" && settingsOpen) {
      setOnboardingStep("api");
    }
    if (onboardingStep === "return-home" && !settingsOpen) {
      setOnboardingStep("chat");
    }
  }, [onboardingStep, settingsOpen]);

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
    if (onboardingStep === "chat" && memoryModelStatus && !memoryModelStatus.installed) {
      void openMemoryModelDialog();
    }
  }, [memoryModelStatus, onboardingStep, openMemoryModelDialog]);

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

    // Sync language settings to backend on startup
    const savedResponseLang = readStringSetting(APP_SETTING_KEYS.responseLanguage, "");
    const savedUserLang = readStringSetting(APP_SETTING_KEYS.userLanguage, "");
    if (savedResponseLang) setResponseLanguage(savedResponseLang).catch(console.error);
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

    // Sync the active character's persona to the backend on startup
    userProfileHydration.finally(() => {
      import("./ui/widgets/CharacterManager").then(async ({ composeSystemPrompt }) => {
      const { listCharacters, setPersona, setActiveCharacterId } = await import("./lib/kokoro-bridge");
      try {
        const all = await listCharacters();
        setCharacters(all);
        const savedId = readStringSetting(APP_SETTING_KEYS.activeCharacterId, "");
        const char = savedId ? all.find(c => c.id === savedId) ?? all[0] : all[0];
        if (char) {
          const prompt = composeSystemPrompt(char);
          writeStringSetting(APP_SETTING_KEYS.persona, prompt);
          setPersonaState(prompt);
          await setPersona(prompt);
          await setActiveCharacterId(char.id);
          console.log("[App] Synced persona on startup:", char.name);
        }
      } catch (e) {
        console.error("[App] Failed to sync persona on startup:", e);
      }
    });
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
    const unlistenModChatDelta = onChatTurnDelta(({ turn_id, delta }) => {
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

    const unlistenModChatDone = onChatTurnFinish(({ turn_id, status }) => {
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

  const arrayBufferToBase64 = (buffer: ArrayBuffer): string => {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  };

  const buildCharactersBackupJson = async (): Promise<string> => {
    const sqliteChars = await listCharacters();
    const idbChars = await characterDb.getAll();
    const idbByStableId = new Map(idbChars.filter(c => c.stableId).map(c => [c.stableId, c]));
    const userName = readStringSetting(APP_SETTING_KEYS.userName, "User");
    const userPersona = readStringSetting(APP_SETTING_KEYS.userPersona, "");
    const userLanguage = readOptionalStringSetting(APP_SETTING_KEYS.userLanguage);
    const responseLanguage = readOptionalStringSetting(APP_SETTING_KEYS.responseLanguage);
    const voiceInterrupt = readOptionalStringSetting(APP_SETTING_KEYS.voiceInterrupt);

    await setUserName(userName);
    await setUserPersona(userPersona);

    const characters = await Promise.all(sqliteChars.map(async (char) => {
      const idbChar = idbByStableId.get(char.id);
      if (idbChar?.avatarBlob) {
        return { ...char, stableId: char.id, avatarB64: arrayBufferToBase64(await idbChar.avatarBlob.arrayBuffer()) };
      }
      return { ...char, stableId: char.id };
    }));

    return JSON.stringify({
      characters,
      activeCharacterId: readOptionalStringSetting(APP_SETTING_KEYS.activeCharacterId),
      userName,
      userPersona,
      userLanguage,
      responseLanguage,
      voiceInterrupt,
    });
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
      const result = await exportData(filePath, await buildCharactersBackupJson());
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
        if (payload.responseLanguage != null) writeStringSetting(APP_SETTING_KEYS.responseLanguage, payload.responseLanguage);
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
        if (targetCharacterId) writeStringSetting(APP_SETTING_KEYS.activeCharacterId, targetCharacterId);
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
      if (target) handleCustomModelChange(target.path);
    }
    if (detail.action === 'set_persona' && detail.data?.persona) {
      setPersonaState(detail.data.persona);
      writeStringSetting(APP_SETTING_KEYS.persona, detail.data.persona);
      setPersona(detail.data.persona).catch(console.error);
    }
    if (detail.action === 'set_language' && detail.data?.language) {
      setResponseLanguageState(detail.data.language);
      writeStringSetting(APP_SETTING_KEYS.responseLanguage, detail.data.language);
      setResponseLanguage(detail.data.language).catch(console.error);
    }
    // Full Config Save Handlers
    if (detail.action === 'save_llm_config' && detail.data?.config) {
      setLlmConfig(detail.data.config);
      saveLlmConfig(detail.data.config).catch(console.error);
    }
    if (detail.action === 'save_tts_config' && detail.data?.config) {
      setTtsConfig(detail.data.config);
      saveTtsConfig(detail.data.config).then(() => {
        // Refresh providers & voices after save
        Promise.all([listTtsProviders(), listTtsVoices()]).then(([p, v]) => {
          setTtsProviders(p);
          setTtsVoices(v);
        }).catch(err => console.error("[App] Failed to refresh TTS lists:", err));
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
        .then(() => Promise.all([getBotStatus(), getTelegramConfig(), getTelegramStatus()]))
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
      setProactiveEnabledState(enabled);
      writeBooleanSetting(APP_SETTING_KEYS.proactiveEnabled, enabled);
      import("./lib/kokoro-bridge").then(({ setProactiveEnabled }) => {
        setProactiveEnabled(enabled).catch(console.error);
      });
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
      writeBooleanSetting(APP_SETTING_KEYS.ttsEnabled, !!detail.data?.enabled);
    }
    if (detail.action === 'set_tts_speed' && detail.data?.speed !== undefined) {
      writeStringSetting(APP_SETTING_KEYS.ttsSpeed, String(detail.data.speed));
    }
    if (detail.action === 'set_tts_pitch' && detail.data?.pitch !== undefined) {
      writeStringSetting(APP_SETTING_KEYS.ttsPitch, String(detail.data.pitch));
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
      if (speed !== undefined) writeStringSetting(APP_SETTING_KEYS.ttsSpeed, String(speed));
      if (pitch !== undefined) writeStringSetting(APP_SETTING_KEYS.ttsPitch, String(pitch));
      if (voice !== undefined) writeStringSetting(APP_SETTING_KEYS.ttsVoice, voice);
      if (provider !== undefined) writeStringSetting(APP_SETTING_KEYS.ttsProvider, provider);
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
      handleCustomModelChange(newPath);
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
                setCustomModelPath(modelPath);
                writeStringSetting(APP_SETTING_KEYS.customModelPath, modelPath);
                const models = await listLive2dModels();
                setAvailableModels(models);
              } catch (e) { console.error('[App] import zip failed:', e); }
            } else {
              try {
                const modelPath = await importLive2dFolder(selected);
                setCustomModelPath(modelPath);
                writeStringSetting(APP_SETTING_KEYS.customModelPath, modelPath);
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
        .then(nextPath => {
          if (customModelPath === modelPath) handleCustomModelChange(nextPath);
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
      import('./lib/kokoro-bridge').then(async ({ listCharacters }) => {
        const all = await listCharacters();
        setCharacters(all);
      }).catch(console.error);
    }
    if (detail.action === 'select_character' && detail.data?.id != null) {
      const characterId = detail.data.id;
      import('./ui/widgets/CharacterManager').then(async ({ composeSystemPrompt }) => {
        const { listCharacters, setActiveCharacterId, setCharacterName } = await import('./lib/kokoro-bridge');
        const all = await listCharacters();
        const char = all.find(c => c.id === characterId);
        if (char) {
          writeStringSetting(APP_SETTING_KEYS.activeCharacterId, char.id);
          const prompt = composeSystemPrompt(char);
          setPersonaState(prompt);
          setPersona(prompt).catch(console.error);
          setCharacterName(char.name).catch(console.error);
          setActiveCharacterId(char.id).catch(console.error);
          setCharacters(all);
        }
      }).catch(console.error);
    }
    if (detail.action === 'create_character') {
      import('./lib/kokoro-bridge').then(async ({ createCharacter, listCharacters, setActiveCharacterId, setCharacterName }) => {
        const id = crypto.randomUUID();
        const now = Date.now();
        await createCharacter({ id, name: 'New Character', persona: '', user_nickname: 'User', source_format: 'manual', created_at: now, updated_at: now });
        const all = await listCharacters();
        setCharacters(all);
        const newChar = all.find(c => c.id === id);
        if (newChar) {
          writeStringSetting(APP_SETTING_KEYS.activeCharacterId, newChar.id);
          const { composeSystemPrompt } = await import('./ui/widgets/CharacterManager');
          const prompt = composeSystemPrompt(newChar);
          setPersonaState(prompt);
          setPersona(prompt).catch(console.error);
          setCharacterName(newChar.name).catch(console.error);
          setActiveCharacterId(newChar.id).catch(console.error);
        }
      }).catch(console.error);
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
          const { createCharacter, listCharacters, setActiveCharacterId, setCharacterName } = await import('./lib/kokoro-bridge');
          const profile = await parseCharacterCard(file);
          const id = crypto.randomUUID();
          const now = Date.now();
          await createCharacter({ id, ...profile, created_at: now, updated_at: now });
          const all = await listCharacters();
          setCharacters(all);
          const char = all.find(c => c.id === id);
          if (char) {
            writeStringSetting(APP_SETTING_KEYS.activeCharacterId, char.id);
            const { composeSystemPrompt } = await import('./ui/widgets/CharacterManager');
            const prompt = composeSystemPrompt(char);
            setPersonaState(prompt);
            setPersona(prompt).catch(console.error);
            setCharacterName(char.name).catch(console.error);
            setActiveCharacterId(char.id).catch(console.error);
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

  // Determine active background based on mode
  let activeBackgroundUrl = bgSlideshow.currentUrl;

  if (bgSlideshow.config.mode === "generated" && generatedImage) {
    activeBackgroundUrl = generatedImage;
  } else if (bgSlideshow.config.mode === "static") {
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

  return (
    <ThemeProvider initialTheme={defaultTheme}>
      {/* Background image rendered inside LayoutRenderer, behind Live2D */}
      <LayoutRenderer
        config={layout}
        transparent={!!activeBackgroundUrl}
        backgroundLayer={
          <BackgroundLayer
            imageUrl={activeBackgroundUrl}
            blur={bgSlideshow.config.blur}
            blurAmount={bgSlideshow.config.blurAmount}
          />
        }
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

      <OnboardingOverlay
        step={onboardingStep}
        selectedLanguage={onboardingLanguage}
        settingsOpen={settingsOpen}
        activeSettingsTab={activeSettingsTab}
        onLanguageSelect={previewOnboardingLanguage}
        onAdvance={advanceOnboarding}
        onDismiss={() => closeOnboarding("dismissed")}
      />

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

      {/* Camera watcher — lives at app root so it persists when settings panel closes */}
      <CameraWatcher
        enabled={visionConfig?.camera_enabled ?? false}
        deviceId={visionConfig?.camera_device_id ?? undefined}
      />
    </ThemeProvider>
  );
}

export default App;
