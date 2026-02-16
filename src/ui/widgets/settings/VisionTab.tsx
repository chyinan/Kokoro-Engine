import { useState, useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import {
    Eye, MonitorSmartphone, Timer, Gauge, Server, KeyRound, Cpu,
    Camera, Loader2, AlertTriangle, Download, CheckCircle2, XCircle
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
    getVisionConfig, saveVisionConfig, captureScreenNow,
    listOllamaModels, pullOllamaModel, onOllamaPullProgress,
} from "../../../lib/kokoro-bridge";
import type { VisionConfig, OllamaModelInfo, OllamaPullProgress } from "../../../lib/kokoro-bridge";

export default function VisionTab() {
    const { t } = useTranslation();
    const [config, setConfig] = useState<VisionConfig | null>(null);
    const [loading, setLoading] = useState(true);
    const [capturing, setCapturing] = useState(false);
    const [captureResult, setCaptureResult] = useState<string | null>(null);
    const [ollamaModels, setOllamaModels] = useState<OllamaModelInfo[]>([]);
    const [ollamaReachable, setOllamaReachable] = useState(true);
    const [dirty, setDirty] = useState(false);

    // ── Model install state ──
    const [pulling, setPulling] = useState(false);
    const [pullProgress, setPullProgress] = useState<OllamaPullProgress | null>(null);
    const [pullError, setPullError] = useState<string | null>(null);
    const [pullDone, setPullDone] = useState(false);
    const unlistenRef = useRef<(() => void) | null>(null);

    // Load config on mount
    useEffect(() => {
        loadConfig();
        return () => {
            unlistenRef.current?.();
        };
    }, []);

    const loadConfig = async () => {
        try {
            const cfg = await getVisionConfig();
            setConfig(cfg);
            setLoading(false);

            // Try to load Ollama models in background
            if (cfg.vlm_provider === "ollama" && cfg.vlm_base_url) {
                try {
                    const models = await listOllamaModels(cfg.vlm_base_url.replace("/v1", ""));
                    setOllamaModels(models);
                    setOllamaReachable(true);
                } catch {
                    setOllamaReachable(false);
                }
            }
        } catch (e) {
            console.error("[VisionTab] Failed to load config:", e);
            setLoading(false);
        }
    };

    const update = (patch: Partial<VisionConfig>) => {
        if (!config) return;
        setConfig({ ...config, ...patch });
        setDirty(true);
    };

    const handleSave = async () => {
        if (!config) return;
        try {
            await saveVisionConfig(config);
            setDirty(false);
        } catch (e) {
            console.error("[VisionTab] Failed to save config:", e);
        }
    };

    const handleTestCapture = async () => {
        setCapturing(true);
        setCaptureResult(null);
        try {
            const desc = await captureScreenNow();
            setCaptureResult(desc);
        } catch (e) {
            setCaptureResult(`Error: ${e}`);
        } finally {
            setCapturing(false);
        }
    };

    // ── Model detection ──
    const isOllamaProvider = config?.vlm_provider === "ollama";
    const modelInstalled = isOllamaProvider && ollamaModels.length > 0
        ? ollamaModels.some(m => {
            // Ollama model names can have `:latest` suffix
            const configModel = (config?.vlm_model || "").split(":")[0].toLowerCase();
            const installedModel = m.name.split(":")[0].toLowerCase();
            return installedModel === configModel;
        })
        : true; // If we can't check, don't show warning
    const showModelWarning = isOllamaProvider && ollamaReachable && ollamaModels.length > 0 && !modelInstalled && !pulling && !pullDone;

    // ── Install model handler ──
    const handleInstallModel = async () => {
        if (!config || !config.vlm_base_url) return;
        setPulling(true);
        setPullError(null);
        setPullDone(false);
        setPullProgress(null);

        // Listen for progress events
        const unlisten = await onOllamaPullProgress((p) => {
            setPullProgress(p);
            if (p.status === "success") {
                setPullDone(true);
                setPulling(false);
                // Refresh model list
                refreshModels();
            }
        });
        unlistenRef.current = unlisten;

        try {
            const baseUrl = config.vlm_base_url.replace("/v1", "");
            await pullOllamaModel(baseUrl, config.vlm_model);
            // If the promise resolved without a "success" status event, mark done
            if (!pullDone) {
                setPullDone(true);
                setPulling(false);
                refreshModels();
            }
        } catch (e) {
            setPullError(String(e));
            setPulling(false);
        } finally {
            unlisten();
            unlistenRef.current = null;
        }
    };

    const refreshModels = async () => {
        if (!config?.vlm_base_url) return;
        try {
            const models = await listOllamaModels(config.vlm_base_url.replace("/v1", ""));
            setOllamaModels(models);
        } catch { /* ignore */ }
    };

    // ── Progress helpers ──
    const progressPercent = pullProgress?.total && pullProgress?.completed
        ? Math.min(100, Math.round((pullProgress.completed / pullProgress.total) * 100))
        : null;

    const progressLabel = (() => {
        if (!pullProgress) return t("settings.vision.progress.initializing");
        const { status } = pullProgress;
        if (status === "success") return t("settings.vision.progress.complete");
        if (status.startsWith("pulling")) {
            if (progressPercent !== null) return t("settings.vision.progress.downloading", { percent: progressPercent });
            return t("settings.vision.progress.pulling_manifest");
        }
        if (status.includes("verifying")) return t("settings.vision.progress.verifying");
        if (status.includes("writing")) return t("settings.vision.progress.writing");
        if (status.includes("removing")) return t("settings.vision.progress.cleanup");
        return status;
    })();

    if (loading || !config) {
        return (
            <div className="flex items-center justify-center py-12 text-[var(--color-text-muted)]">
                <Loader2 size={20} className="animate-spin mr-2" /> {t("settings.vision.loading")}
            </div>
        );
    }

    return (
        <div className="space-y-5">
            {/* Enable Vision toggle */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <Eye size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div>
                        <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                            {t("settings.vision.enable.label")}
                        </div>
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {t("settings.vision.enable.desc")}
                        </div>
                    </div>
                </div>
                <motion.button
                    whileTap={{ scale: 0.95 }}
                    onClick={() => update({ enabled: !config.enabled })}
                    className={clsx(
                        "w-12 h-6 rounded-full relative transition-colors duration-200",
                        config.enabled
                            ? "bg-[var(--color-accent)]"
                            : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: config.enabled ? 24 : 2 }}
                        transition={{ type: "spring", stiffness: 500, damping: 30 }}
                        className={clsx(
                            "w-5 h-5 rounded-full absolute top-0.5",
                            config.enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                        )}
                    />
                </motion.button>
            </div>

            {/* VLM Provider Config — always shown so user can configure before enabling */}
            <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                className="space-y-4 pl-7"
            >
                {/* Provider Type */}
                <div className="space-y-2">
                    <div className="flex items-center gap-2">
                        <Cpu size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                        <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.provider.label")}</label>
                    </div>
                    <select
                        value={config.vlm_provider}
                        onChange={(e) => {
                            const prov = e.target.value;
                            update({
                                vlm_provider: prov,
                                vlm_base_url: prov === "ollama" ? "http://localhost:11434/v1" : "https://api.openai.com/v1",
                                vlm_model: prov === "ollama" ? "minicpm-v" : "gpt-4o",
                                vlm_api_key: prov === "ollama" ? null : config.vlm_api_key,
                            });
                            // Reset install state
                            setPullDone(false);
                            setPullError(null);
                        }}
                        className={clsx(
                            "w-full px-3 py-2 rounded-lg text-sm",
                            "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                            "text-[var(--color-text-primary)]",
                            "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                        )}
                    >
                        <option value="ollama">{t("settings.vision.provider.ollama")}</option>
                        <option value="openai">{t("settings.vision.provider.openai")}</option>
                    </select>
                </div>

                {/* Ollama not reachable warning */}
                <AnimatePresence>
                    {isOllamaProvider && !ollamaReachable && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-warning)]/30 bg-[var(--color-warning)]/5 p-3"
                        >
                            <div className="flex items-start gap-2">
                                <AlertTriangle size={14} className="text-[var(--color-warning)] mt-0.5 shrink-0" />
                                <p className="text-xs text-[var(--color-warning)] leading-relaxed">
                                    {t("settings.vision.ollama.warning")}{" "}
                                    <span className="font-mono">{config.vlm_base_url?.replace("/v1", "") || "http://localhost:11434"}</span>
                                </p>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Base URL */}
                <div className="space-y-2">
                    <div className="flex items-center gap-2">
                        <Server size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                        <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.base_url")}</label>
                    </div>
                    <input
                        type="text"
                        value={config.vlm_base_url || ""}
                        onChange={(e) => update({ vlm_base_url: e.target.value || null })}
                        placeholder={config.vlm_provider === "ollama" ? "http://localhost:11434/v1" : "https://api.openai.com/v1"}
                        className={clsx(
                            "w-full px-3 py-2 rounded-lg text-sm",
                            "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                            "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                            "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                        )}
                    />
                </div>

                {/* Model */}
                <div className="space-y-2">
                    <div className="flex items-center gap-2">
                        <MonitorSmartphone size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                        <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.model.label")}</label>
                    </div>
                    {config.vlm_provider === "ollama" && ollamaModels.length > 0 ? (
                        <select
                            value={config.vlm_model}
                            onChange={(e) => {
                                update({ vlm_model: e.target.value });
                                setPullDone(false);
                                setPullError(null);
                            }}
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        >
                            {ollamaModels.map(m => (
                                <option key={m.name} value={m.name}>{m.name}</option>
                            ))}
                            {/* Show current model as option even if not installed */}
                            {!ollamaModels.some(m => m.name.split(":")[0].toLowerCase() === config.vlm_model.split(":")[0].toLowerCase()) && (
                                <option value={config.vlm_model}>{config.vlm_model} {t("settings.vision.model.not_installed_prefix")}</option>
                            )}
                        </select>
                    ) : (
                        <input
                            type="text"
                            value={config.vlm_model}
                            onChange={(e) => {
                                update({ vlm_model: e.target.value });
                                setPullDone(false);
                                setPullError(null);
                            }}
                            placeholder={config.vlm_provider === "ollama" ? "minicpm-v" : "gpt-4o"}
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        />
                    )}
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {config.vlm_provider === "ollama"
                            ? t("settings.vision.model.recommend.ollama")
                            : t("settings.vision.model.recommend.openai")}
                    </p>
                </div>

                {/* ── Model not installed warning + install UI ── */}
                <AnimatePresence>
                    {showModelWarning && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-warning)]/30 bg-[var(--color-warning)]/5 p-3 space-y-3"
                        >
                            <div className="flex items-start gap-2">
                                <AlertTriangle size={14} className="text-[var(--color-warning)] mt-0.5 shrink-0" />
                                <div className="space-y-1">
                                    <p className="text-xs text-[var(--color-warning)] font-semibold">
                                        {t("settings.vision.model.install_warning", { model: config.vlm_model })}
                                    </p>
                                    <p className="text-xs text-[var(--color-text-muted)]">
                                        {t("settings.vision.model.install_desc")}
                                    </p>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.97 }}
                                onClick={handleInstallModel}
                                className={clsx(
                                    "w-full flex items-center justify-center gap-2 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase",
                                    "bg-[var(--color-warning)]/20 border border-[var(--color-warning)]/30",
                                    "text-[var(--color-warning)]",
                                    "hover:bg-[var(--color-warning)]/30 transition-colors"
                                )}
                            >
                                <Download size={13} strokeWidth={1.5} />
                                {t("settings.vision.model.install_btn", { model: config.vlm_model })}
                            </motion.button>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* ── Pull progress UI ── */}
                <AnimatePresence>
                    {pulling && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/5 p-3 space-y-2"
                        >
                            <div className="flex items-center gap-2">
                                <Loader2 size={14} className="text-[var(--color-accent)] animate-spin" />
                                <span className="text-xs text-[var(--color-accent)] font-semibold">
                                    {t("settings.vision.model.installing", { model: config.vlm_model })}
                                </span>
                            </div>
                            <p className="text-xs text-[var(--color-text-muted)]">
                                {progressLabel}
                            </p>
                            {/* Progress bar */}
                            <div className="w-full h-1.5 rounded-full bg-[var(--color-bg-surface)] overflow-hidden">
                                <motion.div
                                    className="h-full rounded-full bg-[var(--color-accent)]"
                                    initial={{ width: "0%" }}
                                    animate={{
                                        width: progressPercent !== null ? `${progressPercent}%` : "30%",
                                    }}
                                    transition={{
                                        duration: 0.3,
                                        ease: "easeOut",
                                        ...(progressPercent === null ? {
                                            repeat: Infinity,
                                            repeatType: "mirror" as const,
                                            duration: 1.5
                                        } : {})
                                    }}
                                />
                            </div>
                            {progressPercent !== null && (
                                <p className="text-xs text-[var(--color-text-muted)] text-right font-mono">
                                    {pullProgress?.completed
                                        ? `${(pullProgress.completed / 1024 / 1024).toFixed(0)} MB`
                                        : ""
                                    }
                                    {pullProgress?.total
                                        ? ` / ${(pullProgress.total / 1024 / 1024).toFixed(0)} MB`
                                        : ""
                                    }
                                </p>
                            )}
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* ── Pull complete ── */}
                <AnimatePresence>
                    {pullDone && !pulling && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-success)]/30 bg-[var(--color-success)]/5 p-3"
                        >
                            <div className="flex items-center gap-2">
                                <CheckCircle2 size={14} className="text-[var(--color-success)]" />
                                <span className="text-xs text-[var(--color-success)] font-semibold">
                                    {t("settings.vision.model.success", { model: config.vlm_model })}
                                </span>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* ── Pull error ── */}
                <AnimatePresence>
                    {pullError && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-error)]/30 bg-[var(--color-error)]/5 p-3"
                        >
                            <div className="flex items-start gap-2">
                                <XCircle size={14} className="text-[var(--color-error)] mt-0.5 shrink-0" />
                                <div className="space-y-1">
                                    <p className="text-xs text-[var(--color-error)] font-semibold">
                                        {t("settings.vision.model.failed")}
                                    </p>
                                    <p className="text-xs text-[var(--color-text-muted)] break-all">
                                        {pullError}
                                    </p>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.97 }}
                                onClick={handleInstallModel}
                                className={clsx(
                                    "mt-2 w-full flex items-center justify-center gap-2 py-1.5 rounded-lg text-xs",
                                    "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                                    "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                                )}
                            >
                                <Download size={12} strokeWidth={1.5} />
                                {t("settings.vision.model.retry")}
                            </motion.button>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* API Key (only for online providers) */}
                {config.vlm_provider !== "ollama" && (
                    <div className="space-y-2">
                        <div className="flex items-center gap-2">
                            <KeyRound size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.api_key")}</label>
                        </div>
                        <input
                            type="password"
                            value={config.vlm_api_key || ""}
                            onChange={(e) => update({ vlm_api_key: e.target.value || null })}
                            placeholder="sk-..."
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        />
                    </div>
                )}

                {/* Interval */}
                <div className="space-y-2">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                            <Timer size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">
                                {t("settings.vision.interval.label")}
                            </label>
                        </div>
                        <span className="text-sm text-[var(--color-accent)] font-mono">{config.interval_secs}s</span>
                    </div>
                    <input
                        type="range"
                        min={5}
                        max={60}
                        step={5}
                        value={config.interval_secs}
                        onChange={(e) => update({ interval_secs: Number(e.target.value) })}
                        className="w-full accent-[var(--color-accent)]"
                    />
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {t("settings.vision.interval.desc")}
                    </p>
                </div>

                {/* Sensitivity */}
                <div className="space-y-2">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                            <Gauge size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">
                                {t("settings.vision.sensitivity.label")}
                            </label>
                        </div>
                        <span className="text-sm text-[var(--color-accent)] font-mono">{(config.change_threshold * 100).toFixed(0)}%</span>
                    </div>
                    <input
                        type="range"
                        min={1}
                        max={20}
                        step={1}
                        value={config.change_threshold * 100}
                        onChange={(e) => update({ change_threshold: Number(e.target.value) / 100 })}
                        className="w-full accent-[var(--color-accent)]"
                    />
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {t("settings.vision.sensitivity.desc", { percent: (config.change_threshold * 100).toFixed(0) })}
                    </p>
                </div>

                {/* Save Config Button */}
                {dirty && (
                    <motion.button
                        initial={{ opacity: 0, y: 5 }}
                        animate={{ opacity: 1, y: 0 }}
                        whileTap={{ scale: 0.97 }}
                        onClick={handleSave}
                        className={clsx(
                            "w-full py-2 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                            "bg-[var(--color-accent)] text-black",
                            "hover:bg-white transition-colors"
                        )}
                    >
                        {t("settings.vision.save")}
                    </motion.button>
                )}

                {/* Test Capture Button */}
                <div className="space-y-2">
                    <motion.button
                        whileTap={{ scale: 0.97 }}
                        onClick={handleTestCapture}
                        disabled={capturing}
                        className={clsx(
                            "w-full flex items-center justify-center gap-2 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                            capturing && "opacity-50 cursor-not-allowed"
                        )}
                    >
                        {capturing ? (
                            <Loader2 size={14} className="animate-spin" />
                        ) : (
                            <Camera size={14} strokeWidth={1.5} />
                        )}
                        {capturing ? t("settings.vision.test.capturing") : t("settings.vision.test.button")}
                    </motion.button>
                    {captureResult && (
                        <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-3">
                            <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                                👁️ {captureResult}
                            </p>
                        </div>
                    )}
                </div>

                {/* Privacy note */}
                <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-3">
                    <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                        {t("settings.vision.privacy_note")}
                    </p>
                </div>
            </motion.div>
        </div>
    );
}
