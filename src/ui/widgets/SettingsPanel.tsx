import { useState, useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { X, Key, User, Volume2, Package, Image, PersonStanding, Save, Check, Sparkles, Brain, Mic, Music, Eye, Server } from "lucide-react";
import { ModList } from "../mods/ModList";
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
import { useTranslation } from "react-i18next";
import { setPersona, setResponseLanguage, setUserLanguage, listTtsProviders, listTtsVoices, getTtsConfig, saveTtsConfig, getImageGenConfig, saveImageGenConfig, getSttConfig, saveSttConfig } from "../../lib/kokoro-bridge";
import type { ProviderStatus, VoiceProfile, TtsSystemConfig, ImageGenSystemConfig, SttConfig } from "../../lib/kokoro-bridge";
import type { BackgroundConfig } from "../hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "../../features/live2d/Live2DViewer";

type TabId = "api" | "persona" | "tts" | "stt" | "sing" | "mods" | "bg" | "model" | "imagegen" | "memory" | "vision" | "mcp";

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
}

const tabs: { id: TabId; label: string; icon: typeof Key }[] = [
    { id: "bg", label: "settings.tabs.bg", icon: Image },
    { id: "model", label: "settings.tabs.model", icon: PersonStanding },
    { id: "imagegen", label: "settings.tabs.imagegen", icon: Sparkles },
    { id: "api", label: "settings.tabs.api", icon: Key },
    { id: "persona", label: "settings.tabs.persona", icon: User },
    { id: "tts", label: "settings.tabs.tts", icon: Volume2 },
    { id: "stt", label: "settings.tabs.stt", icon: Mic },
    { id: "sing", label: "settings.tabs.sing", icon: Music },
    { id: "memory", label: "settings.tabs.memory", icon: Brain },
    { id: "vision", label: "settings.tabs.vision", icon: Eye },
    { id: "mods", label: "settings.tabs.mods", icon: Package },
    { id: "mcp", label: "settings.tabs.mcp", icon: Server },
];

export default function SettingsPanel({ isOpen, onClose, backgroundControls, displayMode, onDisplayModeChange, customModelPath, onCustomModelChange }: SettingsPanelProps) {
    const { t, i18n } = useTranslation();
    const [activeTab, setActiveTab] = useState<TabId>("bg");
    const bg = backgroundControls;
    const overlayRef = useRef<HTMLDivElement>(null);

    // ── Local Buffer State ───────────────────────────────
    // We hold changes locally until "Save" is clicked.

    // Display & Model
    const [localDisplayMode, setLocalDisplayMode] = useState(displayMode);
    const [localCustomModelPath, setLocalCustomModelPath] = useState(customModelPath);

    // Background Config
    const [localBgConfig, setLocalBgConfig] = useState<BackgroundConfig>({ ...bg.config });

    // Sync local state when panel opens or props change
    useEffect(() => {
        if (isOpen) {
            setLocalDisplayMode(displayMode);
            setLocalCustomModelPath(customModelPath);
            setLocalBgConfig({ ...bg.config });
            setPersonaText(localStorage.getItem("kokoro_persona") || "You are a friendly, warm companion character. Respond with personality and emotion.");
            setTtsVoice(localStorage.getItem("kokoro_tts_voice") || "alloy");
            setTtsSpeed(localStorage.getItem("kokoro_tts_speed") || "1.0");
            setTtsPitch(localStorage.getItem("kokoro_tts_pitch") || "1.0");
            setTtsProviderId(localStorage.getItem("kokoro_tts_provider") || "browser");
            setTtsEnabled(localStorage.getItem("kokoro_tts_enabled") === "true");
            setVisionEnabled(localStorage.getItem("kokoro_vision_enabled") === "true");
            setSttEnabled(localStorage.getItem("kokoro_stt_enabled") === "true");
            setSttAutoSend(localStorage.getItem("kokoro_stt_auto_send") === "true");
            setSttLanguage(localStorage.getItem("kokoro_stt_language") || "");
            setVoiceInterrupt(localStorage.getItem("kokoro_voice_interrupt") === "true");
            setResponseLang(localStorage.getItem("kokoro_response_language") || "");
            setUserLang(localStorage.getItem("kokoro_user_language") || "");
            // Fetch backend data
            fetchData();
        }
    }, [isOpen, displayMode, customModelPath, bg.config]);

    // Update local BG config helper
    const updateBgConfig = (update: Partial<BackgroundConfig>) => {
        setLocalBgConfig(prev => ({ ...prev, ...update }));
    };


    // Persona state
    const [persona, setPersonaText] = useState(() => localStorage.getItem("kokoro_persona") || "You are a friendly, warm companion character. Respond with personality and emotion.");

    // TTS state
    const [ttsVoice, setTtsVoice] = useState(() => localStorage.getItem("kokoro_tts_voice") || "alloy");
    const [ttsSpeed, setTtsSpeed] = useState(() => localStorage.getItem("kokoro_tts_speed") || "1.0");
    const [ttsPitch, setTtsPitch] = useState(() => localStorage.getItem("kokoro_tts_pitch") || "1.0");
    const [ttsProviderId, setTtsProviderId] = useState(() => localStorage.getItem("kokoro_tts_provider") || "browser");
    const [ttsEnabled, setTtsEnabled] = useState(() => localStorage.getItem("kokoro_tts_enabled") === "true");
    const [ttsProviders, setTtsProviders] = useState<ProviderStatus[]>([]);
    const [ttsVoices, setTtsVoices] = useState<VoiceProfile[]>([]);
    const [isTtsLoading, setIsTtsLoading] = useState(false);
    const [localTtsConfig, setLocalTtsConfig] = useState<TtsSystemConfig | null>(null);

    // Image Gen State
    const [localImageGenConfig, setLocalImageGenConfig] = useState<ImageGenSystemConfig | null>(null);

    // Vision Mode
    const [visionEnabled, setVisionEnabled] = useState(() => localStorage.getItem("kokoro_vision_enabled") === "true");

    // Save feedback
    const [saved, setSaved] = useState(false);

    // STT state
    const [localSttConfig, setLocalSttConfig] = useState<SttConfig | null>(null);
    const [sttEnabled, setSttEnabled] = useState(() => localStorage.getItem("kokoro_stt_enabled") === "true"); // Keeping for now as UI toggle often wraps config
    const [sttAutoSend, setSttAutoSend] = useState(() => localStorage.getItem("kokoro_stt_auto_send") === "true");
    const [sttLanguage, setSttLanguage] = useState(() => localStorage.getItem("kokoro_stt_language") || "");
    const [voiceInterrupt, setVoiceInterrupt] = useState(() => localStorage.getItem("kokoro_voice_interrupt") === "true");

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
            const [providers, voices, ttsConfig, imageGenConfig, sttConfig] = await Promise.all([
                listTtsProviders(),
                listTtsVoices(),
                getTtsConfig(),
                getImageGenConfig(),
                getSttConfig(),
            ]);
            setTtsProviders(providers);
            setTtsVoices(voices);
            setLocalTtsConfig(ttsConfig);
            setLocalImageGenConfig(imageGenConfig);
            setLocalSttConfig(sttConfig);
        } catch (e) {
            console.error("[SettingsPanel] Failed to fetch data:", e);
        } finally {
            setIsTtsLoading(false);
        }
    };

    const handleSave = async () => {
        // Persist to localStorage (non-LLM settings)
        localStorage.setItem("kokoro_persona", persona);
        localStorage.setItem("kokoro_tts_voice", ttsVoice);
        localStorage.setItem("kokoro_tts_speed", ttsSpeed);
        localStorage.setItem("kokoro_tts_pitch", ttsPitch);
        localStorage.setItem("kokoro_tts_provider", ttsProviderId);
        localStorage.setItem("kokoro_tts_enabled", ttsEnabled ? "true" : "false");
        localStorage.setItem("kokoro_vision_enabled", visionEnabled ? "true" : "false");
        localStorage.setItem("kokoro_stt_enabled", sttEnabled ? "true" : "false");
        localStorage.setItem("kokoro_stt_auto_send", sttAutoSend ? "true" : "false");
        localStorage.setItem("kokoro_stt_language", sttLanguage);
        localStorage.setItem("kokoro_voice_interrupt", voiceInterrupt ? "true" : "false");
        localStorage.setItem("kokoro_response_language", responseLang);
        localStorage.setItem("kokoro_user_language", userLang);

        // Commit core settings
        onDisplayModeChange(localDisplayMode);
        onCustomModelChange(localCustomModelPath);

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
            try {
                await saveTtsConfig(localTtsConfig);
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
            // Sync local storage states into config before saving if needed,
            // or assume SttTab writes directly to localSttConfig.
            // Let's ensure top-level toggles are synced if we are still using them.
            // But SttTab will likely modify localSttConfig directly now.
            // For now, let's just save what's in localSttConfig.
            try {
                await saveSttConfig(localSttConfig);
                // Also update local storage for any non-config legacy uses if any?
                // The implementation plan says migrate. So we should rely on config.
            } catch (e) {
                console.error("[SettingsPanel] Failed to save STT config:", e);
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
                            {activeTab === "api" && (
                                <ApiTab
                                    visionEnabled={visionEnabled}
                                    onVisionEnabledChange={setVisionEnabled}
                                />
                            )}

                            {activeTab === "persona" && (
                                <CharacterManager
                                    onPersonaChange={(prompt) => setPersonaText(prompt)}
                                    responseLanguage={responseLang}
                                    onResponseLanguageChange={setResponseLang}
                                    userLanguage={userLang}
                                    onUserLanguageChange={setUserLang}
                                />
                            )}

                            {activeTab === "memory" && (
                                <MemoryPanel
                                    characterId={localStorage.getItem("kokoro_active_character_id") || "default"}
                                />
                            )}

                            {activeTab === "tts" && (
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
                            )}

                            {activeTab === "stt" && localSttConfig && (
                                <SttTab
                                    sttConfig={localSttConfig}
                                    onSttConfigChange={setLocalSttConfig}
                                    voiceInterrupt={voiceInterrupt}
                                    onVoiceInterruptChange={setVoiceInterrupt}
                                />
                            )}

                            {activeTab === "sing" && (
                                <SingTab />
                            )}

                            {activeTab === "model" && (
                                <ModelTab
                                    displayMode={localDisplayMode}
                                    onDisplayModeChange={setLocalDisplayMode}
                                    customModelPath={localCustomModelPath}
                                    onCustomModelPathChange={setLocalCustomModelPath}
                                />
                            )}

                            {activeTab === "imagegen" && localImageGenConfig && (
                                <ImageGenSettings
                                    config={localImageGenConfig}
                                    onChange={setLocalImageGenConfig}
                                />
                            )}

                            {activeTab === "mods" && (
                                <div className="h-[400px]">
                                    <ModList />
                                </div>
                            )}

                            {activeTab === "bg" && (
                                <BackgroundTab
                                    bgConfig={localBgConfig}
                                    onBgConfigChange={updateBgConfig}
                                    backgroundControls={bg}
                                />
                            )}

                            {activeTab === "vision" && (
                                <VisionTab />
                            )}
                            {activeTab === "mcp" && (
                                <McpTab />
                            )}
                        </div>

                        {/* General Settings (Language) & Footer */}
                        <div className="flex items-center justify-between p-5 border-t border-[var(--color-border)]">
                            <div className="flex items-center gap-3">
                                <div className="text-xs text-[var(--color-text-secondary)] uppercase tracking-wider font-heading font-semibold">
                                    {t("settings.app_language.label")}
                                </div>
                                <select
                                    value={i18n.language}
                                    onChange={(e) => {
                                        i18n.changeLanguage(e.target.value);
                                        localStorage.setItem("kokoro_app_language", e.target.value);
                                    }}
                                    className="bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-primary)] text-xs rounded px-2 py-1 focus:outline-none focus:border-[var(--color-accent)]"
                                >
                                    <option value="en">English</option>
                                    <option value="zh">中文</option>
                                    <option value="ja">日本語</option>
                                    <option value="ko">한국어</option>
                                </select>
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
                                    Cancel
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
