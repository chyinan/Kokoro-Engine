import { useState, useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { X, Key, User, Volume2, Package, Image, PersonStanding, Save, Check, Sparkles, Brain, Mic, Music, Eye, Server, Bot, Shield, HardDrive, Ghost } from "lucide-react";
import { ModList } from "../mods/ModList";
import { Select } from "@/components/ui/select";
import CharacterManager from "./CharacterManager";
import ImageGenSettings from "./ImageGenSettings";
import MemoryPanel from "./MemoryPanel";
import ApiTab from "./settings/ApiTab";
import TtsTab from "./settings/TtsTab";
import SttTab from "./settings/SttTab";
import SingTab from "./settings/SingTab";
import ModelTab from "./settings/ModelTab";
import BackgroundTab from "./settings/BackgroundTab";
import VisionTab from "./settings/VisionTab";
import McpTab from "./settings/McpTab";
import TelegramTab from "./settings/TelegramTab";
import { JailbreakTab } from "./settings/JailbreakTab";
import { BackupTab } from "./settings/BackupTab";
import PetTab from "./settings/PetTab";
import { useTranslation } from "react-i18next";
import { setPersona, setResponseLanguage, setUserLanguage, listTtsProviders, listTtsVoices, getTtsConfig, saveTtsConfig, saveImageGenConfig, getSttConfig, saveSttConfig, saveTelegramConfig, getTelegramConfig, saveLlmConfig } from "../../lib/kokoro-bridge";
import type {
    ProviderStatus,
    VoiceProfile,
    TtsSystemConfig,
    ImageGenSystemConfig,
    SttConfig,
    TelegramConfig,
    TelegramStatus,
    Live2dModelInfo,
    LlmConfig,
    VisionConfig,
    McpServerStatus,
    ModManifest,
    GptSovitsModels,
    MemoryRecord,
    RvcModelInfo,
    SingingProgressEvent,
    CharacterRecord,
} from "../../lib/kokoro-bridge";
import type { BackgroundConfig } from "../hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "../../features/live2d/Live2DViewer";

type TabId = "api" | "persona" | "tts" | "stt" | "sing" | "mods" | "bg" | "model" | "imagegen" | "memory" | "vision" | "mcp" | "telegram" | "jailbreak" | "backup" | "pet";

export interface BackgroundControls {
    config: BackgroundConfig;
    setConfig: (update: Partial<BackgroundConfig>) => void;
    images: string[];
    importFiles: (files: FileList) => Promise<number>;
    removeImage: (index: number) => Promise<void>;
    clearImages: () => Promise<void>;
    imageCount: number;
}

interface SettingsPanelProps {
    isOpen: boolean;
    onClose: () => void;
    backgroundControls: BackgroundControls;
    displayMode: Live2DDisplayMode;
    onDisplayModeChange: (mode: Live2DDisplayMode) => void;
    customModelPath: string | null;
    onCustomModelChange: (path: string | null) => void;
    gazeTracking?: boolean;
    onGazeTrackingChange?: (enabled: boolean) => void;
    renderFps: number;
    onRenderFpsChange: (fps: number) => void;
    // Optional props for external state management (Mod support)
    availableModels?: Live2dModelInfo[];
    persona?: string;
    responseLanguage?: string;
    ttsConfig?: TtsSystemConfig;
    llmConfig?: LlmConfig;
    onLlmConfigSaved?: (cfg: LlmConfig) => void;
    sttConfig?: SttConfig;
    visionConfig?: VisionConfig;
    onVisionConfigChange?: (cfg: VisionConfig) => void;
    imageGenConfig?: ImageGenSystemConfig;
    telegramConfig?: TelegramConfig;
    mcpServers?: McpServerStatus[];
    modList?: ModManifest[];
    ttsProviders?: ProviderStatus[];
    ttsVoices?: VoiceProfile[];
    // Dynamic State
    voiceInterrupt?: boolean;
    onVoiceInterruptChange?: (v: boolean) => void;
    fetchedLlmModels?: string[];
    scannedTtsModels?: Record<string, GptSovitsModels>;
    // New: Full Parity Props
    memoryList?: MemoryRecord[];
    memoryTotal?: number;
    rvcAvailable?: boolean;
    rvcModels?: RvcModelInfo[];
    singProgress?: SingingProgressEvent | null;
    sdModels?: string[];
    capturedScreenUrl?: string | null;
    userLanguage?: string;
    activeCharacterId?: string;
    characters?: CharacterRecord[];
    // User Profile
    userName?: string;
    userPersona?: string;
    proactiveEnabled?: boolean;
    initialTelegramStatus?: TelegramStatus | null;
}

const tabs: { id: TabId; label: string; icon: typeof Key }[] = [
    // 核心体验
    { id: "persona", label: "settings.tabs.persona", icon: User },
    { id: "model", label: "settings.tabs.model", icon: PersonStanding },
    { id: "tts", label: "settings.tabs.tts", icon: Volume2 },
    { id: "stt", label: "settings.tabs.stt", icon: Mic },
    { id: "bg", label: "settings.tabs.bg", icon: Image },
    { id: "imagegen", label: "settings.tabs.imagegen", icon: Sparkles },
    // AI 能力扩展
    { id: "vision", label: "settings.tabs.vision", icon: Eye },
    { id: "memory", label: "settings.tabs.memory", icon: Brain },
    { id: "mcp", label: "settings.tabs.mcp", icon: Server },
    // 外部集成
    { id: "mods", label: "settings.tabs.mods", icon: Package },
    { id: "sing", label: "settings.tabs.sing", icon: Music },
    { id: "telegram", label: "settings.tabs.telegram", icon: Bot },
    // 系统 / 高级
    { id: "api", label: "settings.tabs.api", icon: Key },
    { id: "jailbreak", label: "settings.tabs.jailbreak", icon: Shield },
    { id: "pet", label: "settings.tabs.pet", icon: Ghost },
    { id: "backup", label: "settings.tabs.backup", icon: HardDrive },
];

function getDefaultTtsVoice(providerId: string, voices: VoiceProfile[]): string {
    if (providerId === "browser") {
        return "";
    }

    if (providerId === "openai") {
        return "alloy";
    }

    const providerVoice = voices.find(v => v.provider_id === providerId);
    return providerVoice?.voice_id || "";
}

function stripProviderVoiceId(providerId: string, voiceId: string): string {
    return voiceId.startsWith(`${providerId}_`) ? voiceId.slice(providerId.length + 1) : voiceId;
}

function usesShortTtsVoiceId(providerId: string, ttsConfig?: TtsSystemConfig | null): boolean {
    const provider = ttsConfig?.providers.find(p => p.id === providerId);
    return provider?.provider_type === "edge_tts";
}

function normalizeTtsVoice(
    providerId: string,
    voice: string,
    voices: VoiceProfile[],
    ttsConfig?: TtsSystemConfig | null,
): string {
    if (!voice) {
        if (providerId === "openai") {
            const provider = ttsConfig?.providers.find(p => p.id === providerId);
            return provider?.default_voice || "alloy";
        }

        if (usesShortTtsVoiceId(providerId, ttsConfig)) {
            const provider = ttsConfig?.providers.find(p => p.id === providerId);
            if (provider?.default_voice) {
                return provider.default_voice;
            }
            // No default_voice configured — prefer the well-known zh-CN default
            // that the Rust backend uses, rather than grabbing the first
            // alphabetical voice (which would be Afrikaans / Arabic / Spanish…).
            const zhVoice = voices.find(
                v => v.provider_id === providerId && v.voice_id.includes("zh-CN-XiaoyiNeural")
            );
            if (zhVoice) return stripProviderVoiceId(providerId, zhVoice.voice_id);
            const providerVoice = voices.find(v => v.provider_id === providerId);
            return providerVoice ? stripProviderVoiceId(providerId, providerVoice.voice_id) : "";
        }

        return getDefaultTtsVoice(providerId, voices);
    }

    if (providerId === "browser") {
        return voice === "browser_default" ? voice : "";
    }

    if (providerId === "openai") {
        return voice;
    }

    const matchesProvider = voices.some(v => {
        if (v.provider_id !== providerId) return false;
        if (usesShortTtsVoiceId(providerId, ttsConfig)) {
            return stripProviderVoiceId(providerId, v.voice_id) === voice;
        }
        return v.voice_id === voice;
    });

    if (matchesProvider) {
        return voice;
    }

    if (usesShortTtsVoiceId(providerId, ttsConfig)) {
        const provider = ttsConfig?.providers.find(p => p.id === providerId);
        if (provider?.default_voice) {
            return provider.default_voice;
        }
        // Same zh-CN preference as above for the "voice doesn't match" branch.
        const zhVoice = voices.find(
            v => v.provider_id === providerId && v.voice_id.includes("zh-CN-XiaoyiNeural")
        );
        if (zhVoice) return stripProviderVoiceId(providerId, zhVoice.voice_id);
    }

    return getDefaultTtsVoice(providerId, voices);
}

export default function SettingsPanel({ isOpen, onClose, backgroundControls, displayMode, onDisplayModeChange, customModelPath, onCustomModelChange, gazeTracking: gazeTrackingProp, onGazeTrackingChange, renderFps, onRenderFpsChange, sttConfig: sttConfigProp, voiceInterrupt: _voiceInterruptProp, imageGenConfig: imageGenConfigProp, telegramConfig: telegramConfigProp, llmConfig: llmConfigProp, onLlmConfigSaved, visionConfig: visionConfigProp, mcpServers: mcpServersProp, characters: charactersProp, initialTelegramStatus, onVisionConfigChange }: SettingsPanelProps) {
    const { t, i18n } = useTranslation();
    const [activeTab, setActiveTab] = useState<TabId>(() => {
        const saved = localStorage.getItem("kokoro_settings_active_tab");
        return (saved as TabId) || "bg";
    });
    const bg = backgroundControls;
    const overlayRef = useRef<HTMLDivElement>(null);
    const latestLlmConfigRef = useRef<LlmConfig | null>(llmConfigProp ?? null);

    // ── Local Buffer State ───────────────────────────────
    // We hold changes locally until "Save" is clicked.

    // Display & Model
    const [localDisplayMode, setLocalDisplayMode] = useState(displayMode);
    const [localCustomModelPath, setLocalCustomModelPath] = useState(customModelPath);
    const [localGazeTracking, setLocalGazeTracking] = useState(gazeTrackingProp ?? true);

    // Background Config
    const [localBgConfig, setLocalBgConfig] = useState<BackgroundConfig>({ ...bg.config });

    // Sync local state only when the panel opens; while editing, keep local form state authoritative.
    useEffect(() => {
        if (isOpen) {
            setLocalDisplayMode(displayMode);
            setLocalCustomModelPath(customModelPath);
            latestLlmConfigRef.current = llmConfigProp ?? null;
            setLocalGazeTracking(gazeTrackingProp ?? true);
            setLocalBgConfig({ ...bg.config });
            setPersonaText(localStorage.getItem("kokoro_persona") || "You are a friendly, warm companion character. Respond with personality and emotion.");
            setTtsVoice(localStorage.getItem("kokoro_tts_voice") || "");
            setTtsSpeed(localStorage.getItem("kokoro_tts_speed") || "1.0");
            setTtsPitch(localStorage.getItem("kokoro_tts_pitch") || "1.0");
            setTtsProviderId(localStorage.getItem("kokoro_tts_provider") || "browser");
            setTtsEnabled(localStorage.getItem("kokoro_tts_enabled") === "true");
            setVisionEnabled(localStorage.getItem("kokoro_vision_enabled") === "true");
            setVoiceInterrupt(localStorage.getItem("kokoro_voice_interrupt") === "true");
            setResponseLang(localStorage.getItem("kokoro_response_language") || "");
            setUserLang(localStorage.getItem("kokoro_user_language") || "");
            setLocalTelegramConfig(telegramConfigProp ?? null);
            fetchData();
        }
    }, [isOpen, telegramConfigProp]);

    const [mountedTabs, setMountedTabs] = useState<Set<TabId>>(() => new Set([activeTab]));

    // Persist active tab selection
    useEffect(() => {
        localStorage.setItem("kokoro_settings_active_tab", activeTab);
    }, [activeTab]);

    // Keep visited tabs mounted to avoid remount flicker/reload on tab switch
    useEffect(() => {
        setMountedTabs(prev => {
            if (prev.has(activeTab)) return prev;
            const next = new Set(prev);
            next.add(activeTab);
            return next;
        });
    }, [activeTab]);

    // Load Telegram config only when panel first opens, not on every bg.config change
    useEffect(() => {
        if (!isOpen) return;
        if (telegramConfigProp) {
            setLocalTelegramConfig(telegramConfigProp);
            return;
        }
        setLocalTelegramConfig(null); // reset so fetchTelegramConfig always loads fresh
        fetchTelegramConfig();
    }, [isOpen, telegramConfigProp]);

    // Update local BG config helper
    const updateBgConfig = (update: Partial<BackgroundConfig>) => {
        setLocalBgConfig(prev => ({ ...prev, ...update }));
    };


    // Persona state
    const [persona, setPersonaText] = useState(() => localStorage.getItem("kokoro_persona") || "You are a friendly, warm companion character. Respond with personality and emotion.");

    // TTS state
    const [ttsVoice, setTtsVoice] = useState(() => localStorage.getItem("kokoro_tts_voice") || "");
    const [ttsSpeed, setTtsSpeed] = useState(() => localStorage.getItem("kokoro_tts_speed") || "1.0");
    const [ttsPitch, setTtsPitch] = useState(() => localStorage.getItem("kokoro_tts_pitch") || "1.0");
    const [ttsProviderId, setTtsProviderId] = useState(() => localStorage.getItem("kokoro_tts_provider") || "browser");
    const [ttsEnabled, setTtsEnabled] = useState(() => localStorage.getItem("kokoro_tts_enabled") === "true");
    const [ttsProviders, setTtsProviders] = useState<ProviderStatus[]>([]);
    const [ttsVoices, setTtsVoices] = useState<VoiceProfile[]>([]);
    const [isTtsLoading, setIsTtsLoading] = useState(false);
    const [localTtsConfig, setLocalTtsConfig] = useState<TtsSystemConfig | null>(null);

    // Image Gen State — initialize from prop to avoid IPC fetch on every open
    const [localImageGenConfig, setLocalImageGenConfig] = useState<ImageGenSystemConfig | null>(imageGenConfigProp ?? null);

    // Keep local imagegen config synced when App-side preload arrives later.
    useEffect(() => {
        if (imageGenConfigProp === undefined) return;
        setLocalImageGenConfig(imageGenConfigProp ?? null);
    }, [imageGenConfigProp]);

    // Vision Mode
    const [visionEnabled, setVisionEnabled] = useState(() => localStorage.getItem("kokoro_vision_enabled") === "true");

    // Save feedback
    const [saved, setSaved] = useState(false);

    // STT state
    const [localSttConfig, setLocalSttConfig] = useState<SttConfig | null>(sttConfigProp ?? null);
    const [voiceInterrupt, setVoiceInterrupt] = useState(() => localStorage.getItem("kokoro_voice_interrupt") === "true");

    // Telegram config state
    const [localTelegramConfig, setLocalTelegramConfig] = useState<TelegramConfig | null>(null);

    // Response Language
    const [responseLang, setResponseLang] = useState(() => localStorage.getItem("kokoro_response_language") || "");

    // User Language (for translation)
    const [userLang, setUserLang] = useState(() => localStorage.getItem("kokoro_user_language") || "");

    // Click outside to close
    useEffect(() => {
        const handleClick = (e: MouseEvent) => {
            if (overlayRef.current && e.target === overlayRef.current) {
                onClose();
            }
        };
        if (isOpen) {
            document.addEventListener("mousedown", handleClick);
        }
        return () => document.removeEventListener("mousedown", handleClick);
    }, [isOpen, onClose]);

    // Escape to close
    useEffect(() => {
        const handleKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
        };
        if (isOpen) {
            document.addEventListener("keydown", handleKey);
        }
        return () => document.removeEventListener("keydown", handleKey);
    }, [isOpen, onClose]);

    const fetchData = async () => {
        setIsTtsLoading(true);
        try {
            const [providers, voices, ttsConfig] = await Promise.all([
                listTtsProviders(),
                listTtsVoices(),
                getTtsConfig(),
            ]);
            setTtsProviders(providers);
            setTtsVoices(voices);
            setLocalTtsConfig(ttsConfig);
            const sttConfig = await getSttConfig();
            setLocalSttConfig(sttConfig);
        } catch (e) {
            console.error("[SettingsPanel] Failed to fetch data:", e);
        } finally {
            setIsTtsLoading(false);
        }
    };

    const fetchTelegramConfig = async () => {
        try {
            const telegramConfig = await getTelegramConfig();
            setLocalTelegramConfig(telegramConfig);
        } catch (e) {
            console.error("[SettingsPanel] Failed to fetch telegram config:", e);
        }
    };

    useEffect(() => {
        if (ttsVoices.length === 0) return;
        setTtsVoice(prev => normalizeTtsVoice(ttsProviderId, prev, ttsVoices, localTtsConfig));
    }, [ttsProviderId, ttsVoices, localTtsConfig]);

    const handleSave = async () => {
        // Persist to localStorage (non-LLM settings)
        localStorage.setItem("kokoro_persona", persona);
        localStorage.setItem("kokoro_tts_voice", ttsVoice);
        localStorage.setItem("kokoro_tts_speed", ttsSpeed);
        localStorage.setItem("kokoro_tts_pitch", ttsPitch);
        localStorage.setItem("kokoro_tts_provider", ttsProviderId);
        localStorage.setItem("kokoro_tts_enabled", ttsEnabled ? "true" : "false");
        localStorage.setItem("kokoro_vision_enabled", visionEnabled ? "true" : "false");
        if (localSttConfig) {
            const activeSttProvider = localSttConfig.providers?.find(p => p.id === localSttConfig.active_provider);
            localStorage.setItem("kokoro_stt_enabled", activeSttProvider?.enabled ? "true" : "false");
            localStorage.setItem("kokoro_stt_auto_send", localSttConfig.auto_send ? "true" : "false");
            localStorage.setItem("kokoro_stt_language", localSttConfig.language || "");
            localStorage.setItem("kokoro_stt_continuous_listening", localSttConfig.continuous_listening ? "true" : "false");
            localStorage.setItem("kokoro_wake_word_enabled", localSttConfig.wake_word_enabled ? "true" : "false");
            localStorage.setItem("kokoro_wake_word", localSttConfig.wake_word || "");
        }
        window.dispatchEvent(new Event("kokoro-stt-settings-changed"));
        localStorage.setItem("kokoro_voice_interrupt", voiceInterrupt ? "true" : "false");
        localStorage.setItem("kokoro_response_language", responseLang);
        localStorage.setItem("kokoro_user_language", userLang);

        // Commit core settings
        onDisplayModeChange(localDisplayMode);
        onCustomModelChange(localCustomModelPath);
        onGazeTrackingChange?.(localGazeTracking);

        // Commit background config
        bg.setConfig(localBgConfig);

        // Send persona to backend
        try {
            await setPersona(persona);
        } catch (e) {
            console.error("[SettingsPanel] Failed to set persona:", e);
        }

        // Send response language to backend
        try {
            await setResponseLanguage(responseLang);
        } catch (e) {
            console.error("[SettingsPanel] Failed to set response language:", e);
        }

        // Send user language to backend
        try {
            await setUserLanguage(userLang);
        } catch (e) {
            console.error("[SettingsPanel] Failed to set user language:", e);
        }

        // Persist TTS Config
        if (localTtsConfig) {
            const ttsConfigToSave: TtsSystemConfig = {
                ...localTtsConfig,
                providers: localTtsConfig.providers.map((provider) => {
                    if (
                        provider.id === ttsProviderId
                        && (provider.provider_type === "openai" || provider.provider_type === "edge_tts")
                    ) {
                        return {
                            ...provider,
                            default_voice: ttsVoice || null,
                        };
                    }
                    return provider;
                }),
            };

            try {
                await saveTtsConfig(ttsConfigToSave);
                setLocalTtsConfig(ttsConfigToSave);
                // Refresh provider status after saving config
                const [providers, voices] = await Promise.all([
                    listTtsProviders(),
                    listTtsVoices(),
                ]);
                setTtsProviders(providers);
                setTtsVoices(voices);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save TTS config:", e);
            }
        }

        // Commit Image Gen Config
        if (localImageGenConfig) {
            try {
                await saveImageGenConfig(localImageGenConfig);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save Image Gen config:", e);
            }
        }

        // Commit STT Config
        if (localSttConfig) {
            try {
                await saveSttConfig(localSttConfig);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save STT config:", e);
            }
        }

        // Commit Telegram Config
        if (localTelegramConfig) {
            try {
                await saveTelegramConfig(localTelegramConfig);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save Telegram config:", e);
            }
        }

        // Commit LLM Config (if ApiTab has unsaved changes)
        if (latestLlmConfigRef.current) {
            try {
                await saveLlmConfig(latestLlmConfigRef.current);
                onLlmConfigSaved?.(latestLlmConfigRef.current);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save LLM config:", e);
            }
        }

        setSaved(true);
        setTimeout(() => setSaved(false), 2000);
    };

    const handleCancel = () => {
        onClose();
    };

    return (
        <AnimatePresence>
            {isOpen && (
                <motion.div
                    ref={overlayRef}
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.2 }}
                    className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50 backdrop-blur-sm"
                    style={{ pointerEvents: "auto" }}
                >
                    <motion.div
                        initial={{ opacity: 0, scale: 0.95, y: 20 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        exit={{ opacity: 0, scale: 0.95, y: 20 }}
                        transition={{ type: "spring", stiffness: 300, damping: 30 }}
                        className={clsx(
                            "w-[min(640px,90vw)] h-[min(80vh,700px)]",
                            "flex flex-col",
                            "bg-[var(--color-bg-elevated)] backdrop-blur-2xl",
                            "border border-[var(--color-border)] rounded-xl shadow-lg overflow-hidden"
                        )}
                    >
                        {/* Header */}
                        <div className="flex items-center justify-between p-5 border-b border-[var(--color-border)]">
                            <h2 className="font-heading text-lg font-bold tracking-widest uppercase text-[var(--color-accent)] drop-shadow-[var(--glow-accent)]">
                                {t("settings.title")}
                            </h2>
                            <motion.button
                                whileHover={{ scale: 1.1 }}
                                whileTap={{ scale: 0.95 }}
                                onClick={onClose}
                                className="p-2 rounded-md text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] transition-colors"
                                aria-label="Close settings"
                            >
                                <X size={18} strokeWidth={1.5} />
                            </motion.button>
                        </div>

                        {/* Tabs */}
                        {/* Tabs */}
                        <div className="border-b border-[var(--color-border)] bg-[var(--color-bg-surface-soft)]/50">
                            <div className="flex flex-wrap gap-1 p-2">
                                {tabs.map(({ id, label, icon: Icon }) => (
                                    <button
                                        key={id}
                                        onClick={() => setActiveTab(id)}
                                        className={clsx(
                                            "flex items-center gap-2 px-3 py-2 text-[11px] font-heading font-semibold tracking-wider uppercase transition-all rounded-md flex-grow justify-center",
                                            activeTab === id
                                                ? "bg-[var(--color-bg-elevated)] text-[var(--color-accent)] shadow-sm border border-[var(--color-border)]"
                                                : "text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-surface-soft)] border border-transparent"
                                        )}
                                    >
                                        <Icon size={14} strokeWidth={1.5} />
                                        {t(label)}
                                    </button>
                                ))}
                            </div>
                        </div>

                        {/* Content */}
                        <div className="flex-1 overflow-y-auto p-5 space-y-5 scrollable">
                            {mountedTabs.has("api") && (
                                <div className={activeTab === "api" ? "block" : "hidden"}>
                                    <ApiTab
                                        visionEnabled={visionEnabled}
                                        onVisionEnabledChange={setVisionEnabled}
                                        initialConfig={llmConfigProp ?? null}
                                        onConfigSaved={(cfg) => {
                                            latestLlmConfigRef.current = cfg;
                                            onLlmConfigSaved?.(cfg);
                                        }}
                                        onConfigChange={(cfg) => { latestLlmConfigRef.current = cfg; }}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("persona") && (
                                <div className={activeTab === "persona" ? "block" : "hidden"}>
                                    <CharacterManager
                                        onPersonaChange={(prompt) => setPersonaText(prompt)}
                                        responseLanguage={responseLang}
                                        onResponseLanguageChange={setResponseLang}
                                        userLanguage={userLang}
                                        onUserLanguageChange={setUserLang}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("memory") && (
                                <div className={activeTab === "memory" ? "block" : "hidden"}>
                                    <MemoryPanel
                                        characterId={localStorage.getItem("kokoro_active_character_id") || "default"}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("tts") && (
                                <div className={activeTab === "tts" ? "block" : "hidden"}>
                                    <TtsTab
                                        ttsConfig={localTtsConfig}
                                        onTtsConfigChange={setLocalTtsConfig}
                                        providers={ttsProviders}
                                        voices={ttsVoices}
                                        isTtsLoading={isTtsLoading}
                                        onRefresh={fetchData}
                                        ttsEnabled={ttsEnabled}
                                        onTtsEnabledChange={setTtsEnabled}
                                        ttsProviderId={ttsProviderId}
                                        onTtsProviderIdChange={setTtsProviderId}
                                        ttsVoice={ttsVoice}
                                        onTtsVoiceChange={setTtsVoice}
                                        ttsSpeed={ttsSpeed}
                                        onTtsSpeedChange={setTtsSpeed}
                                        ttsPitch={ttsPitch}
                                        onTtsPitchChange={setTtsPitch}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("stt") && localSttConfig && (
                                <div className={activeTab === "stt" ? "block" : "hidden"}>
                                    <SttTab
                                        sttConfig={localSttConfig}
                                        onSttConfigChange={setLocalSttConfig}
                                        voiceInterrupt={voiceInterrupt}
                                        onVoiceInterruptChange={setVoiceInterrupt}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("sing") && (
                                <div className={activeTab === "sing" ? "block" : "hidden"}>
                                    <SingTab />
                                </div>
                            )}

                            {mountedTabs.has("model") && (
                                <div className={activeTab === "model" ? "block" : "hidden"}>
                                    <ModelTab
                                        displayMode={localDisplayMode}
                                        onDisplayModeChange={setLocalDisplayMode}
                                        customModelPath={localCustomModelPath}
                                        onCustomModelPathChange={setLocalCustomModelPath}
                                        gazeTracking={localGazeTracking}
                                        onGazeTrackingChange={setLocalGazeTracking}
                                        renderFps={renderFps}
                                        onRenderFpsChange={onRenderFpsChange}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("imagegen") && localImageGenConfig && (
                                <div className={activeTab === "imagegen" ? "block" : "hidden"}>
                                    <ImageGenSettings
                                        config={localImageGenConfig}
                                        onChange={setLocalImageGenConfig}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("mods") && (
                                <div className={clsx(activeTab === "mods" ? "block" : "hidden", "h-[400px]")}>
                                    <ModList />
                                </div>
                            )}

                            {mountedTabs.has("bg") && (
                                <div className={activeTab === "bg" ? "block" : "hidden"}>
                                    <BackgroundTab
                                        bgConfig={localBgConfig}
                                        onBgConfigChange={updateBgConfig}
                                        backgroundControls={bg}
                                    />
                                </div>
                            )}

                            {mountedTabs.has("vision") && (
                                <div className={activeTab === "vision" ? "block" : "hidden"}>
                                    <VisionTab
                                        initialConfig={visionConfigProp ?? null}
                                        onConfigChange={onVisionConfigChange}
                                    />
                                </div>
                            )}
                            {mountedTabs.has("mcp") && (
                                <div className={activeTab === "mcp" ? "block" : "hidden"}>
                                    <McpTab initialServers={mcpServersProp} />
                                </div>
                            )}
                            {mountedTabs.has("telegram") && (
                                <div className={activeTab === "telegram" ? "block" : "hidden"}>
                                    <TelegramTab
                                        config={localTelegramConfig}
                                        initialStatus={initialTelegramStatus}
                                        initialCharacters={charactersProp}
                                        onUpdate={(patch) => setLocalTelegramConfig(prev => prev ? { ...prev, ...patch } : prev)}
                                    />
                                </div>
                            )}
                            {mountedTabs.has("jailbreak") && (
                                <div className={activeTab === "jailbreak" ? "block" : "hidden"}>
                                    <JailbreakTab />
                                </div>
                            )}
                            {mountedTabs.has("backup") && (
                                <div className={activeTab === "backup" ? "block" : "hidden"}>
                                    <BackupTab />
                                </div>
                            )}
                            {mountedTabs.has("pet") && (
                                <div className={activeTab === "pet" ? "block" : "hidden"}>
                                    <PetTab />
                                </div>
                            )}
                        </div>

                        {/* General Settings (Language) & Footer */}
                        <div className="flex items-center justify-between p-5 border-t border-[var(--color-border)]">
                            <div className="flex items-center gap-3">
                                <div className="text-xs text-[var(--color-text-secondary)] uppercase tracking-wider font-heading font-semibold">
                                    {t("settings.app_language.label")}
                                </div>
                                <Select
                                    value={i18n.language.split("-")[0]}
                                    onChange={(v) => {
                                        i18n.changeLanguage(v);
                                        localStorage.setItem("kokoro_app_language", v);
                                    }}
                                    options={[
                                        { value: "en", label: "English" },
                                        { value: "zh", label: "中文" },
                                        { value: "ja", label: "日本語" },
                                        { value: "ko", label: "한국어" },
                                        { value: "ru", label: "Русский" },
                                    ]}
                                    className="min-w-[120px]"
                                />
                            </div>

                            <div className="flex items-center gap-3">
                                <motion.button
                                    whileHover={{ scale: 1.02 }}
                                    whileTap={{ scale: 0.98 }}
                                    onClick={handleCancel}
                                    className={clsx(
                                        "px-4 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                                        "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                                    )}
                                >
                                    {t("common.actions.cancel")}
                                </motion.button>
                                <motion.button
                                    whileHover={{ scale: 1.05 }}
                                    whileTap={{ scale: 0.95 }}
                                    onClick={handleSave}
                                    className={clsx(
                                        "flex items-center gap-2 px-5 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                                        "bg-[var(--color-accent)] text-black",
                                        "hover:bg-white transition-colors"
                                    )}
                                >
                                    {saved ? <Check size={16} strokeWidth={2} /> : <Save size={16} strokeWidth={1.5} />}
                                    {saved ? t("common.actions.saved") : t("common.actions.save")}
                                </motion.button>
                            </div>
                        </div>
                    </motion.div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
