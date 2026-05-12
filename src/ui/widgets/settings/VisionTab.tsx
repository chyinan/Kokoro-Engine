import { useState, useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import {
    Eye, MonitorSmartphone, Timer, Gauge, Server, KeyRound, Cpu,
    Camera, Loader2, AlertTriangle, Video
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
    getVisionConfig, saveVisionConfig, captureScreenNow,
    listOllamaModels,
    listAnthropicModels,
    getLlamaCppStatus,
    listVisionScreens,
} from "../../../lib/kokoro-bridge";
import type { VisionConfig, OllamaModelInfo, VisionScreenInfo } from "../../../lib/kokoro-bridge";
import { Select } from "@/components/ui/select";

type CameraPreviewIssue = "no_devices" | "permission_denied" | "unsupported" | "unavailable";
type VisionContextHistoryMode = VisionConfig["vision_context_history_mode"];

function getCameraPreviewIssue(error: unknown): CameraPreviewIssue {
    const name = error instanceof DOMException ? error.name : "";
    if (name === "NotFoundError" || name === "DevicesNotFoundError") return "no_devices";
    if (name === "NotAllowedError" || name === "PermissionDeniedError" || name === "SecurityError") {
        return "permission_denied";
    }
    return "unavailable";
}

export default function VisionTab({ initialConfig = null, onConfigChange }: { initialConfig?: VisionConfig | null; onConfigChange?: (cfg: VisionConfig) => void } = {}) {
    const { t } = useTranslation();
    const [config, setConfig] = useState<VisionConfig | null>(initialConfig);
    const [loading, setLoading] = useState(initialConfig === null);
    const [capturing, setCapturing] = useState(false);
    const [captureResult, setCaptureResult] = useState<string | null>(null);
    const [ollamaModels, setOllamaModels] = useState<OllamaModelInfo[]>([]);
    const [ollamaReachable, setOllamaReachable] = useState(true);
    const [llamaCppModels, setLlamaCppModels] = useState<string[]>([]);
    const [llamaCppReachable, setLlamaCppReachable] = useState(true);
    const [anthropicModels, setAnthropicModels] = useState<string[]>([]);
    const [anthropicReachable, setAnthropicReachable] = useState(true);
    const [screens, setScreens] = useState<VisionScreenInfo[]>([]);
    const [screensLoading, setScreensLoading] = useState(false);
    const [screensError, setScreensError] = useState<string | null>(null);
    const [dirty, setDirty] = useState(false);
    const [editingInterval, setEditingInterval] = useState(false);
    const [intervalInput, setIntervalInput] = useState("");

    // ── Camera device picker + preview ──
    const [cameraDevices, setCameraDevices] = useState<MediaDeviceInfo[]>([]);
    const [selectedDeviceId, setSelectedDeviceId] = useState<string>("");
    const [cameraDevicesLoaded, setCameraDevicesLoaded] = useState(false);
    const [cameraPreviewLoading, setCameraPreviewLoading] = useState(false);
    const [cameraPreviewReady, setCameraPreviewReady] = useState(false);
    const [cameraPreviewIssue, setCameraPreviewIssue] = useState<CameraPreviewIssue | null>(null);
    const previewVideoRef = useRef<HTMLVideoElement>(null);
    const previewStreamRef = useRef<MediaStream | null>(null);

    useEffect(() => {
        if (!config?.camera_enabled) {
            stopPreview();
            setCameraDevicesLoaded(false);
            setCameraPreviewLoading(false);
            setCameraPreviewReady(false);
            setCameraPreviewIssue(null);
            return;
        }
        enumerateDevices(config.camera_device_id ?? "");
    }, [config?.camera_enabled]);

    useEffect(() => {
        if (!config?.camera_enabled) return;
        if (!cameraDevicesLoaded || cameraDevices.length === 0) return;
        startPreview(selectedDeviceId);
    }, [selectedDeviceId, config?.camera_enabled, cameraDevicesLoaded, cameraDevices.length]);

    async function enumerateDevices(preferredId: string = "") {
        stopPreview();
        setCameraDevicesLoaded(false);
        setCameraPreviewLoading(true);
        setCameraPreviewReady(false);
        setCameraPreviewIssue(null);

        if (!navigator.mediaDevices?.getUserMedia || !navigator.mediaDevices?.enumerateDevices) {
            setCameraDevices([]);
            setSelectedDeviceId("");
            setCameraPreviewIssue("unsupported");
            setCameraPreviewLoading(false);
            setCameraDevicesLoaded(true);
            return;
        }

        try {
            // Request permission first so labels are populated
            await navigator.mediaDevices.getUserMedia({ video: true }).then(s => s.getTracks().forEach(t => t.stop()));
            const devices = await navigator.mediaDevices.enumerateDevices();
            const videoDevices = devices.filter(d => d.kind === "videoinput");
            setCameraDevices(videoDevices);
            if (videoDevices.length === 0) {
                setSelectedDeviceId("");
                setCameraPreviewIssue("no_devices");
                return;
            }
            const initial = preferredId && videoDevices.some(d => d.deviceId === preferredId)
                ? preferredId
                : videoDevices[0]?.deviceId ?? "";
            setSelectedDeviceId(initial);
        } catch (err) {
            console.error("[VisionTab] enumerateDevices failed:", err);
            setCameraDevices([]);
            setSelectedDeviceId("");
            setCameraPreviewIssue(getCameraPreviewIssue(err));
        } finally {
            setCameraPreviewLoading(false);
            setCameraDevicesLoaded(true);
        }
    }

    async function startPreview(deviceId: string) {
        stopPreview();
        setCameraPreviewLoading(true);
        setCameraPreviewReady(false);
        setCameraPreviewIssue(null);

        if (!navigator.mediaDevices?.getUserMedia) {
            setCameraPreviewIssue("unsupported");
            setCameraPreviewLoading(false);
            return;
        }

        try {
            const constraints = deviceId
                ? { video: { deviceId: { exact: deviceId } } }
                : { video: true };
            const stream = await navigator.mediaDevices.getUserMedia(constraints);
            previewStreamRef.current = stream;
            if (previewVideoRef.current) {
                previewVideoRef.current.srcObject = stream;
                await previewVideoRef.current.play();
            }
            setCameraPreviewReady(true);
        } catch (err) {
            console.error("[VisionTab] preview failed:", err);
            setCameraPreviewIssue(getCameraPreviewIssue(err));
            stopPreview();
        } finally {
            setCameraPreviewLoading(false);
        }
    }

    function stopPreview() {
        previewStreamRef.current?.getTracks().forEach(t => t.stop());
        previewStreamRef.current = null;
        if (previewVideoRef.current) previewVideoRef.current.srcObject = null;
    }

    // Load config on mount
    useEffect(() => {
        if (initialConfig) {
            setConfig(initialConfig);
            setLoading(false);
        } else {
            loadConfig();
        }
        return () => {
            stopPreview();
        };
    }, [initialConfig]);

    useEffect(() => {
        let cancelled = false;
        const loadScreens = async () => {
            setScreensLoading(true);
            setScreensError(null);
            try {
                const list = await listVisionScreens();
                if (cancelled) return;
                setScreens(list);
            } catch (error) {
                if (cancelled) return;
                setScreens([]);
                setScreensError(String(error));
            } finally {
                if (!cancelled) setScreensLoading(false);
            }
        };

        loadScreens().catch((error) => {
            console.error("[VisionTab] Failed to load screens:", error);
        });

        return () => {
            cancelled = true;
        };
    }, []);

    useEffect(() => {
        if (!config) return;

        const provider = config.vlm_provider;
        const baseUrl = config.vlm_base_url;

        if (provider === "llm" || provider === "openai" || !baseUrl) {
            setOllamaModels([]);
            setLlamaCppModels([]);
            setAnthropicModels([]);
            setOllamaReachable(true);
            setLlamaCppReachable(true);
            setAnthropicReachable(true);
            return;
        }

        let cancelled = false;
        const timer = window.setTimeout(() => {
            const refreshModels = async () => {
                if (provider === "ollama") {
                    try {
                        const models = await listOllamaModels(baseUrl);
                        if (cancelled) return;
                        setOllamaModels(models);
                        setOllamaReachable(true);
                    } catch {
                        if (cancelled) return;
                        setOllamaModels([]);
                        setOllamaReachable(false);
                    }
                    if (!cancelled) {
                        setLlamaCppModels([]);
                        setLlamaCppReachable(true);
                        setAnthropicModels([]);
                        setAnthropicReachable(true);
                    }
                    return;
                }

                if (provider === "llama_cpp") {
                    try {
                        const status = await getLlamaCppStatus(baseUrl);
                        if (cancelled) return;
                        const detectedModels = Array.from(new Set([
                            ...status.available_models,
                            ...(status.current_model ? [status.current_model] : []),
                        ]));
                        setLlamaCppModels(detectedModels);
                        setLlamaCppReachable(true);
                    } catch {
                        if (cancelled) return;
                        setLlamaCppModels([]);
                        setLlamaCppReachable(false);
                    }
                    if (!cancelled) {
                        setOllamaModels([]);
                        setOllamaReachable(true);
                        setAnthropicModels([]);
                        setAnthropicReachable(true);
                    }
                    return;
                }

                if (provider === "anthropic") {
                    const apiKey = config.vlm_api_key || "";
                    if (!apiKey) {
                        setAnthropicModels([]);
                        setAnthropicReachable(true);
                        setOllamaModels([]);
                        setLlamaCppModels([]);
                        setOllamaReachable(true);
                        setLlamaCppReachable(true);
                        return;
                    }

                    try {
                        const models = await listAnthropicModels(baseUrl, apiKey);
                        if (cancelled) return;
                        setAnthropicModels(models);
                        setAnthropicReachable(true);
                    } catch {
                        if (cancelled) return;
                        setAnthropicModels([]);
                        setAnthropicReachable(false);
                    }
                    if (!cancelled) {
                        setOllamaModels([]);
                        setLlamaCppModels([]);
                        setOllamaReachable(true);
                        setLlamaCppReachable(true);
                    }
                }
            };

            refreshModels().catch((err) => {
                console.error("[VisionTab] Failed to refresh provider models:", err);
            });
        }, 300);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [config?.vlm_provider, config?.vlm_base_url, config?.vlm_api_key]);

    const loadConfig = async () => {
        try {
            const cfg = await getVisionConfig();
            setConfig(cfg);
            setLoading(false);
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

    const persistVisionConfig = async (cfg: VisionConfig) => {
        await saveVisionConfig(cfg);
        localStorage.setItem("kokoro_vision_config", JSON.stringify(cfg));
        window.dispatchEvent(new Event("kokoro-vision-settings-changed"));
        onConfigChange?.(cfg);
    };

    const handleSave = async () => {
        if (!config) return;
        try {
            await persistVisionConfig(config);
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
    const isLlamaCppProvider = config?.vlm_provider === "llama_cpp";
    const isAnthropicProvider = config?.vlm_provider === "anthropic";
    const isLlmProvider = config?.vlm_provider === "llm";
    const isOnlineProvider = config?.vlm_provider === "openai" || isAnthropicProvider;
    const detectedModels = isOllamaProvider
        ? ollamaModels.map((model) => model.name)
        : isLlamaCppProvider
            ? llamaCppModels
            : isAnthropicProvider
                ? anthropicModels
            : [];
    const selectedDisplayId = config?.display_id ?? "__auto__";
    const screenOptions = [
        {
            value: "__auto__",
            label: t("settings.vision.display.auto"),
            description: t("settings.vision.display.auto_desc"),
        },
        ...screens.map((screen) => ({
            value: screen.display_id,
            label: screen.label,
            description: `${screen.x},${screen.y} · ${screen.width}×${screen.height}${screen.is_primary ? ` · ${t("settings.vision.display.primary")}` : ""}`,
        })),
        ...(config?.display_id && !screens.some((screen) => screen.display_id === config.display_id)
            ? [{
                value: config.display_id,
                label: t("settings.vision.display.missing"),
                description: config.display_id,
            }]
            : []),
    ];
    const providerReachable = isOllamaProvider
        ? ollamaReachable
        : isLlamaCppProvider
            ? llamaCppReachable
            : isAnthropicProvider
                ? anthropicReachable
            : true;
    const hasMatchingDetectedModel = detectedModels.some((model) => {
        const configModel = (config?.vlm_model || "").split(":")[0].toLowerCase();
        const detectedModel = model.split(":")[0].toLowerCase();
        return detectedModel === configModel;
    });
    const modelInstalled = isOllamaProvider && ollamaModels.length > 0
        ? ollamaModels.some(m => {
            // Ollama model names can have `:latest` suffix
            const configModel = (config?.vlm_model || "").split(":")[0].toLowerCase();
            const installedModel = m.name.split(":")[0].toLowerCase();
            return installedModel === configModel;
        })
        : true; // If we can't check, don't show warning
    const showModelWarning = isOllamaProvider && ollamaReachable && ollamaModels.length > 0 && !modelInstalled;

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
                    onClick={async () => {
                        const next = { ...config, vlm_enabled: !config.vlm_enabled };
                        setConfig(next);
                        setDirty(false);
                        try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                    }}
                    className={clsx(
                        "w-12 h-6 rounded-full relative transition-colors duration-200",
                        config.vlm_enabled
                            ? "bg-[var(--color-accent)]"
                            : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: config.vlm_enabled ? 24 : 2 }}
                        transition={{ type: "spring", stiffness: 500, damping: 30 }}
                        className={clsx(
                            "w-5 h-5 rounded-full absolute top-0.5",
                            config.vlm_enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
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
                    <Select
                        value={config.vlm_provider}
                        onChange={(prov) => {
                            update({
                                vlm_provider: prov,
                                vlm_base_url: prov === "ollama"
                                    ? "http://localhost:11434/v1"
                                    : prov === "anthropic"
                                        ? "https://api.anthropic.com/v1"
                                    : prov === "llama_cpp"
                                        ? "http://127.0.0.1:8080"
                                        : prov === "llm"
                                            ? null
                                            : "https://api.openai.com/v1",
                                vlm_model: prov === "ollama" || prov === "llama_cpp"
                                    ? "minicpm-v"
                                    : prov === "anthropic"
                                        ? "claude-sonnet-4-20250514"
                                    : prov === "llm"
                                        ? ""
                                        : "gpt-4o",
                                vlm_api_key: prov === "openai" || prov === "anthropic" ? config.vlm_api_key : null,
                            });
                        }}
                        options={[
                            { value: "ollama", label: t("settings.vision.provider.ollama") },
                            { value: "llama_cpp", label: t("settings.vision.provider.llama_cpp") },
                            { value: "openai", label: t("settings.vision.provider.openai") },
                            { value: "anthropic", label: t("settings.vision.provider.anthropic") },
                            { value: "llm", label: t("settings.vision.provider.llm") },
                        ]}
                    />
                </div>

                {/* Local provider not reachable warning */}
                <AnimatePresence>
                    {(isOllamaProvider || isLlamaCppProvider || isAnthropicProvider) && !providerReachable && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="rounded-lg border border-[var(--color-warning)]/30 bg-[var(--color-warning)]/5 p-3"
                        >
                            <div className="flex items-start gap-2">
                                <AlertTriangle size={14} className="text-[var(--color-warning)] mt-0.5 shrink-0" />
                                <p className="text-xs text-[var(--color-warning)] leading-relaxed">
                                    {isOllamaProvider
                                        ? t("settings.vision.ollama.warning")
                                        : isLlamaCppProvider
                                            ? t("settings.vision.llama_cpp.warning")
                                            : t("settings.vision.anthropic.warning")}{" "}
                                    <span className="font-mono">{config.vlm_base_url || (isAnthropicProvider ? "https://api.anthropic.com/v1" : "http://127.0.0.1:8080")}</span>
                                </p>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* LLM provider info note */}
                {isLlmProvider && (
                    <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-accent)]/30 p-3">
                        <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                            {t("settings.vision.provider.llm_note")}
                        </p>
                    </div>
                )}

                {/* Base URL */}
                {!isLlmProvider && (
                    <div className="space-y-2">
                        <div className="flex items-center gap-2">
                            <Server size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.base_url")}</label>
                        </div>
                        <input
                            type="text"
                            value={config.vlm_base_url || ""}
                            onChange={(e) => update({ vlm_base_url: e.target.value || null })}
                            placeholder={config.vlm_provider === "ollama"
                                ? "http://localhost:11434/v1"
                                : config.vlm_provider === "anthropic"
                                    ? "https://api.anthropic.com/v1"
                                : config.vlm_provider === "llama_cpp"
                                    ? "http://127.0.0.1:8080"
                                    : "https://api.openai.com/v1"}
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        />
                    </div>
                )}

                {/* Model */}
                {!isLlmProvider && (
                <div className="space-y-2">
                    <div className="flex items-center gap-2">
                        <MonitorSmartphone size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                        <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.model.label")}</label>
                    </div>
                    {detectedModels.length > 0 ? (
                        <Select
                            value={config.vlm_model}
                            onChange={(v) => update({ vlm_model: v })}
                            options={[
                                ...detectedModels.map((model) => ({ value: model, label: model })),
                                ...(!hasMatchingDetectedModel && config.vlm_model
                                    ? [{
                                        value: config.vlm_model,
                                        label: isOllamaProvider
                                            ? `${config.vlm_model} ${t("settings.vision.model.not_installed_prefix")}`
                                            : config.vlm_model,
                                    }]
                                    : []),
                            ]}
                        />
                    ) : (
                        <input
                            type="text"
                            value={config.vlm_model}
                            onChange={(e) => update({ vlm_model: e.target.value })}
                            placeholder={config.vlm_provider === "ollama" || config.vlm_provider === "llama_cpp"
                                ? "minicpm-v"
                                : config.vlm_provider === "anthropic"
                                    ? "claude-sonnet-4-20250514"
                                    : "gpt-4o"}
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
                            : config.vlm_provider === "llama_cpp"
                                ? t("settings.vision.model.recommend.llama_cpp")
                                : config.vlm_provider === "anthropic"
                                    ? t("settings.vision.model.recommend.anthropic")
                                    : t("settings.vision.model.recommend.openai")}
                    </p>
                </div>
                )}

                {/* ── Model not installed warning ── */}
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
                                        {t("settings.vision.model.install_desc_manual")}
                                    </p>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* API Key (only for online providers) */}
                {isOnlineProvider && !isLlmProvider && (
                    <div className="space-y-2">
                        <div className="flex items-center gap-2">
                            <KeyRound size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">{t("settings.vision.api_key")}</label>
                        </div>
                        <input
                            type="password"
                            value={config.vlm_api_key || ""}
                            onChange={(e) => update({ vlm_api_key: e.target.value || null })}
                            placeholder={isAnthropicProvider ? "sk-ant-..." : "sk-..."}
                            className={clsx(
                                "w-full px-3 py-2 rounded-lg text-sm",
                                "bg-[var(--color-bg-surface)] border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "focus:outline-none focus:border-[var(--color-accent)] transition-colors"
                            )}
                        />
                    </div>
                )}

                {/* Display selection */}
                <div className="space-y-2">
                    <div className="flex items-center gap-2">
                        <MonitorSmartphone size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                        <label className="text-sm text-[var(--color-text-primary)]">
                            {t("settings.vision.display.label")}
                        </label>
                    </div>
                    <Select
                        value={selectedDisplayId}
                        onChange={(value) => update({ display_id: value === "__auto__" ? null : value })}
                        options={screenOptions}
                        disabled={screensLoading}
                        placeholder={screensLoading ? t("settings.vision.display.loading") : t("settings.vision.display.auto")}
                    />
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {screensError
                            ? t("settings.vision.display.error", { error: screensError })
                            : t("settings.vision.display.desc")}
                    </p>
                </div>

                {/* Interval */}
                <div className="space-y-2">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                            <Timer size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <label className="text-sm text-[var(--color-text-primary)]">
                                {t("settings.vision.interval.label")}
                            </label>
                        </div>
                        {editingInterval ? (
                            <input
                                type="number"
                                min={5}
                                autoFocus
                                value={intervalInput}
                                onChange={(e) => setIntervalInput(e.target.value)}
                                onBlur={() => {
                                    const v = parseInt(intervalInput, 10);
                                    if (!isNaN(v) && v >= 5) update({ capture_interval_secs: v });
                                    setEditingInterval(false);
                                }}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                                    if (e.key === "Escape") setEditingInterval(false);
                                }}
                                className="w-20 px-2 py-0.5 rounded text-sm text-right font-mono bg-[var(--color-bg-surface)] border border-[var(--color-accent)] text-[var(--color-accent)] focus:outline-none [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
                            />
                        ) : (
                            <span
                                className="text-sm text-[var(--color-accent)] font-mono cursor-pointer select-none"
                                title={t("settings.vision.interval.dblclick_hint")}
                                onDoubleClick={() => {
                                    setIntervalInput(String(config.capture_interval_secs));
                                    setEditingInterval(true);
                                }}
                            >
                                {config.capture_interval_secs}s
                            </span>
                        )}
                    </div>
                    <input
                        type="range"
                        min={5}
                        max={60}
                        step={5}
                        value={Math.min(config.capture_interval_secs, 60)}
                        onChange={(e) => update({ capture_interval_secs: Number(e.target.value) })}
                        className="w-full accent-[var(--color-accent)]"
                    />
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {t("settings.vision.interval.desc")}
                        {config.capture_interval_secs > 60 && (
                            <span className="ml-1 text-[var(--color-accent)]">
                                ({t("settings.vision.interval.custom")})
                            </span>
                        )}
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

                {/* LLM context history */}
                <div className="space-y-2 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/60 px-3 py-3">
                    <div className="flex items-start gap-3">
                        <MonitorSmartphone size={15} strokeWidth={1.5} className="mt-0.5 text-[var(--color-text-muted)]" />
                        <div className="min-w-0">
                            <div className="text-sm text-[var(--color-text-primary)]">
                                {t("settings.vision.contextHistory.label")}
                            </div>
                            <div className="mt-0.5 text-xs text-[var(--color-text-muted)] leading-relaxed">
                                {t("settings.vision.contextHistory.desc")}
                            </div>
                        </div>
                    </div>
                    <Select
                        value={config.vision_context_history_mode ?? "latest"}
                        onChange={async (value) => {
                            const mode = value as VisionContextHistoryMode;
                            const next = { ...config, vision_context_history_mode: mode };
                            setConfig(next);
                            setDirty(false);
                            try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                        }}
                        options={[
                            { value: "latest", label: t("settings.vision.contextHistory.latest") },
                            { value: "full", label: t("settings.vision.contextHistory.full") },
                        ]}
                        className="w-full"
                    />
                </div>

                {/* Background observation behavior */}
                <div className="overflow-hidden rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/60">
                    <div className="border-b border-[var(--color-border)]/70 px-3 py-2">
                        <div className="text-xs font-heading font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                            {t("settings.vision.behavior.title")}
                        </div>
                        {t("settings.vision.behavior.desc") && (
                            <div className="mt-0.5 text-[11px] leading-relaxed text-[var(--color-text-muted)]">
                                {t("settings.vision.behavior.desc")}
                            </div>
                        )}
                    </div>

                    <div className="divide-y divide-[var(--color-border)]/70">
                        <div className="flex items-center justify-between gap-4 px-3 py-3">
                            <div className="flex items-start gap-3">
                                <MonitorSmartphone size={15} strokeWidth={1.5} className="mt-0.5 text-[var(--color-text-muted)]" />
                                <div>
                                    <div className="text-sm text-[var(--color-text-primary)]">
                                        {t("settings.vision.auto.label")}
                                    </div>
                                    <div className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                                        {t("settings.vision.auto.desc")}
                                    </div>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.95 }}
                                onClick={async () => {
                                    const next = { ...config, auto_vision_enabled: !config.auto_vision_enabled };
                                    setConfig(next);
                                    setDirty(false);
                                    try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                                }}
                                className={clsx(
                                    "w-12 h-6 rounded-full relative transition-colors duration-200 shrink-0",
                                    config.auto_vision_enabled
                                        ? "bg-[var(--color-accent)]"
                                        : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                                )}
                                aria-pressed={config.auto_vision_enabled}
                            >
                                <motion.div
                                    animate={{ x: config.auto_vision_enabled ? 24 : 2 }}
                                    transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                    className={clsx(
                                        "w-5 h-5 rounded-full absolute top-0.5",
                                        config.auto_vision_enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                                    )}
                                />
                            </motion.button>
                        </div>

                        <div className="flex items-center justify-between gap-4 px-3 py-3">
                            <div className="flex items-start gap-3">
                                <Eye size={15} strokeWidth={1.5} className="mt-0.5 text-[var(--color-text-muted)]" />
                                <div>
                                    <div className="text-sm text-[var(--color-text-primary)]">
                                        {t("settings.vision.proactive.label")}
                                    </div>
                                    <div className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                                        {t("settings.vision.proactive.desc")}
                                    </div>
                                </div>
                            </div>
                            <motion.button
                                whileTap={{ scale: 0.95 }}
                                onClick={async () => {
                                    const next = { ...config, proactive_vision_enabled: !config.proactive_vision_enabled };
                                    setConfig(next);
                                    setDirty(false);
                                    try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                                }}
                                className={clsx(
                                    "w-12 h-6 rounded-full relative transition-colors duration-200 shrink-0",
                                    config.proactive_vision_enabled
                                        ? "bg-[var(--color-accent)]"
                                        : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                                )}
                                aria-pressed={config.proactive_vision_enabled}
                            >
                                <motion.div
                                    animate={{ x: config.proactive_vision_enabled ? 24 : 2 }}
                                    transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                    className={clsx(
                                        "w-5 h-5 rounded-full absolute top-0.5",
                                        config.proactive_vision_enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                                    )}
                                />
                            </motion.button>
                        </div>
                    </div>
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
                        disabled={capturing || dirty}
                        className={clsx(
                            "w-full flex items-center justify-center gap-2 py-2 rounded-lg text-sm font-heading font-semibold tracking-wider",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                            (capturing || dirty) && "opacity-50 cursor-not-allowed"
                        )}
                    >
                        {capturing ? (
                            <Loader2 size={14} className="animate-spin" />
                        ) : (
                            <Camera size={14} strokeWidth={1.5} />
                        )}
                        <span className="relative top-[2px]">
                            {capturing ? t("settings.vision.test.capturing") : t("settings.vision.test.button")}
                        </span>
                    </motion.button>
                    {dirty && (
                        <p className="text-xs text-[var(--color-warning)] text-center">
                            {t("settings.vision.test.save_first")}
                        </p>
                    )}
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

            {/* Camera section */}
            <div className="pt-2 border-t border-[var(--color-border)] space-y-4">
                {/* Camera enable toggle */}
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                        <Video size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                        <div>
                            <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                {t("settings.vision.camera.enable.label")}
                            </div>
                            <div className="text-xs text-[var(--color-text-muted)]">
                                {t("settings.vision.camera.enable.desc")}
                            </div>
                        </div>
                    </div>
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={async () => {
                            const next = { ...config, camera_enabled: !config.camera_enabled };
                            setConfig(next);
                            setDirty(false);
                            try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                        }}
                        className={clsx(
                            "w-12 h-6 rounded-full relative transition-colors duration-200",
                            config.camera_enabled
                                ? "bg-[var(--color-accent)]"
                                : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: config.camera_enabled ? 24 : 2 }}
                            transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            className={clsx(
                                "w-5 h-5 rounded-full absolute top-0.5",
                                config.camera_enabled ? "bg-black" : "bg-[var(--color-text-muted)]"
                            )}
                        />
                    </motion.button>
                </div>

                {/* Camera device picker + preview */}
                <AnimatePresence>
                    {config.camera_enabled && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="space-y-2 pl-7"
                        >
                            {/* Device picker */}
                            {cameraDevices.length > 1 && (
                                <div className="space-y-1 pt-1">
                                    <div className="flex items-center gap-2">
                                        <Video size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                                        <label className="text-sm text-[var(--color-text-primary)]">
                                            {t("settings.vision.camera.device.label")}
                                        </label>
                                    </div>
                                    <Select
                                        value={selectedDeviceId}
                                        onChange={async v => {
                                            setSelectedDeviceId(v);
                                            const next = { ...config, camera_device_id: v || null };
                                            setConfig(next);
                                            setDirty(false);
                                            try { await persistVisionConfig(next); } catch (e) { console.error("[VisionTab] auto-save failed:", e); }
                                        }}
                                        options={cameraDevices.map(d => ({
                                            value: d.deviceId,
                                            label: d.label || `Camera ${cameraDevices.indexOf(d) + 1}`,
                                        }))}
                                    />
                                </div>
                            )}

                            {/* Live preview */}
                            <div className="relative rounded-lg overflow-hidden border border-[var(--color-border)] bg-black aspect-video w-full mt-2">
                                <video
                                    ref={previewVideoRef}
                                    className={clsx(
                                        "w-full h-full object-cover",
                                        !cameraPreviewReady && "invisible"
                                    )}
                                    muted
                                    playsInline
                                />
                                {!cameraPreviewReady && (
                                    <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-[var(--color-bg-surface)] px-4 text-center">
                                        {cameraPreviewLoading ? (
                                            <Loader2 size={20} className="animate-spin text-[var(--color-accent)]" />
                                        ) : (
                                            <AlertTriangle size={20} className="text-[var(--color-warning)]" />
                                        )}
                                        <p className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                            {cameraPreviewLoading
                                                ? t("settings.vision.camera.feedback.loading")
                                                : t(`settings.vision.camera.feedback.${cameraPreviewIssue ?? "unavailable"}`)}
                                        </p>
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>


        </div>
    );
}
