import { motion } from "framer-motion";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { useEffect, useState, useRef } from "react";
import { Mic, Languages, Send, HandMetal, Server, Wand2, Download, CheckCircle, CircleAlert } from "lucide-react";
import type { SttConfig, SttProviderConfig, SenseVoiceLocalModelStatus, SenseVoiceLocalDownloadProgress } from "../../../lib/kokoro-bridge";
import { getSenseVoiceLocalStatus, downloadSenseVoiceLocalModel, onSenseVoiceLocalProgress } from "../../../lib/kokoro-bridge";
import { Select } from "@/components/ui/select";

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

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function SenseVoiceLocalPanel({ config, onUpdate }: {
    config: SttProviderConfig;
    onUpdate: (update: Partial<SttProviderConfig>) => void;
}) {
    const { t } = useTranslation();
    const [modelStatus, setModelStatus] = useState<SenseVoiceLocalModelStatus | null>(null);
    const [progress, setProgress] = useState<SenseVoiceLocalDownloadProgress | null>(null);
    const [downloading, setDownloading] = useState(false);
    const unlistenRef = useRef<(() => void) | null>(null);

    useEffect(() => {
        getSenseVoiceLocalStatus().then(setModelStatus).catch(console.error);

        onSenseVoiceLocalProgress((p) => setProgress(p)).then((unlisten) => {
            unlistenRef.current = unlisten;
        });

        return () => {
            unlistenRef.current?.();
        };
    }, []);

    const startDownload = async () => {
        setDownloading(true);
        setProgress(null);
        try {
            const result = await downloadSenseVoiceLocalModel();
            setModelStatus(result);
        } catch (e) {
            console.error("SenseVoice local model download failed:", e);
        } finally {
            setDownloading(false);
            setProgress(null);
        }
    };

    const progressPercent = progress && progress.total_bytes
        ? Math.round((progress.downloaded_bytes / progress.total_bytes) * 100)
        : null;

    const inputClass = "w-full px-3 py-1.5 rounded-md text-sm bg-[var(--color-bg-elevated)] border border-[var(--color-border)] focus:border-[var(--color-accent)] outline-none text-[var(--color-text-primary)]";
    const labelClass = "text-xs text-[var(--color-text-secondary)]";

    return (
        <div className="space-y-4">
            {/* Model Install Card */}
            <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface)] overflow-hidden">
                <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border)]/60">
                    <div className="flex items-center gap-2">
                        <Server size={14} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                        <span className="text-sm font-semibold text-[var(--color-text-primary)]">
                            {t("settings.stt.sensevoice_local.install_title")}
                        </span>
                    </div>
                    {modelStatus && (
                        modelStatus.installed ? (
                            <span className="flex items-center gap-1 text-xs text-emerald-400">
                                <CheckCircle size={12} />
                                {t("settings.stt.sensevoice_local.installed")}
                            </span>
                        ) : (
                            <span className="flex items-center gap-1 text-xs text-[var(--color-text-muted)]">
                                <CircleAlert size={12} />
                                {t("settings.stt.sensevoice_local.not_installed")}
                            </span>
                        )
                    )}
                </div>

                <div className="px-4 py-3 space-y-3">
                    {/* Progress bar or download button */}
                    {downloading ? (
                        <div className="space-y-2">
                            <div className="w-full h-1.5 bg-[var(--color-border)] rounded-full overflow-hidden">
                                <motion.div
                                    className="h-full bg-[var(--color-accent)] rounded-full"
                                    animate={{ width: progressPercent != null ? `${progressPercent}%` : "40%" }}
                                    transition={{ ease: "linear", duration: 0.3 }}
                                    style={{ width: progressPercent != null ? `${progressPercent}%` : undefined }}
                                />
                            </div>
                            <p className="text-xs text-[var(--color-text-muted)]">
                                {progress?.stage === "extracting"
                                    ? t("settings.stt.sensevoice_local.extracting")
                                    : progress?.stage === "complete"
                                        ? t("settings.stt.sensevoice_local.complete")
                                        : t("settings.stt.sensevoice_local.downloading", {
                                            downloaded: formatBytes(progress?.downloaded_bytes ?? 0),
                                            total: progress?.total_bytes ? formatBytes(progress.total_bytes) : "…",
                                        })
                                }
                            </p>
                        </div>
                    ) : (
                        <button
                            onClick={startDownload}
                            disabled={modelStatus?.installed}
                            className={clsx(
                                "flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-semibold transition-colors",
                                modelStatus?.installed
                                    ? "text-[var(--color-text-muted)] bg-[var(--color-bg-elevated)] cursor-not-allowed opacity-50"
                                    : "text-[var(--color-bg-base)] bg-[var(--color-accent)] hover:opacity-90 active:scale-95"
                            )}
                        >
                            <Download size={12} />
                            {t("settings.stt.sensevoice_local.download")}
                        </button>
                    )}

                    {/* Install dir */}
                    {modelStatus?.installed && (
                        <div className="space-y-1">
                            <div className={labelClass}>{t("settings.stt.sensevoice_local.install_dir")}</div>
                            <div className="text-xs text-[var(--color-text-muted)] font-mono break-all bg-[var(--color-bg-elevated)] px-2 py-1 rounded">
                                {modelStatus.install_dir}
                            </div>
                        </div>
                    )}
                </div>
            </div>

            {/* Advanced config fields */}
            <div className="grid grid-cols-1 gap-3">
                {/* model_path */}
                <div className="space-y-1">
                    <div className={labelClass}>{t("settings.stt.fields.model_path")}</div>
                    <input
                        type="text"
                        value={config.model_path || ""}
                        onChange={(e) => onUpdate({ model_path: e.target.value || undefined })}
                        placeholder={modelStatus?.model_path ?? ""}
                        className={inputClass}
                    />
                </div>

                {/* tokens_path */}
                <div className="space-y-1">
                    <div className={labelClass}>{t("settings.stt.fields.tokens_path")}</div>
                    <input
                        type="text"
                        value={config.tokens_path || ""}
                        onChange={(e) => onUpdate({ tokens_path: e.target.value || undefined })}
                        placeholder={modelStatus?.tokens_path ?? ""}
                        className={inputClass}
                    />
                </div>

                {/* num_threads */}
                <div className="space-y-1">
                    <div className={labelClass}>{t("settings.stt.fields.num_threads")}</div>
                    <input
                        type="number"
                        min={1}
                        max={32}
                        value={config.num_threads ?? 2}
                        onChange={(e) => onUpdate({ num_threads: parseInt(e.target.value) || 2 })}
                        className={inputClass}
                    />
                </div>

                {/* use_itn toggle */}
                <div className="flex items-center justify-between">
                    <div className={labelClass}>{t("settings.stt.fields.use_itn")}</div>
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={() => onUpdate({ use_itn: !(config.use_itn ?? true) })}
                        className={clsx(
                            "w-10 h-5 rounded-full relative transition-colors duration-200",
                            (config.use_itn ?? true)
                                ? "bg-[var(--color-accent)]"
                                : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: (config.use_itn ?? true) ? 20 : 2 }}
                            transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            className={clsx(
                                "w-4 h-4 rounded-full absolute top-0.5",
                                (config.use_itn ?? true) ? "bg-black" : "bg-[var(--color-text-muted)]"
                            )}
                        />
                    </motion.button>
                </div>
            </div>
        </div>
    );
}

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
    useEffect(() => {
        if (sttConfig && activeProvider && sttConfig.active_provider !== activeProvider.id) {
            onSttConfigChange({ ...sttConfig, active_provider: activeProvider.id });
        }
    }, [sttConfig?.active_provider, activeProvider?.id]);

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

    const getInfoText = () => {
        if (!activeProvider) return "";
        switch (activeProvider.provider_type) {
            case "openai_whisper":
                return t("settings.stt.info.openai");
            case "whisper_cpp":
                return t("settings.stt.info.whisper_cpp", { url: activeProvider.base_url || "default URL" });
            case "sensevoice_local":
                return t("settings.stt.info.sensevoice_local");
            default:
                return t("settings.stt.info.generic");
        }
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

                        <Select
                            value={sttConfig.active_provider}
                            onChange={(newId) => {
                                const newProviders = sttConfig.providers.map(p => ({
                                    ...p,
                                    enabled: p.id === newId ? true : p.enabled
                                }));
                                onSttConfigChange({ ...sttConfig, active_provider: newId, providers: newProviders });
                            }}
                            options={sttConfig.providers.map(p => ({
                                value: p.id,
                                label: getProviderLabel(p.provider_type),
                            }))}
                        />

                        {/* Active Provider Config Fields */}
                        {activeProvider && (
                            <div className="space-y-3 pt-2 border-t border-[var(--color-border)]/50 mt-2">
                                {activeProvider.provider_type === "sensevoice_local" ? (
                                    <SenseVoiceLocalPanel
                                        config={activeProvider}
                                        onUpdate={(update) => updateProvider(activeProvider.id, update)}
                                    />
                                ) : (
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

                                        {/* Model (faster_whisper and openai_whisper) */}
                                        {(activeProvider.provider_type === 'faster_whisper' || activeProvider.provider_type === 'openai_whisper') && (
                                            <div className="space-y-1">
                                                <div className="text-xs text-[var(--color-text-secondary)]">{t("settings.stt.fields.model")}</div>
                                                <input
                                                    type="text"
                                                    value={activeProvider.model || ""}
                                                    onChange={(e) => updateProvider(activeProvider.id, { model: e.target.value })}
                                                    placeholder={activeProvider.provider_type === 'openai_whisper' ? "whisper-1" : "base"}
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
                                )}
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
                            <Select
                                value={sttConfig.language || ""}
                                onChange={(v) => updateConfig({ language: v || undefined })}
                                options={LANG_OPTIONS.map(opt => ({
                                    value: opt.value,
                                    label: opt.value === "" ? t(opt.label) : opt.label,
                                }))}
                            />
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

                        {/* Wake word toggle + input */}
                        <div className="space-y-2">
                            <div className="flex items-center justify-between">
                                <div className="flex items-center gap-2">
                                    <Wand2 size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                                    <div>
                                        <div className="text-sm text-[var(--color-text-primary)]">
                                            {t("settings.stt.wake_word.title")}
                                        </div>
                                        <div className="text-xs text-[var(--color-text-muted)]">
                                            {t("settings.stt.wake_word.desc")}
                                        </div>
                                    </div>
                                </div>
                                <motion.button
                                    whileTap={{ scale: 0.95 }}
                                    onClick={() => updateConfig({ wake_word_enabled: !sttConfig.wake_word_enabled })}
                                    className={clsx(
                                        "w-12 h-6 rounded-full relative transition-colors duration-200",
                                        sttConfig.wake_word_enabled
                                            ? "bg-[var(--color-accent)]"
                                            : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                                    )}
                                >
                                    <motion.div
                                        animate={{ x: sttConfig.wake_word_enabled ? 24 : 2 }}
                                        transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                        className={clsx(
                                            "w-5 h-5 rounded-full absolute top-0.5",
                                            sttConfig.wake_word_enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                                        )}
                                    />
                                </motion.button>
                            </div>
                            {sttConfig.wake_word_enabled && (
                                <input
                                    type="text"
                                    value={sttConfig.wake_word || ""}
                                    onChange={(e) => updateConfig({ wake_word: e.target.value || undefined })}
                                    placeholder={t("settings.stt.wake_word.placeholder")}
                                    className="w-full px-3 py-1.5 rounded-md text-sm bg-[var(--color-bg-elevated)] border border-[var(--color-border)] focus:border-[var(--color-accent)] outline-none text-[var(--color-text-primary)]"
                                />
                            )}
                        </div>
                    </div>

                    {/* Info note */}
                    <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-3">
                        <p className="text-xs text-[var(--color-text-muted)] leading-relaxed flex gap-2">
                            <span className="shrink-0 pt-0.5">ℹ️</span>
                            <span>{getInfoText()}</span>
                        </p>
                    </div>
                </motion.div>
            )}
        </div>
    );
}
