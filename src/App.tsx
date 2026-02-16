import { useState, useEffect, useMemo } from "react";
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
  onChatExpression,
  onChatAction,
  dispatchModEvent,
  type ImageGenResult,
  type ModThemeJson,
} from "./lib/kokoro-bridge";
import type { ThemeConfig } from "./ui/layout/types";
import { modMessageBus } from "./ui/mods/ModMessageBus";

function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [displayMode, setDisplayMode] = useState<Live2DDisplayMode>(
    () => (localStorage.getItem("kokoro_display_mode") as Live2DDisplayMode) || "full"
  );
  const bgSlideshow = useBackgroundSlideshow();
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);

  const [customModelPath, setCustomModelPath] = useState<string | null>(
    () => localStorage.getItem("kokoro_custom_model_path")
  );

  const modelUrl = useMemo(() => {
    if (customModelPath) {
      // customModelPath is a relative path like "hiyori_pro_zh/runtime/model.model3.json"
      // Serve via our custom live2d:// protocol for proper relative URL resolution
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

    // Sync the active character's persona to the backend on startup
    // (CharacterManager only mounts when Settings > PERSONA tab is open)
    import("./ui/widgets/CharacterManager").then(async ({ composeSystemPrompt }) => {
      const { characterDb } = await import("./lib/db");
      const { setPersona } = await import("./lib/kokoro-bridge");
      try {
        const all = await characterDb.getAll();
        const savedId = localStorage.getItem("kokoro_active_character_id");
        const char = savedId ? all.find(c => c.id === Number(savedId)) : all[0];
        if (char) {
          const prompt = composeSystemPrompt(char);
          localStorage.setItem("kokoro_persona", prompt);
          await setPersona(prompt);
          console.log("[App] Synced persona on startup:", char.name);
        }
      } catch (e) {
        console.error("[App] Failed to sync persona on startup:", e);
      }
    });

    // Listen for generated images
    const unlistenImageGen = onImageGenDone((result: ImageGenResult) => {
      // Convert local file path to asset URL
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
      unlistenModScriptEvent.then(unlisten => unlisten());
    };
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

      <SettingsPanel
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
      />
    </ThemeProvider>
  );
}

export default App;
