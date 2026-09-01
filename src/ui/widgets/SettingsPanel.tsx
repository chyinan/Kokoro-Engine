// pattern: Imperative Shell

import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { X, Key, User, Volume2, Package, Image, PersonStanding, Save, Check, Sparkles, Brain, Mic, Eye, Server, Bot, Shield, HardDrive, Ghost, Info } from "lucide-react";
import { ModList } from "../mods/ModList";
import ContentLibrary from "./ContentLibrary";
import { Select } from "@/components/ui/select";
import CharacterManager, { type CharacterManagerRef } from "./CharacterManager";
import type { CharacterRuntimeOverrides } from "../../features/characters/character-runtime-overrides";
import ImageGenSettings from "./ImageGenSettings";
import MemoryPanel from "./MemoryPanel";
import ApiTab from "./settings/ApiTab";
import TtsTab from "./settings/TtsTab";
import SttTab from "./settings/SttTab";
import ModelTab from "./settings/ModelTab";
import BackgroundTab from "./settings/BackgroundTab";
import VisionTab from "./settings/VisionTab";
import McpTab from "./settings/McpTab";
import BotTab from "./settings/BotTab";
import { JailbreakTab } from "./settings/JailbreakTab";
import { BackupTab } from "./settings/BackupTab";
import PetTab from "./settings/PetTab";
import AboutTab from "./settings/AboutTab";
import { useTranslation } from "react-i18next";
import { setUserLanguage, listTtsProviders, listTtsVoices, getTtsConfig, saveTtsConfig, saveImageGenConfig, getSttConfig, saveSttConfig, getBotConfig, saveBotConfig, saveLlmConfig, saveVisionConfig } from "../../lib/kokoro-bridge";
import {
    isBackgroundConfigDirty,
    isBotConfigDirty,
    isImageGenConfigDirty,
    isLlmConfigDirty,
    isRuntimeDirty,
    isSttConfigDirty,
    isTtsConfigDirty,
    isTtsParamsDirty,
    isVisionConfigDirty,
    type TtsParamSnapshot,
} from "./settings-dirty-check";
import type {
    ProviderStatus,
    VoiceProfile,
    TtsSystemConfig,
    ImageGenSystemConfig,
    SttConfig,
    BotConfig,
    BotStatus,
    AutoBackupConfig,
    TelegramConfig,
    TelegramStatus,
    Live2dModelInfo,
    LlmConfig,
    VisionConfig,
    McpServerStatus,
    ModManifest,
    GptSovitsModels,
    MemoryRecord,
    CharacterRecord,
} from "../../lib/kokoro-bridge";
import { normalizeBackgroundConfigForImageCount, type BackgroundConfig } from "../hooks/useBackgroundSlideshow";
import type { Live2DDisplayMode } from "../../features/live2d/Live2DViewer";
import {
    APP_SETTING_KEYS,
    dispatchRuntimeSettingsChanged,
    readBooleanSetting,
    readStringSetting,
    writeBooleanSetting,
    writeStringSetting,
} from "../../lib/app-settings";
import {
    SETTINGS_GROUPS,
    SETTINGS_TAB_IDS,
    getSettingsGroupForTab,
    type SettingsTabId,
} from "./settings/settings-groups";
export type { SettingsTabId } from "./settings/settings-groups";

const SETTINGS_TAB_ID_SET = new Set<string>(SETTINGS_TAB_IDS);

export function normalizeSettingsTabId(tab: string | null | undefined): SettingsTabId {
    if (tab === "telegram") {
        return "bot";
    }
    return tab && SETTINGS_TAB_ID_SET.has(tab) ? (tab as SettingsTabId) : "bg";
}

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
    activeTab?: SettingsTabId;
    onActiveTabChange?: (tab: SettingsTabId) => void;
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
    responseLanguage?: string;
    ttsConfig?: TtsSystemConfig;
    llmConfig?: LlmConfig;
    onLlmConfigSaved?: (cfg: LlmConfig) => void;
    sttConfig?: SttConfig;
    visionConfig?: VisionConfig;
    onVisionConfigChange?: (cfg: VisionConfig) => void;
    imageGenConfig?: ImageGenSystemConfig;
    telegramConfig?: TelegramConfig;
    botConfig?: BotConfig;
    botStatus?: BotStatus;
    autoBackupConfig?: AutoBackupConfig;
    backupStatus?: unknown;
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
    sdModels?: string[];
    capturedScreenUrl?: string | null;
    userLanguage?: string;
    activeCharacterId?: string;
    characterToEditId?: string | null;
    onActivateCharacter: (characterId: string) => Promise<void>;
    onCharacterRuntimeChange: (overrides: Readonly<CharacterRuntimeOverrides>) => Promise<void>;
    onCharactersChanged?: (characters: ReadonlyArray<CharacterRecord>) => void;
    characters?: CharacterRecord[];
    resolveAvatarUrl?: (path: string) => string;
    // User Profile
    userName?: string;
    userPersona?: string;
    proactiveEnabled?: boolean;
    initialTelegramStatus?: TelegramStatus | null;
}

const tabs: { id: SettingsTabId; label: string; icon: typeof Key }[] = [
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
    { id: "bot", label: "settings.tabs.bot", icon: Bot },
    // 系统 / 高级
    { id: "api", label: "settings.tabs.api", icon: Key },
    { id: "jailbreak", label: "settings.tabs.jailbreak", icon: Shield },
    { id: "pet", label: "settings.tabs.pet", icon: Ghost },
    { id: "backup", label: "settings.tabs.backup", icon: HardDrive },
    { id: "about", label: "settings.tabs.about", icon: Info },
];

const APP_LANGUAGE_OPTIONS = [
    { value: "en", label: "English" },
    { value: "zh", label: "简体中文" },
    { value: "zh-TW", label: "繁體中文" },
    { value: "ja", label: "日本語" },
    { value: "ko", label: "한국어" },
    { value: "ru", label: "Русский" },
];

function getAppLanguageSelectValue(language: string | undefined) {
    const normalized = language?.trim().toLowerCase() ?? "";
    if (
        normalized.startsWith("zh-tw") ||
        normalized.startsWith("zh-hant") ||
        normalized.startsWith("zh-hk") ||
        normalized.startsWith("zh-mo")
    ) {
        return "zh-TW";
    }

    const base = normalized.split("-")[0];
    return APP_LANGUAGE_OPTIONS.some(option => option.value === base) ? base : "en";
}

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

function isReferenceCloneTtsProvider(providerId: string, ttsConfig?: TtsSystemConfig | null): boolean {
    const provider = ttsConfig?.providers.find(p => p.id === providerId);
    return provider?.provider_type === "gpt_sovits" || provider?.provider_type === "omnivoice";
}

function normalizeTtsVoice(
    providerId: string,
    voice: string,
    voices: VoiceProfile[],
    ttsConfig?: TtsSystemConfig | null,
): string {
    if (isReferenceCloneTtsProvider(providerId, ttsConfig)) {
        return "";
    }

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

export default function SettingsPanel({ isOpen, onClose, activeTab: activeTabProp, onActiveTabChange, backgroundControls, displayMode, onDisplayModeChange, customModelPath, onCustomModelChange: _onCustomModelChange, gazeTracking: gazeTrackingProp, onGazeTrackingChange, renderFps, onRenderFpsChange, sttConfig: sttConfigProp, voiceInterrupt: _voiceInterruptProp, imageGenConfig: imageGenConfigProp, llmConfig: llmConfigProp, onLlmConfigSaved, visionConfig: visionConfigProp, mcpServers: mcpServersProp, characters: charactersProp, resolveAvatarUrl, initialTelegramStatus, onVisionConfigChange, onActivateCharacter, onCharacterRuntimeChange, onCharactersChanged, characterToEditId, activeCharacterId }: SettingsPanelProps) {
    const { t, i18n } = useTranslation();
    const [internalActiveTab, setInternalActiveTab] = useState<SettingsTabId>(() => {
        const saved = readStringSetting(APP_SETTING_KEYS.settingsActiveTab, "");
        return normalizeSettingsTabId(saved);
    });
    const activeTab = activeTabProp ?? internalActiveTab;
    const handleActiveTabChange = (tab: SettingsTabId) => {
        if (activeTabProp === undefined) {
            setInternalActiveTab(tab);
        }
        onActiveTabChange?.(tab);
    };
    const bg = backgroundControls;
    const overlayRef = useRef<HTMLDivElement>(null);
    const latestLlmConfigRef = useRef<LlmConfig | null>(llmConfigProp ?? null);
    const bgConfigDirtyRef = useRef(false);
    const characterManagerRef = useRef<CharacterManagerRef | null>(null);

    // ── Local Buffer State ───────────────────────────────
    // We hold changes locally until "Save" is clicked.

    // Display & Model
    const [localDisplayMode, setLocalDisplayMode] = useState(displayMode);
    const [localCustomModelPath, setLocalCustomModelPath] = useState(customModelPath);
    const [localGazeTracking, setLocalGazeTracking] = useState(gazeTrackingProp ?? true);

    // Background Config
    const [localBgConfig, setLocalBgConfig] = useState<BackgroundConfig>(() => ({
        ...normalizeBackgroundConfigForImageCount(bg.config, bg.imageCount),
    }));

    // Vision Config
    const [localVisionConfig, setLocalVisionConfig] = useState<VisionConfig | null>(visionConfigProp ?? null);

    // Baseline snapshots (recorded when opening Settings to support dirty checking and cancel reset)
    const baselineDisplayModeRef = useRef(displayMode);
    const baselineCustomModelPathRef = useRef(customModelPath);
    const baselineGazeTrackingRef = useRef(gazeTrackingProp ?? true);
    const baselineBgConfigRef = useRef(localBgConfig);
    const baselineTtsParamsRef = useRef<TtsParamSnapshot>({
        enabled: false,
        providerId: "browser",
        voice: "",
        speed: "1.0",
        pitch: "1.0",
    });
    const baselineTtsConfigRef = useRef<TtsSystemConfig | null>(null);
    const baselineSttConfigRef = useRef<SttConfig | null>(null);
    const baselineVoiceInterruptRef = useRef(false);
    const baselineImageGenConfigRef = useRef<ImageGenSystemConfig | null>(null);
    const baselineBotConfigRef = useRef<BotConfig | null>(null);
    const baselineLlmConfigRef = useRef<LlmConfig | null>(null);
    const baselineVisionConfigRef = useRef<VisionConfig | null>(null);
    const baselineVisionEnabledRef = useRef(false);
    const baselineUserLangRef = useRef("");
    const baselineResponseLangRef = useRef("");

    // Sync local state only when the panel opens; while editing, keep local form state authoritative.
    useEffect(() => {
        if (isOpen) {
            setLocalDisplayMode(displayMode);
            setLocalCustomModelPath(customModelPath);
            latestLlmConfigRef.current = llmConfigProp ?? null;
            setLocalGazeTracking(gazeTrackingProp ?? true);
            bgConfigDirtyRef.current = false;
            const currentBg = { ...normalizeBackgroundConfigForImageCount(bg.config, bg.imageCount) };
            setLocalBgConfig(currentBg);

            const voice = readStringSetting(APP_SETTING_KEYS.ttsVoice, "");
            const speed = readStringSetting(APP_SETTING_KEYS.ttsSpeed, "1.0");
            const pitch = readStringSetting(APP_SETTING_KEYS.ttsPitch, "1.0");
            const provider = readStringSetting(APP_SETTING_KEYS.ttsProvider, "browser");
            const enabled = readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false);
            const vision = readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false);
            const voiceInt = readBooleanSetting(APP_SETTING_KEYS.voiceInterrupt, false);
            const respLang = readStringSetting(APP_SETTING_KEYS.responseLanguage, "");
            const uLang = readStringSetting(APP_SETTING_KEYS.userLanguage, "");

            setTtsVoice(voice);
            setTtsSpeed(speed);
            setTtsPitch(pitch);
            setTtsProviderId(provider);
            setTtsEnabled(enabled);
            setVisionEnabled(vision);
            setVoiceInterrupt(voiceInt);
            setResponseLang(respLang);
            setUserLang(uLang);
            setLocalBotConfig(null);
            setLocalVisionConfig(visionConfigProp ?? null);

            // Record baseline snapshots
            baselineDisplayModeRef.current = displayMode;
            baselineCustomModelPathRef.current = customModelPath;
            baselineGazeTrackingRef.current = gazeTrackingProp ?? true;
            baselineBgConfigRef.current = currentBg;
            baselineTtsParamsRef.current = {
                enabled,
                providerId: provider,
                voice,
                speed,
                pitch,
            };
            baselineVisionEnabledRef.current = vision;
            baselineVoiceInterruptRef.current = voiceInt;
            baselineResponseLangRef.current = respLang;
            baselineUserLangRef.current = uLang;
            baselineImageGenConfigRef.current = imageGenConfigProp ?? null;
            baselineLlmConfigRef.current = llmConfigProp ?? null;
            baselineVisionConfigRef.current = visionConfigProp ?? null;

            fetchData();
            fetchBotConfig();
        }
    }, [isOpen]);

    useEffect(() => {
        if (!isOpen || bgConfigDirtyRef.current) return;
        setLocalBgConfig({ ...normalizeBackgroundConfigForImageCount(bg.config, bg.imageCount) });
    }, [isOpen, bg.config, bg.imageCount]);

    const [mountedTabs, setMountedTabs] = useState<Set<SettingsTabId>>(() => new Set([activeTab]));

    // Persist active tab selection
    useEffect(() => {
        writeStringSetting(APP_SETTING_KEYS.settingsActiveTab, activeTab);
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

    // Update local BG config helper
    const updateBgConfig = (update: Partial<BackgroundConfig>) => {
        bgConfigDirtyRef.current = true;
        setLocalBgConfig(prev => ({ ...prev, ...update }));
    };


    // TTS state
    const [ttsVoice, setTtsVoice] = useState(() => readStringSetting(APP_SETTING_KEYS.ttsVoice, ""));
    const [ttsSpeed, setTtsSpeed] = useState(() => readStringSetting(APP_SETTING_KEYS.ttsSpeed, "1.0"));
    const [ttsPitch, setTtsPitch] = useState(() => readStringSetting(APP_SETTING_KEYS.ttsPitch, "1.0"));
    const [ttsProviderId, setTtsProviderId] = useState(() =>
        readStringSetting(APP_SETTING_KEYS.ttsProvider, "browser")
    );
    const [ttsEnabled, setTtsEnabled] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false)
    );

    useEffect(() => {
        const syncCharacterRuntime = () => {
            setTtsVoice(readStringSetting(APP_SETTING_KEYS.ttsVoice, ""));
            setTtsSpeed(readStringSetting(APP_SETTING_KEYS.ttsSpeed, "1.0"));
            setTtsPitch(readStringSetting(APP_SETTING_KEYS.ttsPitch, "1.0"));
            setTtsProviderId(readStringSetting(APP_SETTING_KEYS.ttsProvider, "browser"));
            setTtsEnabled(readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false));
        };
        window.addEventListener("kokoro-character-runtime-changed", syncCharacterRuntime);
        return () => window.removeEventListener("kokoro-character-runtime-changed", syncCharacterRuntime);
    }, []);
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
    const [visionEnabled, setVisionEnabled] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false)
    );

    // Save feedback
    const [saved, setSaved] = useState(false);
    const savedTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // STT state
    const [localSttConfig, setLocalSttConfig] = useState<SttConfig | null>(sttConfigProp ?? null);
    const [voiceInterrupt, setVoiceInterrupt] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.voiceInterrupt, false)
    );

    // Bot config state
    const [localBotConfig, setLocalBotConfig] = useState<BotConfig | null>(null);

    useEffect(() => {
        let aborted = false;
        let cleanup: (() => void) | null = null;
        void listen<{
            approved_user_openid?: string;
            approved_group_openid?: string;
            revision: number;
        }>("qq-authorization-approved", event => {
            if (aborted) return;
            setLocalBotConfig(current => {
                if (!current) {
                    return current;
                }
                if (event.payload.revision <= current.revision) {
                    return current;
                }
                const approvedUser = event.payload.approved_user_openid;
                const approvedGroup = event.payload.approved_group_openid;
                return {
                    ...current,
                    revision: event.payload.revision,
                    qq: {
                        ...current.qq,
                        allowed_user_openids: !approvedUser || current.qq.allowed_user_openids.includes(approvedUser)
                            ? current.qq.allowed_user_openids
                            : [...current.qq.allowed_user_openids, approvedUser],
                        allowed_group_openids: !approvedGroup || current.qq.allowed_group_openids.includes(approvedGroup)
                            ? current.qq.allowed_group_openids
                            : [...current.qq.allowed_group_openids, approvedGroup],
                    },
                };
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

    // Response Language
    const [responseLang, setResponseLang] = useState(() =>
        readStringSetting(APP_SETTING_KEYS.responseLanguage, "")
    );

    // User Language (for translation)
    const [userLang, setUserLang] = useState(() =>
        readStringSetting(APP_SETTING_KEYS.userLanguage, "")
    );

    // Click outside to close
    useEffect(() => {
        const handleClick = (e: MouseEvent) => {
            if (overlayRef.current && e.target === overlayRef.current) {
                handleCancel();
            }
        };
        if (isOpen) {
            document.addEventListener("mousedown", handleClick);
        }
        return () => document.removeEventListener("mousedown", handleClick);
    }, [isOpen]);

    // Escape to close
    useEffect(() => {
        const handleKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") handleCancel();
        };
        if (isOpen) {
            document.addEventListener("keydown", handleKey);
        }
        return () => document.removeEventListener("keydown", handleKey);
    }, [isOpen]);

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
            baselineTtsConfigRef.current = ttsConfig;
            const sttConfig = await getSttConfig();
            setLocalSttConfig(sttConfig);
            baselineSttConfigRef.current = sttConfig;
        } catch (e) {
            console.error("[SettingsPanel] Failed to fetch data:", e);
        } finally {
            setIsTtsLoading(false);
        }
    };

    const fetchBotConfig = async () => {
        try {
            const botConfig = await getBotConfig();
            setLocalBotConfig(botConfig);
            baselineBotConfigRef.current = botConfig;
        } catch (e) {
            console.error("[SettingsPanel] Failed to fetch bot config:", e);
        }
    };

    useEffect(() => {
        if (ttsVoices.length === 0) return;
        setTtsVoice(prev => normalizeTtsVoice(ttsProviderId, prev, ttsVoices, localTtsConfig));
    }, [ttsProviderId, ttsVoices, localTtsConfig]);

    useEffect(() => {
        return () => {
            if (savedTimeoutRef.current) {
                clearTimeout(savedTimeoutRef.current);
            }
        };
    }, []);

    const showSaveFeedback = () => {
        if (savedTimeoutRef.current) {
            clearTimeout(savedTimeoutRef.current);
        }
        setSaved(true);
        savedTimeoutRef.current = setTimeout(() => {
            setSaved(false);
            savedTimeoutRef.current = null;
        }, 2000);
    };

    const handleSave = async () => {
        // 1. Commit Persona Draft
        const personaSaveResult = await characterManagerRef.current?.saveDraft();
        const personaDirty = personaSaveResult?.hasChanges ?? false;

        // 2. Commit Vision Settings & Config
        const visionEnabledDirty = visionEnabled !== baselineVisionEnabledRef.current;
        if (visionEnabledDirty) {
            writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, visionEnabled);
            dispatchRuntimeSettingsChanged("vision");
            baselineVisionEnabledRef.current = visionEnabled;
        }
        const visionConfigDirty = isVisionConfigDirty(baselineVisionConfigRef.current, localVisionConfig);
        if (visionConfigDirty && localVisionConfig) {
            try {
                await saveVisionConfig(localVisionConfig);
                baselineVisionConfigRef.current = { ...localVisionConfig };
                dispatchRuntimeSettingsChanged("vision");
            } catch (e) {
                console.error("[SettingsPanel] Failed to save Vision config:", e);
            }
        }

        // 3. Commit STT Settings & Config
        const sttDirty = isSttConfigDirty(
            baselineSttConfigRef.current,
            localSttConfig,
            baselineVoiceInterruptRef.current,
            voiceInterrupt,
        );
        if (sttDirty) {
            if (localSttConfig) {
                const activeSttProvider = localSttConfig.providers?.find(p => p.id === localSttConfig.active_provider);
                writeBooleanSetting(APP_SETTING_KEYS.sttEnabled, activeSttProvider?.enabled === true);
                writeBooleanSetting(APP_SETTING_KEYS.sttAutoSend, localSttConfig.auto_send);
                writeStringSetting(APP_SETTING_KEYS.sttLanguage, localSttConfig.language || "");
                writeBooleanSetting(APP_SETTING_KEYS.sttContinuousListening, localSttConfig.continuous_listening);
                writeBooleanSetting(APP_SETTING_KEYS.wakeWordEnabled, localSttConfig.wake_word_enabled);
                writeStringSetting(APP_SETTING_KEYS.wakeWord, localSttConfig.wake_word || "");
                try {
                    await saveSttConfig(localSttConfig);
                    baselineSttConfigRef.current = { ...localSttConfig };
                } catch (e) {
                    console.error("[SettingsPanel] Failed to save STT config:", e);
                }
            }
            writeBooleanSetting(APP_SETTING_KEYS.voiceInterrupt, voiceInterrupt);
            baselineVoiceInterruptRef.current = voiceInterrupt;
            dispatchRuntimeSettingsChanged("stt");
        }

        // 4. Commit Core Display & Gaze Tracking
        const displayModeDirty = localDisplayMode !== baselineDisplayModeRef.current;
        if (displayModeDirty) {
            onDisplayModeChange(localDisplayMode);
            baselineDisplayModeRef.current = localDisplayMode;
        }
        const gazeTrackingDirty = localGazeTracking !== baselineGazeTrackingRef.current;
        if (gazeTrackingDirty) {
            onGazeTrackingChange?.(localGazeTracking);
            baselineGazeTrackingRef.current = localGazeTracking;
        }

        // 5. Commit Background Config
        const bgDirty = bgConfigDirtyRef.current || isBackgroundConfigDirty(baselineBgConfigRef.current, localBgConfig);
        if (bgDirty) {
            bg.setConfig(localBgConfig);
            baselineBgConfigRef.current = { ...localBgConfig };
            bgConfigDirtyRef.current = false;
        }

        // 6. Commit User Language (Translation)
        const userLangDirty = userLang !== baselineUserLangRef.current;
        if (userLangDirty) {
            writeStringSetting(APP_SETTING_KEYS.userLanguage, userLang);
            baselineUserLangRef.current = userLang;
            try {
                await setUserLanguage(userLang);
            } catch (e) {
                console.error("[SettingsPanel] Failed to set user language:", e);
            }
        }

        // 7. Commit TTS System Config
        const currentTtsParams: TtsParamSnapshot = {
            enabled: ttsEnabled,
            providerId: ttsProviderId,
            voice: ttsVoice,
            speed: ttsSpeed,
            pitch: ttsPitch,
        };
        const ttsParamsDirty = isTtsParamsDirty(baselineTtsParamsRef.current, currentTtsParams);
        const ttsConfigDirty = isTtsConfigDirty(baselineTtsConfigRef.current, localTtsConfig);
        if (ttsConfigDirty && localTtsConfig) {
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
                baselineTtsConfigRef.current = ttsConfigToSave;
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
        if (ttsParamsDirty) {
            baselineTtsParamsRef.current = currentTtsParams;
        }

        // 8. Track Live2D Model & Response Language changes
        const modelDirty = localCustomModelPath !== baselineCustomModelPathRef.current;
        if (modelDirty) {
            baselineCustomModelPathRef.current = localCustomModelPath;
        }
        const responseLangDirty = responseLang !== baselineResponseLangRef.current;
        if (responseLangDirty) {
            baselineResponseLangRef.current = responseLang;
        }

        // 9. CONDITIONAL RUNTIME RELOAD:
        // Only trigger onCharacterRuntimeChange if one of the runtime-sensitive fields changed!
        const runtimeDirty = isRuntimeDirty({
            personaDirty,
            ttsDirty: ttsParamsDirty,
            modelDirty,
            responseLangDirty,
        });

        if (runtimeDirty) {
            const selectedTtsProvider = localTtsConfig?.providers.find(
                (provider) => provider.id === ttsProviderId,
            ) ?? null;
            try {
                await onCharacterRuntimeChange({
                    ...(responseLangDirty ? { responseLanguage: responseLang } : {}),
                    ...(modelDirty ? { live2dModel: localCustomModelPath } : {}),
                    ...(personaSaveResult?.changedCharacter ? { persona: personaSaveResult.changedCharacter.persona } : {}),
                    ...(ttsParamsDirty ? {
                        tts: {
                            enabled: ttsEnabled,
                            providerId: ttsProviderId || null,
                            providerType: selectedTtsProvider?.provider_type ?? null,
                            voice: ttsVoice || null,
                            speed: Number.parseFloat(ttsSpeed) || 1,
                            pitch: Number.parseFloat(ttsPitch) || 1,
                        },
                    } : {}),
                });
            } catch (e) {
                console.error("[SettingsPanel] Failed to apply character runtime:", e);
            }
        }

        // 10. Commit Image Gen Config
        const imageGenDirty = isImageGenConfigDirty(baselineImageGenConfigRef.current, localImageGenConfig);
        if (imageGenDirty && localImageGenConfig) {
            try {
                await saveImageGenConfig(localImageGenConfig);
                baselineImageGenConfigRef.current = { ...localImageGenConfig };
            } catch (e) {
                console.error("[SettingsPanel] Failed to save Image Gen config:", e);
            }
        }

        // 11. Commit Bot Config
        const botDirty = isBotConfigDirty(baselineBotConfigRef.current, localBotConfig);
        if (botDirty && localBotConfig) {
            try {
                const savedBot = await saveBotConfig(localBotConfig);
                setLocalBotConfig(savedBot);
                baselineBotConfigRef.current = savedBot;
            } catch (e) {
                console.error("[SettingsPanel] Failed to save Bot config:", e);
            }
        }

        // 12. Commit LLM Config (if ApiTab has unsaved changes)
        const llmDirty = isLlmConfigDirty(baselineLlmConfigRef.current, latestLlmConfigRef.current);
        if (llmDirty && latestLlmConfigRef.current) {
            try {
                await saveLlmConfig(latestLlmConfigRef.current);
                baselineLlmConfigRef.current = latestLlmConfigRef.current;
                onLlmConfigSaved?.(latestLlmConfigRef.current);
            } catch (e) {
                console.error("[SettingsPanel] Failed to save LLM config:", e);
            }
        }

        showSaveFeedback();
    };

    const handleCancel = () => {
        characterManagerRef.current?.resetDraft();
        setLocalDisplayMode(baselineDisplayModeRef.current);
        setLocalCustomModelPath(baselineCustomModelPathRef.current);
        setLocalGazeTracking(baselineGazeTrackingRef.current);
        setLocalBgConfig(baselineBgConfigRef.current);
        bg.setConfig(baselineBgConfigRef.current);
        bgConfigDirtyRef.current = false;
        setUserLang(baselineUserLangRef.current);
        setResponseLang(baselineResponseLangRef.current);
        setTtsVoice(baselineTtsParamsRef.current.voice);
        setTtsSpeed(baselineTtsParamsRef.current.speed);
        setTtsPitch(baselineTtsParamsRef.current.pitch);
        setTtsProviderId(baselineTtsParamsRef.current.providerId);
        setTtsEnabled(baselineTtsParamsRef.current.enabled);
        setLocalTtsConfig(baselineTtsConfigRef.current);
        setLocalSttConfig(baselineSttConfigRef.current);
        setVoiceInterrupt(baselineVoiceInterruptRef.current);
        setLocalImageGenConfig(baselineImageGenConfigRef.current);
        setLocalBotConfig(baselineBotConfigRef.current);
        latestLlmConfigRef.current = baselineLlmConfigRef.current;
        setVisionEnabled(baselineVisionEnabledRef.current);
        setLocalVisionConfig(baselineVisionConfigRef.current);
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
                            <div className="flex items-center gap-2">
                                <motion.button
                                    initial={false}
                                    whileHover="hover"
                                    whileTap={{ scale: 0.97 }}
                                    transition={{ type: "spring", stiffness: 380, damping: 26 }}
                                    onClick={onClose}
                                    data-onboarding-id="settings-close-button"
                                    className="inline-flex h-9 w-9 items-center justify-center rounded-md text-[var(--color-text-secondary)] transition-[color,border-color,box-shadow,background-color] duration-200 ease-out hover:bg-[var(--color-accent)]/8 hover:text-[var(--color-accent)]"
                                    aria-label="Close settings"
                                >
                                    <motion.span
                                        variants={{ hover: { rotate: 10, scale: 1.03 } }}
                                        transition={{ type: "spring", stiffness: 420, damping: 24 }}
                                        className="flex items-center justify-center"
                                    >
                                        <X size={18} strokeWidth={1.5} />
                                    </motion.span>
                                </motion.button>
                            </div>
                        </div>

                        {/* Tabs */}
                        <div className="border-b border-[var(--color-border)] bg-[var(--color-bg-surface-soft)]/50">
                            <div className="space-y-2 p-2">
                                {Object.values(SETTINGS_GROUPS).map((group) => (
                                    <div key={group.id} data-settings-group={group.id}>
                                        <div className="flex flex-wrap gap-1">
                                            {group.tabs.map((id) => {
                                                const tab = tabs.find((candidate) => candidate.id === id);
                                                if (!tab) return null;
                                                const Icon = tab.icon;
                                                return (
                                                    <button
                                                        key={id}
                                                        onClick={() => handleActiveTabChange(id)}
                                                        data-settings-group-tab={getSettingsGroupForTab(id)}
                                                        data-onboarding-id={
                                                            id === "api"
                                                                ? "settings-tab-api"
                                                                : id === "persona"
                                                                    ? "settings-tab-persona"
                                                                    : undefined
                                                        }
                                                        className={clsx(
                                                            "flex items-center gap-2 px-3 py-2 text-[11px] font-heading font-semibold tracking-wider uppercase transition-all rounded-md flex-grow justify-center",
                                                            activeTab === id
                                                                ? "bg-[var(--color-bg-elevated)] text-[var(--color-accent)] shadow-sm border border-[var(--color-border)]"
                                                                : "text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-surface-soft)] border border-transparent"
                                                        )}
                                                    >
                                                        <Icon size={14} strokeWidth={1.5} />
                                                        <span className="relative top-[2px]">{t(tab.label)}</span>
                                                    </button>
                                                );
                                            })}
                                        </div>
                                    </div>
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
                                        ref={characterManagerRef}
                                        onActivateCharacter={onActivateCharacter}
                                        onCharacterRuntimeChange={onCharacterRuntimeChange}
                                        onCharactersChanged={onCharactersChanged}
                                        characters={charactersProp}
                                        resolveAvatarUrl={resolveAvatarUrl}
                                        characterToEditId={characterToEditId}
                                        activeCharacterId={activeCharacterId}
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
                                        characterId={readStringSetting(APP_SETTING_KEYS.activeCharacterId, "default") || "default"}
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
                                <div className={clsx(activeTab === "mods" ? "block" : "hidden", "min-h-[812px]")}>
                                    <div className="flex min-h-[812px] flex-col gap-3">
                                        <div className="h-[400px] shrink-0">
                                            <ContentLibrary />
                                        </div>
                                        <div className="h-[400px] shrink-0">
                                            <ModList />
                                        </div>
                                    </div>
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
                                        initialConfig={localVisionConfig}
                                        onConfigChange={(cfg) => {
                                            setLocalVisionConfig(cfg);
                                            onVisionConfigChange?.(cfg);
                                        }}
                                    />
                                </div>
                            )}
                            {mountedTabs.has("mcp") && (
                                <div className={activeTab === "mcp" ? "block" : "hidden"}>
                                    <McpTab
                                        initialServers={mcpServersProp}
                                        visionEnabled={visionConfigProp?.vlm_enabled}
                                        isActive={activeTab === "mcp"}
                                    />
                                </div>
                            )}
                            {mountedTabs.has("bot") && (
                                <div className={activeTab === "bot" ? "block" : "hidden"}>
                                    <BotTab
                                        botConfig={localBotConfig}
                                        initialStatus={initialTelegramStatus}
                                        initialCharacters={charactersProp}
                                        onBotConfigChange={setLocalBotConfig}
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
                            {mountedTabs.has("about") && (
                                <div className={activeTab === "about" ? "block" : "hidden"}>
                                    <AboutTab />
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
                                    value={getAppLanguageSelectValue(i18n.resolvedLanguage || i18n.language)}
                                    onChange={(v) => {
                                        i18n.changeLanguage(v);
                                        writeStringSetting(APP_SETTING_KEYS.appLanguage, v);
                                    }}
                                    options={APP_LANGUAGE_OPTIONS}
                                    className="min-w-[120px]"
                                />
                            </div>

                            <div className="flex items-center gap-3">
                                <motion.button
                                    whileHover={{ scale: 1.02 }}
                                    whileTap={{ scale: 0.98 }}
                                    onClick={handleCancel}
                                    data-onboarding-id="settings-cancel-button"
                                    className={clsx(
                                        "px-4 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                                        "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                                    )}
                                >
                                    <span className="block leading-none translate-y-px">
                                        {t("common.actions.cancel")}
                                    </span>
                                </motion.button>
                                <motion.button
                                    whileHover={{ scale: 1.05 }}
                                    whileTap={{ scale: 0.95 }}
                                    onClick={handleSave}
                                    className={clsx(
                                        "inline-flex items-center gap-2 px-5 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                                        "bg-[var(--color-accent)] text-black",
                                        "hover:bg-white transition-colors"
                                    )}
                                >
                                    {saved ? <Check size={16} strokeWidth={2} className="shrink-0" /> : <Save size={16} strokeWidth={1.5} className="shrink-0" />}
                                    <span className="leading-none translate-y-px">
                                        {saved ? t("common.actions.saved") : t("common.actions.save")}
                                    </span>
                                </motion.button>
                            </div>
                        </div>
                    </motion.div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
