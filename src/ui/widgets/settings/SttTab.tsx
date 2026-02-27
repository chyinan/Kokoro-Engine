import { motion } from "framer-motion";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { Mic, Languages, Send, HandMetal, Server } from "lucide-react";
import type { SttConfig, SttProviderConfig } from "../../../lib/kokoro-bridge";

interface SttTabProps {
    sttConfig: SttConfig;
    onSttConfigChange: (config: SttConfig) => void;
    voiceInterrupt: boolean;
    onVoiceInterruptChange: (enabled: boolean) => void;
}

const LANG_OPTIONS = [
    { value: "", label: "settings.stt.language.auto" },
    { value: "zh", label: "中文 (Chinese)" },
    { value: "en", label: "English" },
    { value: "ja", label: "日本語 (Japanese)" },
    { value: "ko", label: "한국어 (Korean)" },
    { value: "es", label: "Español (Spanish)" },
    { value: "fr", label: "Français (French)" },
    { value: "de", label: "Deutsch (German)" },
    { value: "ru", label: "Русский (Russian)" },
];

export default function SttTab({
    sttConfig,
    onSttConfigChange,
    voiceInterrupt,
    onVoiceInterruptChange,
}: SttTabProps) {
    const { t } = useTranslation();
    const activeProvider = sttConfig?.providers?.find(p => p.id === sttConfig.active_provider)
        ?? sttConfig?.providers?.[0];

    // If active_provider doesn't match any provider, fix it
    if (sttConfig && activeProvider && sttConfig.active_provider !== activeProvider.id) {
        onSttConfigChange({ ...sttConfig, active_provider: activeProvider.id });
    }

    if (!sttConfig || !sttConfig.providers) {
        return <div className="p-4 text-sm text-red-400">{t("settings.stt.error_invalid_config")}</div>;
    }

    const updateConfig = (update: Partial<SttConfig>) => {
        onSttConfigChange({ ...sttConfig, ...update });
    };

    const updateProvider = (providerId: string, update: Partial<SttProviderConfig>) => {
        const newProviders = sttConfig.providers.map(p => {
            if (p.id === providerId) {
                return { ...p, ...update };
            }
            return p;
        });
        updateConfig({ providers: newProviders });
    };

    const isEnabled = activeProvider?.enabled ?? false;

    const toggleEnabled = () => {
        if (!activeProvider) return;
        updateProvider(activeProvider.id, { enabled: !isEnabled });
    };

    const getProviderLabel = (type: string) => {
        const key = `settings.stt.providers.${type}`;
        const label = t(key);
        return label !== key ? label : type;
    };

    return (
        <div className="space-y-6">
            {/* Enable STT toggle (mapped to active provider's enable state) */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <Mic size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div>
                        <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                            {t("settings.stt.voice_input.title")}
                        </div>
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {t("settings.stt.voice_input.desc")}
                        </div>
                    </div>
                </div>
                <motion.button
                    whileTap={{ scale: 0.95 }}
                    onClick={toggleEnabled}
                    className={clsx(
                        "w-12 h-6 rounded-full relative transition-colors duration-200",
                        isEnabled
                            ? "bg-[var(--color-accent)]"
                            : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: isEnabled ? 24 : 2 }}
                        transition={{ type: "spring", stiffness: 500, damping: 30 }}
                        className={clsx(
                            "w-5 h-5 rounded-full absolute top-0.5",
                            isEnabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                        )}
                    />
                </motion.button>
            </div>

            {isEnabled && (
                <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: "auto" }}
                    className="space-y-6 pl-0"
                >
                    {/* Provider Selection */}
                    <div className="space-y-3 p-4 rounded-xl bg-[var(--color-bg-surface-soft)] border border-[var(--color-border)]">
                        <div className="flex items-center gap-2 mb-2">
                            <Server size={14} strokeWidth={1.5} className="text-[var(--color-text-secondary)]" />
                            <label className="text-sm font-semibold text-[var(--color-text-primary)]">
                                {t("settings.stt.provider.label")}
                            </label>
                        </div>

                        <select
                            value={sttConfig.active_provider}
                            onChange={(e) => {
                                const newId = e.target.value;
                                const newProviders = sttConfig.providers.map(p => ({
                                    ...p,
                                    enabled: p.id === newId ? true : p.enabled
                                }));
                                onSttConfigChange({ ...sttConfig, active_provider: newId, providers: newProviders });
                            }}
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        >
                            {sttConfig.providers.map(p => (
                                <option key={p.id} value={p.id}>
                                    {getProviderLabel(p.provider_type)}
                                </option>
                            ))}
                        </select>

                        {/* Active Provider Config Fields */}
                        {activeProvider && (
                            <div className="space-y-3 pt-2 border-t border-[var(--color-border)]/50 mt-2">
                                <div className="grid grid-cols-1 gap-3">
                                    {/* Base URL (for local providers) */}
                                    {activeProvider.provider_type !== 'openai_whisper' && (
                                        <div className="space-y-1">
                                            <div className="text-xs text-[var(--color-text-secondary)]">{t("settings.stt.fields.server_url")}</div>
                                            <input
                                                type="text"
                                                value={activeProvider.base_url || ""}
                                                onChange={(e) => updateProvider(activeProvider.id, { base_url: e.target.value })}
                                                placeholder="http://127.0.0.1:8080"
                                                className="w-full px-3 py-1.5 rounded-md text-sm bg-[var(--color-bg-elevated)] border border-[var(--color-border)] focus:border-[var(--color-accent)] outline-none"
                                            />
                                        </div>
                                    )}

                                    {/* API Key (OpenAI mainly) */}
                                    {activeProvider.provider_type === 'openai_whisper' && (
                                        <div className="space-y-1">
                                            <div className="text-xs text-[var(--color-text-secondary)]">{t("settings.stt.fields.api_key_hint")}</div>
                                            <input
                                                type="password"
                                                value={activeProvider.api_key || ""}
                                                onChange={(e) => updateProvider(activeProvider.id, { api_key: e.target.value })}
                                                placeholder="sk-..."
                                                className="w-full px-3 py-1.5 rounded-md text-sm bg-[var(--color-bg-elevated)] border border-[var(--color-border)] focus:border-[var(--color-accent)] outline-none"
                                            />
                                        </div>
                                    )}
                                </div>
                            </div>
                        )}
                    </div>

                    <div className="space-y-5 px-1">
                        {/* Auto-send toggle */}
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-2">
                                <Send size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                                <div>
                                    <div className="text-sm text-[var(--color-text-primary)]">
                                        {t("settings.stt.auto_send.title")}
                                    </div>
                                    <div className="text-xs text-[var(--color-text-muted)]">
                                        {t("settings.stt.auto_send.desc")}
                                    </div>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.95 }}
                                onClick={() => updateConfig({ auto_send: !sttConfig.auto_send })}
                                className={clsx(
                                    "w-12 h-6 rounded-full relative transition-colors duration-200",
                                    sttConfig.auto_send
                                        ? "bg-[var(--color-accent)]"
                                        : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                                )}
                            >
                                <motion.div
                                    animate={{ x: sttConfig.auto_send ? 24 : 2 }}
                                    transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                    className={clsx(
                                        "w-5 h-5 rounded-full absolute top-0.5",
                                        sttConfig.auto_send ? "bg-black" : "bg-[var(--color-text-muted)]"
                                    )}
                                />
                            </motion.button>
                        </div>

                        {/* Language hint */}
                        <div className="space-y-2">
                            <div className="flex items-center gap-2">
                                <Languages size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                                <label className="text-sm text-[var(--color-text-primary)]">
                                    {t("settings.stt.language.label")}
                                </label>
                            </div>
                            <select
                                value={sttConfig.language || ""}
                                onChange={(e) => updateConfig({ language: e.target.value || undefined })}
                                className={clsx(
                                    "w-full px-3 py-2 rounded-lg text-sm",
                                    "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                    "text-[var(--color-text-primary)]",
                                    "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                                )}
                            >
                                {LANG_OPTIONS.map(opt => (
                                    <option key={opt.value} value={opt.value}>
                                        {opt.value === "" ? t(opt.label) : opt.label}
                                    </option>
                                ))}
                            </select>
                            <p className="text-xs text-[var(--color-text-muted)]">
                                {t("settings.stt.language.hint")}
                            </p>
                        </div>

                        {/* Voice Interrupt toggle */}
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-2">
                                <HandMetal size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                                <div>
                                    <div className="text-sm text-[var(--color-text-primary)]">
                                        {t("settings.stt.voice_interrupt.title")}
                                    </div>
                                    <div className="text-xs text-[var(--color-text-muted)]">
                                        {t("settings.stt.voice_interrupt.desc")}
                                    </div>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.95 }}
                                onClick={() => onVoiceInterruptChange(!voiceInterrupt)}
                                className={clsx(
                                    "w-12 h-6 rounded-full relative transition-colors duration-200",
                                    voiceInterrupt
                                        ? "bg-[var(--color-accent)]"
                                        : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                                )}
                            >
                                <motion.div
                                    animate={{ x: voiceInterrupt ? 24 : 2 }}
                                    transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                    className={clsx(
                                        "w-5 h-5 rounded-full absolute top-0.5",
                                        voiceInterrupt ? "bg-black" : "bg-[var(--color-text-muted)]"
                                    )}
                                />
                            </motion.button>
                        </div>
                    </div>

                    {/* Info note */}
                    <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-3">
                        <p className="text-xs text-[var(--color-text-muted)] leading-relaxed flex gap-2">
                            <span className="shrink-0 pt-0.5">ℹ️</span>
                            <span>
                                {activeProvider?.provider_type === 'openai_whisper'
                                    ? t("settings.stt.info.openai")
                                    : activeProvider?.provider_type === 'whisper_cpp'
                                        ? t("settings.stt.info.whisper_cpp", { url: activeProvider?.base_url || "default URL" })
                                        : t("settings.stt.info.generic")
                                }
                            </span>
                        </p>
                    </div>
                </motion.div>
            )}
        </div>
    );
}
