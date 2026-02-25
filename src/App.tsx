import { useState, useEffect, useMemo, useSyncExternalStore } from "react";
import { motion } from "framer-motion";
import { Settings } from "lucide-react";
import { LayoutRenderer } from "./ui/layout/LayoutRenderer";
import { LayoutConfig } from "./ui/layout/types";
import { ThemeProvider } from "./ui/theme/ThemeContext";
import { defaultTheme } from "./ui/theme/default";
import { registry } from "./ui/registry/ComponentRegistry";
import { registerCoreComponents } from "./core/init";
import { ttsService } from "./core/services";
import SettingsPanel from "./ui/widgets/SettingsPanel";
import BackgroundLayer from "./ui/widgets/BackgroundLayer";
import { useBackgroundSlideshow } from "./ui/hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "./features/live2d/Live2DViewer";

// Register components synchronously before first render
registerCoreComponents();

// Build layout config as a function of displayMode
function createLayout(displayMode: { mode: Live2DDisplayMode; modelUrl: string }): LayoutConfig {
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
            displayMode: displayMode.mode,
          }
        },
        {
          id: "ui-grid",
          type: "grid",
          zIndex: 10,
          style: {
            gridTemplateColumns: "350px 1fr",
            gridTemplateRows: "1fr 60px",
            gridTemplateAreas: `
                        "highlight main"
                        "footer footer"
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
            },
            {
              id: "footer-bar",
              type: "component",
              component: "FooterBar",
              area: "footer",
              style: { pointerEvents: "auto" }
            }
          ]
        }
      ]
    }
  };
}

import { convertFileSrc } from "@tauri-apps/api/core";
import {
  onImageGenDone,
  onModThemeOverride,
  onModComponentsRegister,
  onModUiMessage,
  onModScriptEvent,
  onChatDelta,
  onChatDone,
  onChatExpression,
  onChatAction,
  streamChat,
  dispatchModEvent,
  listLive2dModels,
  getTtsConfig,
  setPersona,
  setResponseLanguage,
  getProactiveEnabled,
  // Config Getters
  getLlmConfig,
  getImageGenConfig,
  getVisionConfig,
  getSttConfig,
  listMcpServers,
  listMods,
  listTtsProviders,
  listTtsVoices,
  // Actions
  fetchModels,
  listOllamaModels,
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
  // New: Vision
  captureScreenNow,
  // New: Live2D
  deleteLive2dModel,
  importLive2dZip,
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
  type RvcModelInfo,
  type SingingProgressEvent,
} from "./lib/kokoro-bridge";
import type { ThemeConfig } from "./ui/layout/types";
import { modMessageBus } from "./ui/mods/ModMessageBus";

let _regSnap = 0;
const _subscribeFn = (cb: () => void) => {
  return registry.subscribe(() => { _regSnap++; cb(); });
};
const _getSnap = () => _regSnap;

function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
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

  // Lists
  const [mcpServers, setMcpServers] = useState<McpServerStatus[]>([]);
  const [modList, setModList] = useState<ModManifest[]>([]);
  const [ttsProviders, setTtsProviders] = useState<ProviderStatus[]>([]);
  const [ttsVoices, setTtsVoices] = useState<VoiceProfile[]>([]);

  // Character list for mod settings
  const [characters, setCharacters] = useState<any[]>([]);

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
  const [proactiveEnabled, setProactiveEnabledState] = useState(true);

  const modelUrl = useMemo(() => {
    if (customModelPath) {
      return `http://live2d.localhost/${customModelPath}`;
    }
    return "https://cdn.jsdelivr.net/gh/guansss/pixi-live2d-display/test/assets/haru/haru_greeter_t03.model3.json";
  }, [customModelPath]);

  const layout = useMemo(() => createLayout({ mode: displayMode, modelUrl }), [displayMode, modelUrl]);

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

  useEffect(() => {
    ttsService.init();

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
    ]).then(([tts, llm, stt, vision, imageGen, mcp, mods, proactive]) => {
      setTtsConfig(tts);
      setLlmConfig(llm);
      setSttConfig(stt);
      setVisionConfig(vision);
      setImageGenConfig(imageGen);
      setMcpServers(mcp);
      setModList(mods);
      setProactiveEnabledState(proactive);
    }).catch(err => console.error("[App] Failed to fetch initial configs:", err));

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
      const { characterDb } = await import("./lib/db");
      try {
        const all = await characterDb.getAll();
        setCharacters(all); // Populate character list for mod settings
        const savedId = localStorage.getItem("kokoro_active_character_id");
        const char = savedId ? all.find(c => c.id === Number(savedId)) : all[0];
        if (char) {
          const prompt = composeSystemPrompt(char);
          localStorage.setItem("kokoro_persona", prompt);
          setPersonaState(prompt);
          await setPersona(prompt);
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
    const unlistenModChatDelta = onChatDelta((delta) => {
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-delta', delta },
      });
      // Forward to QuickJS scripts so Kokoro.on('chat', ...) works
      dispatchModEvent('chat', { delta }).catch(() => { });
    });

    const unlistenModExpression = onChatExpression((data) => {
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-expression', ...data },
      });
      dispatchModEvent('expression', data).catch(() => { });
    });

    const unlistenModAction = onChatAction((data) => {
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-action', ...data },
      });
      dispatchModEvent('action', data).catch(() => { });
    });

    const unlistenModChatDone = onChatDone(() => {
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: 'chat-done' },
      });
      dispatchModEvent('chat-done', {}).catch(() => { });
    });

    // ── MOD System: Script events → broadcast to iframes ──
    const unlistenModScriptEvent = onModScriptEvent(({ event, payload }) => {
      console.log(`[App] Script event '${event}' → broadcasting to iframes`);
      modMessageBus.broadcast({
        type: 'event',
        payload: { name: `script:${event}`, data: payload },
      });
    });

    return () => {
      ttsService.cleanup();
      unlistenImageGen.then(unlisten => unlisten());
      unlistenModTheme.then(unlisten => unlisten());
      unlistenModComponents.then(unlisten => unlisten());
      unlistenModUiMessage.then(unlisten => unlisten());
      unlistenModChatDelta.then(unlisten => unlisten());
      unlistenModExpression.then(unlisten => unlisten());
      unlistenModAction.then(unlisten => unlisten());
      unlistenModChatDone.then(unlisten => unlisten());
      unlistenModScriptEvent.then(unlisten => unlisten());
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
      import('./lib/db').then(async ({ characterDb }) => {
        const all = await characterDb.getAll();
        setCharacters(all);
      }).catch(console.error);
    }
    if (detail.action === 'select_character' && detail.data?.id != null) {
      import('./ui/widgets/CharacterManager').then(async ({ composeSystemPrompt }) => {
        const { characterDb } = await import('./lib/db');
        const all = await characterDb.getAll();
        const char = all.find(c => c.id === Number(detail.data.id));
        if (char) {
          localStorage.setItem('kokoro_active_character_id', String(char.id));
          const prompt = composeSystemPrompt(char);
          setPersonaState(prompt);
          setPersona(prompt).catch(console.error);
          setCharacters(all);
        }
      }).catch(console.error);
    }
    if (detail.action === 'create_character') {
      import('./lib/db').then(async ({ characterDb }) => {
        const now = Date.now();
        const newId = await characterDb.add({
          name: 'New Character',
          persona: '',
          userNickname: 'User',
          sourceFormat: 'manual',
          createdAt: now,
          updatedAt: now,
        });
        const all = await characterDb.getAll();
        setCharacters(all);
        // Auto-select the new character
        const newChar = all.find(c => c.id === newId);
        if (newChar) {
          localStorage.setItem('kokoro_active_character_id', String(newId));
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
          const { characterDb } = await import('./lib/db');
          const profile = await parseCharacterCard(file);
          const now = Date.now();
          const newId = await characterDb.add({ ...profile, createdAt: now, updatedAt: now });
          const all = await characterDb.getAll();
          setCharacters(all);
          localStorage.setItem('kokoro_active_character_id', String(newId));
          const { composeSystemPrompt } = await import('./ui/widgets/CharacterManager');
          const char = all.find(c => c.id === newId);
          if (char) {
            setPersonaState(composeSystemPrompt(char));
            setPersona(composeSystemPrompt(char)).catch(console.error);
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
  }, []);

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
        whileHover={{ scale: 1.1, rotate: 30 }}
        whileTap={{ scale: 0.9 }}
        onClick={() => setSettingsOpen(true)}
        className="fixed top-4 right-4 z-50 p-2.5 rounded-lg bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)] border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] shadow-lg transition-colors"
        aria-label="Open settings"
      >
        <Settings size={18} strokeWidth={1.5} />
      </motion.button>

      {/* SettingsPanel is retrieved from registry to allow mod overrides */}
      {settingsOpen && (() => {
        const SettingsComponent = registry.get("SettingsPanel") || SettingsPanel;
        const isMod = registry.isModComponent("SettingsPanel");
        const component = (
          <SettingsComponent
            isOpen={settingsOpen}
            onClose={() => setSettingsOpen(false)}
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
            // External state for Mod
            availableModels={availableModels}
            persona={persona}
            responseLanguage={responseLanguage}
            ttsConfig={ttsConfig}
            llmConfig={llmConfig}
            sttConfig={sttConfig}
            visionConfig={visionConfig}
            imageGenConfig={imageGenConfig}
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
          />
        );

        if (isMod) {
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
    </ThemeProvider>
  );
}

export default App;
