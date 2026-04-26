import { useState, useEffect, useMemo, useSyncExternalStore } from "react";
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
import SettingsPanel, { type SettingsTabId } from "./ui/widgets/SettingsPanel";
import BackgroundLayer from "./ui/widgets/BackgroundLayer";
import OnboardingOverlay, {
  type OnboardingLanguageCode,
  type OnboardingStep,
} from "./ui/widgets/OnboardingOverlay";
import { useBackgroundSlideshow } from "./ui/hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "./features/live2d/Live2DViewer";
import { live2dUrl } from "./lib/utils";

// Register components synchronously before first render
registerCoreComponents();

// Build layout config as a function of displayMode
function createLayout(displayMode: { mode: Live2DDisplayMode; modelUrl: string; modelPath: string | null; gazeTracking: boolean; renderFps: number }): LayoutConfig {
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
            modelUrl: displayMode.modelUrl,
            modelPath: displayMode.modelPath,
            displayMode: displayMode.mode,
            gazeTracking: displayMode.gazeTracking,
            maxFps: displayMode.renderFps,
          }
        },
        {
          id: "ui-grid",
          type: "grid",
          zIndex: 10,
          style: {
            gridTemplateColumns: "350px 1fr",
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
              style: { pointerEvents: "auto", margin: "20px 0 20px 20px", padding: "0" },
              motion: "panelEntry"
            }
          ]
        }
      ]
    }
  };
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
  listLive2dModels,
  getTtsConfig,
  setPersona,
  setResponseLanguage,
  getProactiveEnabled,
  // Config Getters
  getLlmConfig,
  getImageGenConfig,
  getTelegramConfig,
  getTelegramStatus,
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
  // New: MCP Management
  addMcpServer,
  removeMcpServer,
  reconnectMcpServer,
  refreshMcpTools,
  // New: Memory
  listMemories,
  updateMemory,
  deleteMemory,
  // New: Singing (RVC)
  checkRvcStatus,
  listRvcModels,
  convertSinging,
  // onSingingProgress — reserved for future use
  // New: ImageGen
  testSdConnection,
  setWindowSize,
  onChatImageGen,
  generateImage,
  // New: Vision
  captureScreenNow,
  // New: Live2D
  deleteLive2dModel,
  importLive2dZip,
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
  type McpServerStatus,
  type ProviderStatus,
  type VoiceProfile,
  type GptSovitsModels,
  type MemoryRecord,
  type TelegramConfig,
  type TelegramStatus,
  type RvcModelInfo,
  type SingingProgressEvent,
  type CharacterRecord,
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

const ONBOARDING_STATUS_KEY = "kokoro_onboarding_status";

const ONBOARDING_LANGUAGE_NAMES: Record<OnboardingLanguageCode, string> = {
  en: "English",
  zh: "中文",
  ja: "日本語",
  ko: "한국어",
  ru: "Русский",
};

function normalizeOnboardingLanguageCode(language: string | null | undefined): OnboardingLanguageCode {
  const base = language?.split("-")[0];
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
    const saved = localStorage.getItem("kokoro_settings_active_tab");
    return (saved as SettingsTabId) || "bg";
  });
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(() =>
    localStorage.getItem(ONBOARDING_STATUS_KEY) ? null : "language"
  );
  const [onboardingLanguage, setOnboardingLanguage] = useState<OnboardingLanguageCode>(() =>
    normalizeOnboardingLanguageCode(
      localStorage.getItem("kokoro_app_language")
      || (typeof navigator !== "undefined" ? navigator.language : "zh")
    )
  );
  const [displayMode, setDisplayMode] = useState<Live2DDisplayMode>(
    () => (localStorage.getItem("kokoro_display_mode") as Live2DDisplayMode) || "full"
  );
  const bgSlideshow = useBackgroundSlideshow();
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);

  // Subscribe to registry changes so SettingsPanel slot picks up mod overrides.
  useSyncExternalStore(_subscribeFn, _getSnap);

  const [customModelPath, setCustomModelPath] = useState<string | null>(
    () => localStorage.getItem("kokoro_custom_model_path")
  );

  const [gazeTracking, setGazeTracking] = useState<boolean>(
    () => localStorage.getItem("kokoro_gaze_tracking") !== "false"
  );
  const [renderFps, setRenderFps] = useState<number>(60);
  const activeLive2dModelPath = customModelPath ?? BUILTIN_LIVE2D_MODEL_PATH;

  const handleGazeTrackingChange = (enabled: boolean) => {
    setGazeTracking(enabled);
    localStorage.setItem("kokoro_gaze_tracking", enabled ? "true" : "false");
  };

  // ── Global Settings State ──
  const [availableModels, setAvailableModels] = useState<Live2dModelInfo[]>([]);
  const [persona, setPersonaState] = useState(() => localStorage.getItem("kokoro_persona") || "");
  const [responseLanguage, setResponseLanguageState] = useState(() => localStorage.getItem("kokoro_response_language") || "zh");

  // Full Config State
  const [ttsConfig, setTtsConfig] = useState<TtsSystemConfig | undefined>(undefined);
  const [llmConfig, setLlmConfig] = useState<LlmConfig | undefined>(undefined);
  const [sttConfig, setSttConfig] = useState<SttConfig | undefined>(undefined);
  const [visionConfig, setVisionConfig] = useState<VisionConfig | undefined>(undefined);
  const [imageGenConfig, setImageGenConfig] = useState<ImageGenSystemConfig | undefined>(undefined);
  const [telegramConfig, setTelegramConfig] = useState<TelegramConfig | undefined>(undefined);
  const [telegramStatus, setTelegramStatus] = useState<TelegramStatus | undefined>(undefined);

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
  // New: Memory, Sing, MCP, Vision, ImageGen dynamic state for mods
  const [memoryList, setMemoryList] = useState<MemoryRecord[]>([]);
  const [memoryTotal, setMemoryTotal] = useState(0);
  const [rvcAvailable, setRvcAvailable] = useState(false);
  const [rvcModels, setRvcModels] = useState<RvcModelInfo[]>([]);
  const [singProgress, setSingProgress] = useState<SingingProgressEvent | null>(null);
  const [sdModels, setSdModels] = useState<string[]>([]);
  const [capturedScreenUrl, setCapturedScreenUrl] = useState<string | null>(null);
  const [userLanguage, setUserLanguageState] = useState(() => localStorage.getItem("kokoro_user_language") || "zh");
  const [proactiveEnabled, setProactiveEnabledState] = useState(() => {
    const saved = localStorage.getItem("kokoro_proactive_enabled");
    return saved !== null ? saved === "true" : true;
  });

  const modelUrl = useMemo(() => {
    if (customModelPath) {
      return live2dUrl(customModelPath);
    }
    return "/live2d/haru/haru_greeter_t03.model3.json";
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
    () => createLayout({ mode: displayMode, modelUrl, modelPath: activeLive2dModelPath, gazeTracking, renderFps }),
    [displayMode, modelUrl, activeLive2dModelPath, gazeTracking, renderFps]
  );

  const handleDisplayModeChange = (mode: Live2DDisplayMode) => {
    setDisplayMode(mode);
    localStorage.setItem("kokoro_display_mode", mode);
  };

  const handleCustomModelChange = (path: string | null) => {
    setCustomModelPath(path);
    if (path) {
      localStorage.setItem("kokoro_custom_model_path", path);
    } else {
      localStorage.removeItem("kokoro_custom_model_path");
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
    localStorage.setItem("kokoro_app_language", language);
    localStorage.setItem("kokoro_response_language", label);
    localStorage.setItem("kokoro_user_language", label);
    setResponseLanguageState(label);
    setUserLanguageState(label);
    setResponseLanguage(label).catch(console.error);
    setUserLanguage(label).catch(console.error);
  };

  const previewOnboardingLanguage = (language: OnboardingLanguageCode) => {
    applyOnboardingLanguage(language);
  };

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
    ]).then(([tts, llm, stt, vision, imageGen, mcp, mods, proactive, telegram, telegramStatus]) => {
      setTtsConfig(tts);
      setLlmConfig(llm);
      setSttConfig(stt);
      setVisionConfig(vision);
      setImageGenConfig(imageGen);
      setMcpServers(mcp);
      setModList(mods);
      setProactiveEnabledState(proactive);
      localStorage.setItem("kokoro_proactive_enabled", String(proactive));
      setTelegramConfig(telegram);
      setTelegramStatus(telegramStatus);
    }).catch(err => console.error("[App] Failed to fetch initial configs:", err));

    // Sync language settings to backend on startup
    const savedResponseLang = localStorage.getItem("kokoro_response_language") || "";
    const savedUserLang = localStorage.getItem("kokoro_user_language") || "";
    if (savedResponseLang) setResponseLanguage(savedResponseLang).catch(console.error);
    if (savedUserLang) setUserLanguage(savedUserLang).catch(console.error);

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
    import("./ui/widgets/CharacterManager").then(async ({ composeSystemPrompt }) => {
      const { listCharacters, setPersona, setActiveCharacterId } = await import("./lib/kokoro-bridge");
      try {
        const all = await listCharacters();
        setCharacters(all);
        const savedId = localStorage.getItem("kokoro_active_character_id");
        const char = savedId ? all.find(c => c.id === savedId) ?? all[0] : all[0];
        if (char) {
          const prompt = composeSystemPrompt(char);
          localStorage.setItem("kokoro_persona", prompt);
          setPersonaState(prompt);
          await setPersona(prompt);
          await setActiveCharacterId(char.id);
          console.log("[App] Synced persona on startup:", char.name);
        }
      } catch (e) {
        console.error("[App] Failed to sync persona on startup:", e);
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

  // ── MOD System: Action listener for UI components ──
  const handleModAction = (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (detail.action === 'close_settings') {
      setSettingsOpen(false);
    }
    if (detail.action === 'send_message' && detail.data?.message) {
      streamChat({
        message: detail.data.message,
        character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
      }).catch(err => console.error("[App] Mod send_message failed:", err));
    }
    // New settings actions
    if (detail.action === 'set_model' && detail.data?.model) {
      const target = availableModels.find(m => m.name === detail.data.model || m.path === detail.data.model);
      if (target) handleCustomModelChange(target.path);
    }
    if (detail.action === 'set_persona' && detail.data?.persona) {
      setPersonaState(detail.data.persona);
      localStorage.setItem("kokoro_persona", detail.data.persona);
      setPersona(detail.data.persona).catch(console.error);
    }
    if (detail.action === 'set_language' && detail.data?.language) {
      setResponseLanguageState(detail.data.language);
      localStorage.setItem("kokoro_response_language", detail.data.language);
      setResponseLanguage(detail.data.language).catch(console.error);
    }
    if (detail.action === 'set_display_mode' && detail.data?.mode) {
      handleDisplayModeChange(detail.data.mode);
    }
    if (detail.action === 'set_background' && detail.data?.url) {
      setGeneratedImage(detail.data.url);
      bgSlideshow.setConfig({ mode: "generated" });
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

    if (detail.action === 'set_voice_interrupt') {
      setVoiceInterrupt(!!detail.data?.enabled);
    }

    if (detail.action === 'set_proactive_enabled') {
      const enabled = !!detail.data?.enabled;
      setProactiveEnabledState(enabled);
      localStorage.setItem("kokoro_proactive_enabled", String(enabled));
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

    // ── TTS Playback Actions ────────────────────────
    if (detail.action === 'set_tts_playback' && detail.data) {
      const { speed, pitch, voice, provider } = detail.data;
      if (speed !== undefined) localStorage.setItem('kokoro_tts_speed', String(speed));
      if (pitch !== undefined) localStorage.setItem('kokoro_tts_pitch', String(pitch));
      if (voice !== undefined) localStorage.setItem('kokoro_tts_voice', voice);
      if (provider !== undefined) localStorage.setItem('kokoro_tts_provider', provider);
    }

    // ── MCP Actions ────────────────────────────────
    if (detail.action === 'add_mcp_server' && detail.data?.config) {
      addMcpServer(detail.data.config)
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
        .catch(err => console.error('[App] Memory update failed:', err));
    }
    if (detail.action === 'delete_memory' && detail.data?.id !== undefined) {
      deleteMemory(detail.data.id)
        .catch(err => console.error('[App] Memory delete failed:', err));
    }

    // ── Singing (RVC) Actions ──────────────────────
    if (detail.action === 'check_rvc_status') {
      checkRvcStatus()
        .then(available => setRvcAvailable(available))
        .catch(err => console.error('[App] RVC check failed:', err));
    }
    if (detail.action === 'list_rvc_models') {
      listRvcModels()
        .then(models => setRvcModels(models))
        .catch(err => console.error('[App] RVC models list failed:', err));
    }
    if (detail.action === 'convert_singing' && detail.data) {
      const { audioPath, modelName, pitchShift, separateVocals, f0Method } = detail.data;
      convertSinging(audioPath, modelName, pitchShift, separateVocals, f0Method)
        .then(result => setSingProgress({ stage: 'done', progress: 100, output_path: result.output_path }))
        .catch(err => console.error('[App] Singing conversion failed:', err));
    }

    // ── ImageGen Actions ───────────────────────────
    if (detail.action === 'test_sd_connection' && detail.data?.baseUrl) {
      testSdConnection(detail.data.baseUrl)
        .then(models => setSdModels(models))
        .catch(err => console.error('[App] SD connection test failed:', err));
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
      setCustomModelPath(newPath);
      localStorage.setItem('kokoro_custom_model', newPath || '');
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
                localStorage.setItem('kokoro_custom_model', modelPath);
                const models = await listLive2dModels();
                setAvailableModels(models);
              } catch (e) { console.error('[App] import zip failed:', e); }
            } else {
              setCustomModelPath(selected);
              localStorage.setItem('kokoro_custom_model', selected);
            }
          }
        });
      });
    }

    // ── Language Actions ───────────────────────────
    if (detail.action === 'set_user_language' && detail.data?.language) {
      setUserLanguageState(detail.data.language);
      localStorage.setItem('kokoro_user_language', detail.data.language);
      setUserLanguage(detail.data.language).catch(console.error);
    }

    // ── User Profile Actions ───────────────────────
    if (detail.action === 'set_user_name' && detail.data?.name) {
      localStorage.setItem('kokoro_user_name', detail.data.name);
      // No backend syncing needed for name currently, just local storage for UI
    }
    if (detail.action === 'set_user_persona' && detail.data?.persona) {
      localStorage.setItem('kokoro_user_persona', detail.data.persona);
    }

    // ── Character Actions ─────────────────────────
    if (detail.action === 'list_characters') {
      import('./lib/kokoro-bridge').then(async ({ listCharacters }) => {
        const all = await listCharacters();
        setCharacters(all);
      }).catch(console.error);
    }
    if (detail.action === 'select_character' && detail.data?.id != null) {
      import('./ui/widgets/CharacterManager').then(async ({ composeSystemPrompt }) => {
        const { listCharacters } = await import('./lib/kokoro-bridge');
        const all = await listCharacters();
        const char = all.find(c => c.id === detail.data.id);
        if (char) {
          localStorage.setItem('kokoro_active_character_id', char.id);
          const prompt = composeSystemPrompt(char);
          setPersonaState(prompt);
          setPersona(prompt).catch(console.error);
          setCharacters(all);
        }
      }).catch(console.error);
    }
    if (detail.action === 'create_character') {
      import('./lib/kokoro-bridge').then(async ({ createCharacter, listCharacters }) => {
        const id = crypto.randomUUID();
        const now = Date.now();
        await createCharacter({ id, name: 'New Character', persona: '', user_nickname: 'User', source_format: 'manual', created_at: now, updated_at: now });
        const all = await listCharacters();
        setCharacters(all);
        const newChar = all.find(c => c.id === id);
        if (newChar) {
          localStorage.setItem('kokoro_active_character_id', newChar.id);
          const { composeSystemPrompt } = await import('./ui/widgets/CharacterManager');
          const prompt = composeSystemPrompt(newChar);
          setPersonaState(prompt);
          setPersona(prompt).catch(console.error);
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
          const { createCharacter, listCharacters } = await import('./lib/kokoro-bridge');
          const profile = await parseCharacterCard(file);
          const id = crypto.randomUUID();
          const now = Date.now();
          await createCharacter({ id, ...profile, created_at: now, updated_at: now });
          const all = await listCharacters();
          setCharacters(all);
          const char = all.find(c => c.id === id);
          if (char) {
            localStorage.setItem('kokoro_active_character_id', char.id);
            const { composeSystemPrompt } = await import('./ui/widgets/CharacterManager');
            const prompt = composeSystemPrompt(char);
            setPersonaState(prompt);
            setPersona(prompt).catch(console.error);
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
      <motion.button
        initial={false}
        whileHover="hover"
        whileTap={{ scale: 0.97 }}
        transition={{ type: "spring", stiffness: 360, damping: 26 }}
        onClick={() => setSettingsOpen(true)}
        data-onboarding-id="settings-button"
        className="fixed top-[34px] right-[35px] z-50 p-3 rounded-full bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)] border border-[var(--color-border)] text-[var(--color-text-secondary)] shadow-lg transition-[color,border-color,box-shadow] duration-200 ease-out hover:border-[var(--color-border-accent)] hover:text-[var(--color-accent)] hover:shadow-[0_0_18px_rgba(0,240,255,0.18)]"
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
            rvcAvailable={rvcAvailable}
            rvcModels={rvcModels}
            singProgress={singProgress}
            sdModels={sdModels}
            capturedScreenUrl={capturedScreenUrl}
            userLanguage={userLanguage}
            activeCharacterId={localStorage.getItem('kokoro_active_character_id') || 'default'}
            characters={characters}
            // User Profile (from localStorage)
            userName={localStorage.getItem('kokoro_user_name') || ''}
            userPersona={localStorage.getItem('kokoro_user_persona') || ''}
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

      {/* Camera watcher — lives at app root so it persists when settings panel closes */}
      <CameraWatcher
        enabled={visionConfig?.camera_enabled ?? false}
        deviceId={visionConfig?.camera_device_id ?? undefined}
      />
    </ThemeProvider>
  );
}

export default App;
