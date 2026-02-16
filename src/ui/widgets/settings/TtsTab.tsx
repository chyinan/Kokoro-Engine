/**
 * TtsTab — Auto-speak toggle, active playback settings,
 * and provider management section.
 *
 * Extracted from SettingsPanel lines 502–798.
 */
import { useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { clsx } from "clsx";
import { Trash2, RefreshCw } from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { inputClasses, labelClasses, sectionHeadingClasses } from "../../styles/settings-primitives";
import { synthesize, listGptSovitsModels } from "../../../lib/kokoro-bridge";
import type { GptSovitsModels } from "../../../lib/kokoro-bridge";
import type { ProviderStatus, VoiceProfile, TtsSystemConfig } from "../../../lib/kokoro-bridge";
import type { ProviderConfigData } from "../../../core/types/mod";

export interface TtsTabProps {
    ttsConfig: TtsSystemConfig | null;
    onTtsConfigChange: (config: TtsSystemConfig) => void;
    providers: ProviderStatus[];
    voices: VoiceProfile[];
    isTtsLoading: boolean;
    onRefresh: () => void;
    // Active playback settings
    ttsEnabled: boolean;
    onTtsEnabledChange: (v: boolean) => void;
    ttsProviderId: string;
    onTtsProviderIdChange: (v: string) => void;
    ttsVoice: string;
    onTtsVoiceChange: (v: string) => void;
    ttsSpeed: string;
    onTtsSpeedChange: (v: string) => void;
    ttsPitch: string;
    onTtsPitchChange: (v: string) => void;
}

export default function TtsTab({
    ttsConfig, onTtsConfigChange,
    providers, voices, isTtsLoading, onRefresh,
    ttsEnabled, onTtsEnabledChange,
    ttsProviderId, onTtsProviderIdChange,
    ttsVoice, onTtsVoiceChange,
    ttsSpeed, onTtsSpeedChange,
    ttsPitch, onTtsPitchChange,
}: TtsTabProps) {
    const [editingProviderId, setEditingProviderId] = useState<string | null>(null);
    const [scannedModels, setScannedModels] = useState<Record<string, GptSovitsModels>>({});
    const { t } = useTranslation();

    // Scan for GPT-SoVITS models when install_path changes
    const scanModels = useCallback(async (providerId: string, installPath: string) => {
        if (!installPath.trim()) {
            setScannedModels(prev => { const next = { ...prev }; delete next[providerId]; return next; });
            return;
        }
        try {
            const models = await listGptSovitsModels(installPath.trim());
            setScannedModels(prev => ({ ...prev, [providerId]: models }));
        } catch (e) {
            console.warn("[TTS] Failed to scan GPT-SoVITS models:", e);
            setScannedModels(prev => { const next = { ...prev }; delete next[providerId]; return next; });
        }
    }, []);

    // Auto-scan on mount for providers that already have install_path
    useEffect(() => {
        if (!ttsConfig) return;
        for (const p of ttsConfig.providers) {
            if (p.provider_type === "gpt_sovits" && p.extra?.install_path) {
                scanModels(p.id, p.extra.install_path as string);
            }
        }
    }, [ttsConfig?.providers.length]); // eslint-disable-line react-hooks/exhaustive-deps

    const addProvider = (type: string) => {
        if (!ttsConfig) return;

        let baseId = type.toLowerCase();
        let newId = baseId;
        let counter = 2;

        const existingIds = ttsConfig.providers.map(p => p.id.toLowerCase());
        while (existingIds.includes(newId)) {
            newId = `${baseId}_${counter}`;
            counter++;
        }

        const newProvider: ProviderConfigData = {
            id: newId,
            provider_type: type,
            enabled: true,
            api_key: "",
            extra: {}
        };
        onTtsConfigChange({
            ...ttsConfig,
            providers: [...ttsConfig.providers, newProvider]
        });
        setEditingProviderId(newId);
    };

    const updateProviderConfig = (index: number, update: Partial<ProviderConfigData>) => {
        if (!ttsConfig) return;
        const newProviders = [...ttsConfig.providers];
        newProviders[index] = { ...newProviders[index], ...update };
        onTtsConfigChange({ ...ttsConfig, providers: newProviders });
    };

    const removeProvider = (index: number) => {
        if (!ttsConfig) return;
        if (ttsConfig.providers[index].id === "browser") return;

        const newProviders = ttsConfig.providers.filter((_, i) => i !== index);
        onTtsConfigChange({ ...ttsConfig, providers: newProviders });
        if (editingProviderId === ttsConfig.providers[index].id) {
            setEditingProviderId(null);
        }
    };

    return (
        <div className="space-y-6">
            {/* Auto-speak toggle */}
            <div className="flex items-center justify-between p-3 rounded-lg bg-black/20 border border-[var(--color-border)]">
                <div>
                    <span className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.tts.auto_speak.label")}</span>
                    <p className="text-[10px] text-[var(--color-text-muted)] mt-0.5">
                        {t("settings.tts.auto_speak.desc")}
                    </p>
                </div>
                <button
                    onClick={() => onTtsEnabledChange(!ttsEnabled)}
                    className={clsx(
                        "w-10 h-5 rounded-full transition-colors relative",
                        ttsEnabled ? "bg-emerald-500/50" : "bg-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: ttsEnabled ? 20 : 2 }}
                        className="absolute top-0.5 w-4 h-4 rounded-full bg-white"
                    />
                </button>
            </div>

            {/* Section: Playback Settings */}
            <div className="space-y-4">
                <h3 className={clsx(sectionHeadingClasses, "mb-3")}>{t("settings.tts.active_settings.title")}</h3>

                {/* Active Provider Selector */}
                <div>
                    <label className={labelClasses}>{t("settings.tts.active_settings.provider")}</label>
                    <select
                        value={ttsProviderId}
                        onChange={e => onTtsProviderIdChange(e.target.value)}
                        className={inputClasses}
                    >
                        {providers.map(p => (
                            <option key={p.id} value={p.id}>
                                {p.id.toUpperCase()} {p.available ? "" : t("settings.tts.active_settings.unavailable")}
                            </option>
                        ))}
                        {providers.length === 0 && <option value="browser">{t("settings.tts.active_settings.browser")}</option>}
                    </select>
                </div>

                {/* Voice Selector — hidden for GPT-SoVITS (voice is determined by model + ref audio) */}
                {!ttsConfig?.providers.find(p => p.id === ttsProviderId && p.provider_type === "gpt_sovits") && (
                    <div>
                        <label className={labelClasses}>{t("settings.tts.active_settings.voice")}</label>
                        <select
                            value={ttsVoice}
                            onChange={e => onTtsVoiceChange(e.target.value)}
                            className={inputClasses}
                        >
                            {(() => {
                                const filtered = voices.filter(v => v.provider_id === ttsProviderId);
                                if (filtered.length === 0) {
                                    return <option value="">{t("settings.tts.active_settings.no_voices")}</option>;
                                }
                                return filtered.map(v => (
                                    <option key={v.voice_id} value={v.voice_id}>
                                        {v.name} ({v.gender} · {v.language})
                                    </option>
                                ));
                            })()}
                        </select>
                    </div>
                )}

                {/* Speed */}
                <div>
                    <label className={labelClasses}>{t("settings.tts.active_settings.speed")}</label>
                    <div className="flex items-center gap-3">
                        <input
                            type="range"
                            min="0.5"
                            max="2.0"
                            step="0.1"
                            value={ttsSpeed}
                            onChange={e => onTtsSpeedChange(e.target.value)}
                            className="flex-1 accent-[var(--color-accent)]"
                        />
                        <span className="text-sm font-mono text-[var(--color-text-secondary)] w-10 text-right">
                            {ttsSpeed}x
                        </span>
                    </div>
                </div>

                {/* Pitch — only show if selected provider supports it */}
                {providers.find(p => p.id === ttsProviderId)?.capabilities.supports_pitch && (
                    <div>
                        <label className={labelClasses}>{t("settings.tts.active_settings.pitch")}</label>
                        <div className="flex items-center gap-3">
                            <input
                                type="range"
                                min="0.5"
                                max="2.0"
                                step="0.1"
                                value={ttsPitch}
                                onChange={e => onTtsPitchChange(e.target.value)}
                                className="flex-1 accent-[var(--color-accent)]"
                            />
                            <span className="text-sm font-mono text-[var(--color-text-secondary)] w-10 text-right">
                                {ttsPitch}x
                            </span>
                        </div>
                    </div>
                )}

                {/* Test Voice Button */}
                <div className="pt-2">
                    <button
                        onClick={() => {
                            synthesize("Hello! This is a test of the TTS system.", {
                                provider_id: ttsProviderId || undefined,
                                voice: ttsVoice || undefined,
                                speed: parseFloat(ttsSpeed || "1.0"),
                                pitch: parseFloat(ttsPitch || "1.0"),
                            }).catch(err => console.error("[TTS] Test failed:", err));
                        }}
                        className={clsx(
                            "w-full py-2.5 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase transition-all",
                            "border border-[var(--color-accent)]/40 text-[var(--color-accent)]",
                            "hover:bg-[var(--color-accent)]/10 hover:border-[var(--color-accent)]",
                            "active:scale-[0.98]"
                        )}
                    >
                        {t("settings.tts.active_settings.test")}
                    </button>
                </div>
            </div>

            <div className="border-t border-[var(--color-border)] opacity-50" />

            {/* Section: Manage Providers */}
            <div className="space-y-4">
                <div className="flex justify-between items-center mb-2">
                    <h3 className={sectionHeadingClasses}>{t("settings.tts.manage_providers.title")}</h3>
                    <button
                        onClick={onRefresh}
                        disabled={isTtsLoading}
                        className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:underline disabled:opacity-50 flex items-center gap-1"
                    >
                        <RefreshCw size={10} className={isTtsLoading ? "animate-spin" : ""} />
                        {isTtsLoading ? t("settings.tts.manage_providers.loading") : t("settings.tts.manage_providers.refresh")}
                    </button>
                </div>

                <div className="space-y-3">
                    {ttsConfig?.providers.map((provider, index) => (
                        <motion.div
                            key={index}
                            layout
                            className={clsx(
                                "rounded-lg border overflow-hidden transition-all",
                                editingProviderId === provider.id
                                    ? "border-[var(--color-accent)] bg-[var(--color-bg-elevated)]"
                                    : "border-[var(--color-border)] bg-black/20"
                            )}
                        >
                            {/* Provider Header */}
                            <div className="flex items-center justify-between p-3">
                                <div className="flex items-center gap-3">
                                    <button
                                        onClick={() => updateProviderConfig(index, { enabled: !provider.enabled })}
                                        className={clsx(
                                            "w-8 h-4 rounded-full transition-colors relative",
                                            provider.enabled ? "bg-emerald-500/50" : "bg-[var(--color-border)]"
                                        )}
                                    >
                                        <motion.div
                                            animate={{ x: provider.enabled ? 16 : 2 }}
                                            className="absolute top-0.5 w-3 h-3 rounded-full bg-white"
                                        />
                                    </button>
                                    <div className="flex flex-col">
                                        <span className={clsx(
                                            "text-sm font-heading font-bold uppercase",
                                            editingProviderId === provider.id ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                        )}>
                                            {provider.id}
                                        </span>
                                        <span className="text-[10px] text-[var(--color-text-muted)] uppercase">
                                            {provider.provider_type}
                                        </span>
                                    </div>
                                </div>
                                <div className="flex items-center gap-2">
                                    <button
                                        onClick={() => setEditingProviderId(editingProviderId === provider.id ? null : provider.id)}
                                        className="p-1.5 rounded hover:bg-white/5 text-[var(--color-text-secondary)]"
                                    >
                                        {editingProviderId === provider.id ? t("settings.tts.manage_providers.done") : t("settings.tts.manage_providers.edit")}
                                    </button>
                                    {provider.id !== "browser" && (
                                        <button
                                            onClick={() => removeProvider(index)}
                                            className="p-1.5 rounded hover:bg-red-500/20 text-[var(--color-text-muted)] hover:text-red-400"
                                        >
                                            <Trash2 size={14} />
                                        </button>
                                    )}
                                </div>
                            </div>

                            {/* Edit Form */}
                            <AnimatePresence>
                                {editingProviderId === provider.id && (
                                    <motion.div
                                        initial={{ height: 0, opacity: 0 }}
                                        animate={{ height: "auto", opacity: 1 }}
                                        exit={{ height: 0, opacity: 0 }}
                                        className="border-t border-[var(--color-border)] p-4 space-y-3 bg-black/20"
                                    >
                                        {/* Common Fields */}
                                        {/* Common API Key Field */}
                                        {(provider.provider_type === "openai" || provider.provider_type === "azure" || provider.provider_type === "elevenlabs") && (
                                            <div>
                                                <label className={labelClasses}>{t("settings.tts.fields.api_key")}</label>
                                                <input
                                                    type="password"
                                                    value={provider.api_key || ""}
                                                    onChange={e => updateProviderConfig(index, { api_key: e.target.value })}
                                                    placeholder="sk-..."
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                            </div>
                                        )}

                                        {/* Base URL Field */}
                                        {(provider.provider_type === "openai" || provider.provider_type === "local_vits" || provider.provider_type === "gpt_sovits" || provider.provider_type === "local_rvc") && (
                                            <div>
                                                <label className={labelClasses}>
                                                    {(provider.provider_type.includes("local") || provider.provider_type === "gpt_sovits") ? t("settings.tts.fields.server_url") : t("settings.tts.fields.base_url")}
                                                </label>
                                                <input
                                                    type="text"
                                                    value={provider.base_url || provider.endpoint || ""}
                                                    onChange={e => updateProviderConfig(index, { base_url: e.target.value })}
                                                    placeholder={
                                                        provider.provider_type === "gpt_sovits" ? "http://127.0.0.1:9880" :
                                                            provider.provider_type.includes("local") ? "http://127.0.0.1:5000" :
                                                                "https://api.openai.com/v1"
                                                    }
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                                {provider.provider_type === "gpt_sovits" && (
                                                    <p className="text-[10px] text-[var(--color-text-muted)] mt-1">
                                                        Requires running <code className="text-[var(--color-accent)]">api_v2.py</code> separately (not the WebUI)
                                                    </p>
                                                )}
                                            </div>
                                        )}

                                        {/* GPT-SoVITS Specific Fields */}
                                        {provider.provider_type === "gpt_sovits" && (
                                            <>
                                                <div>
                                                    <label className={labelClasses}>{t("settings.tts.fields.ref_audio.label")} <span className="text-red-400">*</span></label>
                                                    <input
                                                        type="text"
                                                        value={(provider.extra?.ref_audio_path as string) || ""}
                                                        onChange={e => updateProviderConfig(index, {
                                                            extra: { ...provider.extra, ref_audio_path: e.target.value }
                                                        })}
                                                        placeholder="D:/path/to/reference.wav"
                                                        className={clsx(inputClasses, "font-mono text-xs")}
                                                    />
                                                    <p className="text-[10px] text-[var(--color-text-muted)] mt-1">
                                                        {t("settings.tts.fields.ref_audio.desc")}
                                                    </p>
                                                </div>
                                                <div>
                                                    <label className={labelClasses}>{t("settings.tts.fields.prompt_text.label")} <span className="text-[var(--color-text-muted)]">{t("settings.tts.fields.prompt_text.optional")}</span></label>
                                                    <input
                                                        type="text"
                                                        value={(provider.extra?.prompt_text as string) || ""}
                                                        onChange={e => updateProviderConfig(index, {
                                                            extra: { ...provider.extra, prompt_text: e.target.value }
                                                        })}
                                                        placeholder={t("settings.tts.fields.prompt_text.placeholder")}
                                                        className={clsx(inputClasses, "text-xs")}
                                                    />
                                                </div>
                                                <div className="grid grid-cols-2 gap-3">
                                                    <div>
                                                        <label className={labelClasses}>{t("settings.tts.fields.prompt_lang")}</label>
                                                        <select
                                                            value={(provider.extra?.prompt_lang as string) || "zh"}
                                                            onChange={e => updateProviderConfig(index, {
                                                                extra: { ...provider.extra, prompt_lang: e.target.value }
                                                            })}
                                                            className={inputClasses}
                                                        >
                                                            <option value="zh">中文</option>
                                                            <option value="en">English</option>
                                                            <option value="ja">日本語</option>
                                                            <option value="ko">한국어</option>
                                                            <option value="yue">粵語</option>
                                                            <option value="auto">Auto</option>
                                                        </select>
                                                    </div>
                                                    <div>
                                                        <label className={labelClasses}>{t("settings.tts.fields.text_lang")}</label>
                                                        <select
                                                            value={(provider.extra?.text_lang as string) || "zh"}
                                                            onChange={e => updateProviderConfig(index, {
                                                                extra: { ...provider.extra, text_lang: e.target.value }
                                                            })}
                                                            className={inputClasses}
                                                        >
                                                            <option value="zh">中文</option>
                                                            <option value="en">English</option>
                                                            <option value="ja">日本語</option>
                                                            <option value="ko">한국어</option>
                                                            <option value="yue">粵語</option>
                                                            <option value="auto">Auto</option>
                                                        </select>
                                                    </div>
                                                </div>
                                                <div>
                                                    <label className={labelClasses}>{t("settings.tts.fields.install_path.label")}</label>
                                                    <div className="flex gap-2">
                                                        <input
                                                            type="text"
                                                            value={(provider.extra?.install_path as string) || ""}
                                                            onChange={e => updateProviderConfig(index, {
                                                                extra: { ...provider.extra, install_path: e.target.value }
                                                            })}
                                                            placeholder="D:/Software/GPT-SoVITS-1007-cu124"
                                                            className={clsx(inputClasses, "font-mono text-xs flex-1")}
                                                        />
                                                        <button
                                                            type="button"
                                                            onClick={() => scanModels(provider.id, (provider.extra?.install_path as string) || "")}
                                                            className="px-2 py-1 rounded text-xs bg-[var(--color-surface-2)] hover:bg-[var(--color-surface-3)] text-[var(--color-text-secondary)] transition-colors"
                                                            title="Scan for models"
                                                        >
                                                            <RefreshCw size={14} />
                                                        </button>
                                                    </div>
                                                    <p className="text-[10px] text-[var(--color-text-muted)] mt-1">
                                                        {t("settings.tts.fields.install_path.desc")}
                                                    </p>
                                                </div>
                                                <div className="grid grid-cols-2 gap-3">
                                                    <div>
                                                        <label className={labelClasses}>{t("settings.tts.fields.gpt_model")}</label>
                                                        <input
                                                            type="text"
                                                            list={`gpt-models-${provider.id}`}
                                                            value={(provider.extra?.gpt_weights as string) || ""}
                                                            onChange={e => updateProviderConfig(index, {
                                                                extra: { ...provider.extra, gpt_weights: e.target.value }
                                                            })}
                                                            placeholder={scannedModels[provider.id]?.gpt_models.length ? t("settings.tts.fields.model_placeholder") : "GPT_weights_v2Pro/xxx.ckpt"}
                                                            className={clsx(inputClasses, "font-mono text-xs")}
                                                        />
                                                        <datalist id={`gpt-models-${provider.id}`}>
                                                            {(scannedModels[provider.id]?.gpt_models || []).map(m => (
                                                                <option key={m} value={m} />
                                                            ))}
                                                        </datalist>
                                                    </div>
                                                    <div>
                                                        <label className={labelClasses}>{t("settings.tts.fields.sovits_model")}</label>
                                                        <input
                                                            type="text"
                                                            list={`sovits-models-${provider.id}`}
                                                            value={(provider.extra?.sovits_weights as string) || ""}
                                                            onChange={e => updateProviderConfig(index, {
                                                                extra: { ...provider.extra, sovits_weights: e.target.value }
                                                            })}
                                                            placeholder={scannedModels[provider.id]?.sovits_models.length ? t("settings.tts.fields.model_placeholder") : "SoVITS_weights_v2Pro/xxx.pth"}
                                                            className={clsx(inputClasses, "font-mono text-xs")}
                                                        />
                                                        <datalist id={`sovits-models-${provider.id}`}>
                                                            {(scannedModels[provider.id]?.sovits_models || []).map(m => (
                                                                <option key={m} value={m} />
                                                            ))}
                                                        </datalist>
                                                    </div>
                                                </div>
                                                <p className="text-[10px] text-[var(--color-text-muted)] -mt-1">
                                                    {t("settings.tts.fields.default_hint")}
                                                </p>
                                            </>
                                        )}

                                        {/* Model */}
                                        {(provider.provider_type === "openai" || provider.provider_type === "azure") && (
                                            <div>
                                                <label className={labelClasses}>{t("settings.tts.fields.model_id")}</label>
                                                <input
                                                    type="text"
                                                    value={provider.model || ""}
                                                    onChange={e => updateProviderConfig(index, { model: e.target.value })}
                                                    placeholder="tts-1"
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                            </div>
                                        )}

                                        {/* Model Path (Local) */}
                                        {(provider.provider_type.includes("local")) && (
                                            <div>
                                                <label className={labelClasses}>{t("settings.tts.fields.model_path")}</label>
                                                <input
                                                    type="text"
                                                    value={provider.model_path || ""}
                                                    onChange={e => updateProviderConfig(index, { model_path: e.target.value })}
                                                    placeholder="path/to/model.pth"
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                            </div>
                                        )}

                                        <div className="pt-2 text-[10px] text-[var(--color-text-muted)] italic">
                                            {t("settings.tts.manage_providers.save_hint")}
                                        </div>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </motion.div>
                    ))}
                </div>

                {/* Add Provider Dropdown */}
                <div className="pt-2">
                    <div className="grid grid-cols-2 gap-2">
                        {["openai", "local_vits", "gpt_sovits", "local_rvc", "azure", "elevenlabs"].map(type => (
                            <button
                                key={type}
                                onClick={() => addProvider(type)}
                                className="px-3 py-2 text-xs border border-[var(--color-border)] rounded hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors uppercase tracking-wider"
                            >
                                {t("settings.tts.manage_providers.add")} {type.replace("_", " ")}
                            </button>
                        ))}
                    </div>
                </div>
            </div>
        </div>
    );
}
