import { useState, useEffect, useRef, useTransition } from "react";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { motion, AnimatePresence } from "framer-motion";
import {
    Smile,
    Download,
    Trash2,
    Power,
    Sparkles,
    AlertCircle,
    Loader2,
    Play,
    Send,
    FolderOpen,
    FolderArchive,
    FolderInput,
    RefreshCw,
} from "lucide-react";
import {
    getEmotionModelStatus,
    downloadEmotionModel,
    uninstallEmotionModel,
    toggleEmotionModel,
    inferEmotion,
    openEmotionModelDir,
    importEmotionModelPackage,
    onEmotionModelProgress,
    playCue,
} from "../../../lib/kokoro-bridge";
import type {
    EmotionModelStatus,
    EmotionInferenceResult,
    EmotionModelDownloadProgress,
} from "../../../lib/kokoro-bridge";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";

function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

export function EmotionModelPanel() {
    const { t } = useTranslation();
    const [status, setStatus] = useState<EmotionModelStatus | null>(null);
    const [progress, setProgress] = useState<EmotionModelDownloadProgress | null>(null);
    const [isDownloading, setIsDownloading] = useState(false);
    const [isActionLoading, setIsActionLoading] = useState(false);
    const [errorMessage, setErrorMessage] = useState<string | null>(null);
    const [showUninstallConfirm, setShowUninstallConfirm] = useState(false);

    // Playground state
    const [testInput, setTestInput] = useState("今天真是太开心了，所有任务都顺利完成了！");
    const [inferenceResult, setInferenceResult] = useState<EmotionInferenceResult | null>(null);
    const [isInferring, setIsInferring] = useState(false);
    const [, startTransition] = useTransition();

    const unlistenRef = useRef<(() => void) | null>(null);

    const refreshStatus = async () => {
        try {
            const current = await getEmotionModelStatus();
            setStatus(current);
            if (current.error_message) {
                setErrorMessage(current.error_message);
            } else {
                setErrorMessage(null);
            }
        } catch (err) {
            console.error("Failed to get emotion model status:", err);
            setErrorMessage(String(err));
        }
    };

    useEffect(() => {
        void refreshStatus();

        onEmotionModelProgress((p) => {
            setProgress(p);
            if (p.stage === "ready") {
                setIsDownloading(false);
                setProgress(null);
                void refreshStatus();
            }
        }).then((unlisten) => {
            unlistenRef.current = unlisten;
        });

        return () => {
            unlistenRef.current?.();
        };
    }, []);

    // Auto-refresh status on window focus (e.g. user dragged files into folder)
    useEffect(() => {
        const onFocus = () => {
            void refreshStatus();
        };
        window.addEventListener("focus", onFocus);
        return () => {
            window.removeEventListener("focus", onFocus);
        };
    }, []);

    interface ManualImportOptions {
        directory?: boolean;
    }

    const handleManualImport = async (options: ManualImportOptions = {}) => {
        const isDirectory = options.directory ?? false;
        setIsActionLoading(true);
        setErrorMessage(null);
        try {
            const { open } = await import("@tauri-apps/plugin-dialog");
            const selected = await open({
                title: isDirectory
                    ? t("settings.model.emotion_model.import_dir_title", { defaultValue: "选择已解压的模型目录 (包含 model.onnx 等)" })
                    : t("settings.model.emotion_model.import_btn", { defaultValue: "手动导入模型包 (.zip / 目录)" }),
                multiple: false,
                directory: isDirectory,
                filters: isDirectory
                    ? undefined
                    : [
                        { name: "Emotion Model Archive", extensions: ["zip", "onnx"] },
                        { name: "All Files", extensions: ["*"] },
                    ],
            });

            if (!selected) {
                setIsActionLoading(false);
                return;
            }

            const path = typeof selected === "string" ? selected : selected[0];
            if (!path) {
                setIsActionLoading(false);
                return;
            }

            setProgress({
                stage: "verifying",
                message: isDirectory
                    ? t("settings.model.emotion_model.importing_dir", { defaultValue: "正在校验并导入模型目录..." })
                    : t("settings.model.emotion_model.importing", { defaultValue: "正在校验并导入模型包..." }),
                current_file: "model.onnx",
                file_index: 1,
                file_count: 1,
                downloaded_bytes: 0,
                total_bytes: null,
            });

            const newStatus = await importEmotionModelPackage(path);
            setStatus(newStatus);
            setErrorMessage(null);
        } catch (err) {
            console.error("Manual import failed:", err);
            setErrorMessage(String(err));
        } finally {
            setIsActionLoading(false);
            setProgress(null);
        }
    };

    const handleOpenDirectory = async () => {
        try {
            await openEmotionModelDir();
        } catch (err) {
            console.error("Failed to open model directory:", err);
            setErrorMessage(String(err));
        }
    };

    const handleDownload = async () => {
        setIsDownloading(true);
        setErrorMessage(null);
        setProgress({
            stage: "checking",
            message: t("settings.model.emotion_model.checking", { defaultValue: "检查模型资源中..." }),
            current_file: "model.onnx",
            file_index: 1,
            file_count: 5,
            downloaded_bytes: 0,
            total_bytes: null,
        });

        try {
            const result = await downloadEmotionModel();
            setStatus(result);
        } catch (err) {
            console.error("Emotion model download failed:", err);
            setErrorMessage(String(err));
        } finally {
            setIsDownloading(false);
            setProgress(null);
        }
    };

    const handleUninstall = async () => {
        setIsActionLoading(true);
        setErrorMessage(null);
        try {
            const result = await uninstallEmotionModel();
            setStatus(result);
            setShowUninstallConfirm(false);
            setInferenceResult(null);
        } catch (err) {
            console.error("Failed to uninstall emotion model:", err);
            setErrorMessage(String(err));
        } finally {
            setIsActionLoading(false);
        }
    };

    const handleToggleActive = async () => {
        if (!status) return;
        setIsActionLoading(true);
        setErrorMessage(null);
        try {
            const result = await toggleEmotionModel(!status.is_active);
            setStatus(result);
        } catch (err) {
            console.error("Failed to toggle emotion model active state:", err);
            setErrorMessage(String(err));
        } finally {
            setIsActionLoading(false);
        }
    };

    const handleTestInference = async (textToTest?: string) => {
        const text = (textToTest ?? testInput).trim();
        if (!text) return;
        setIsInferring(true);
        setErrorMessage(null);
        try {
            const res = await inferEmotion(text);
            startTransition(() => {
                setInferenceResult(res);
            });
        } catch (err) {
            console.error("Emotion inference failed:", err);
            setErrorMessage(String(err));
        } finally {
            setIsInferring(false);
        }
    };

    const handlePlayMappedCue = (cueName: string) => {
        void playCue(cueName);
    };

    const progressPercent =
        progress && progress.total_bytes && progress.total_bytes > 0
            ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100))
            : null;

    const samplePrompts = [
        "今天真是太开心了，所有任务都顺利完成了！",
        "心里好难受啊，事情搞砸了，不知该怎么办...",
        "你到底在说什么呀？我怎么完全听不明白呢？",
        "哇！这简直是太出乎我的意料了，太神奇了吧！",
        "哼，我才不在意你刚才说了什么呢，不要理你啦！",
    ];

    return (
        <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/80 backdrop-blur-md overflow-hidden transition-all shadow-sm">
            {/* Header */}
            <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-4 border-b border-[var(--color-border)]/60 bg-black/10">
                <div className="flex items-center gap-2.5">
                    <div className="p-1.5 rounded-lg bg-[var(--color-accent)]/10 text-[var(--color-accent)] border border-[var(--color-accent)]/20">
                        <Smile size={18} strokeWidth={1.8} />
                    </div>
                    <div>
                        <div className="flex items-center gap-2">
                            <span className="text-sm font-semibold font-heading text-[var(--color-text-primary)]">
                                {t("settings.model.emotion_model.title", {
                                    defaultValue: "本地轻量级 ONNX 情感模型 (Chinese-Emotion-Small)",
                                })}
                            </span>
                            <span className="text-[10px] uppercase font-mono px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/30">
                                INT8 ~25MB
                            </span>
                        </div>
                        <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
                            {t("settings.model.emotion_model.desc", {
                                defaultValue: "端侧毫秒级中文 8 维情感识别，无缝驱动 Live2D 微表情与 TTS 语调",
                            })}
                        </p>
                    </div>
                </div>

                {/* Status indicator pill */}
                <div className="flex items-center gap-2">
                    {status ? (
                        status.installed ? (
                            !status.is_valid ? (
                                <span className="flex items-center gap-1.5 text-xs font-medium px-2.5 py-1 rounded-full bg-rose-500/15 text-rose-300 border border-rose-500/30">
                                    <span className="w-1.5 h-1.5 rounded-full bg-rose-400" />
                                    {t("settings.model.emotion_model.status_corrupt", { defaultValue: "模型损坏" })}
                                </span>
                            ) : status.is_active ? (
                                <span className="flex items-center gap-1.5 text-xs font-medium px-2.5 py-1 rounded-full bg-emerald-500/15 text-emerald-300 border border-emerald-500/30 shadow-sm shadow-emerald-500/10">
                                    <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
                                    {t("settings.model.emotion_model.status_active", { defaultValue: "运行中" })}
                                </span>
                            ) : (
                                <span className="flex items-center gap-1.5 text-xs font-medium px-2.5 py-1 rounded-full bg-amber-500/15 text-amber-300 border border-amber-500/30">
                                    <span className="w-1.5 h-1.5 rounded-full bg-amber-400" />
                                    {t("settings.model.emotion_model.status_disabled", { defaultValue: "已停用" })}
                                </span>
                            )
                        ) : (
                            <span className="text-xs text-[var(--color-text-muted)] px-2.5 py-1 rounded-full bg-white/5 border border-white/10">
                                {t("settings.model.emotion_model.status_not_installed", { defaultValue: "未安装" })}
                            </span>
                        )
                    ) : (
                        <Loader2 size={14} className="animate-spin text-[var(--color-text-muted)]" />
                    )}
                </div>
            </div>

            {/* Content Body */}
            <div className="p-5 space-y-4">
                {/* Error Banner with Rich Offline Fallback Actions */}
                {errorMessage && (
                    <div className="p-3.5 rounded-lg bg-rose-500/10 border border-rose-500/30 text-rose-300 text-xs space-y-3">
                        <div className="flex items-start gap-2.5">
                            <AlertCircle size={15} className="mt-0.5 shrink-0 text-rose-400" />
                            <div className="flex-1 break-words font-medium">{errorMessage}</div>
                        </div>

                        <div className="pt-2 border-t border-rose-500/20 flex flex-wrap items-center justify-between gap-2">
                            <span className="text-[11px] text-rose-200/80">
                                {t("settings.model.emotion_model.offline_hint_desc", {
                                    defaultValue: "上游官方源未提供原生 ONNX 格式，或国内网络可能连接超时。您可获取离线包后一键导入：",
                                })}
                            </span>
                            <div className="flex flex-wrap items-center gap-2">
                                <button
                                    type="button"
                                    onClick={() => handleManualImport({ directory: false })}
                                    disabled={isActionLoading || isDownloading}
                                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-rose-500/20 hover:bg-rose-500/30 text-rose-200 border border-rose-500/30 font-semibold text-[11px] transition-all active:scale-95"
                                >
                                    <FolderArchive size={13} />
                                    {t("settings.model.emotion_model.offline_import_action", {
                                        defaultValue: "导入已下载的离线包 (.zip)",
                                    })}
                                </button>
                                <button
                                    type="button"
                                    onClick={() => handleManualImport({ directory: true })}
                                    disabled={isActionLoading || isDownloading}
                                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-rose-500/20 hover:bg-rose-500/30 text-rose-200 border border-rose-500/30 font-semibold text-[11px] transition-all active:scale-95"
                                >
                                    <FolderInput size={13} />
                                    {t("settings.model.emotion_model.offline_import_dir_action", {
                                        defaultValue: "导入解压目录",
                                    })}
                                </button>
                                <button
                                    type="button"
                                    onClick={handleOpenDirectory}
                                    className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-rose-200 border border-white/10 text-[11px] transition-all"
                                >
                                    <FolderOpen size={13} />
                                    {t("settings.model.emotion_model.open_dir_btn", {
                                        defaultValue: "打开存储目录",
                                    })}
                                </button>
                            </div>
                        </div>
                    </div>
                )}

                {/* Download Progress Bar */}
                <AnimatePresence>
                    {isDownloading && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="space-y-2 p-3.5 rounded-lg bg-black/20 border border-[var(--color-border)]"
                        >
                            <div className="flex justify-between items-center text-xs">
                                <span className="text-[var(--color-text-primary)] font-medium flex items-center gap-2">
                                    <Loader2 size={13} className="animate-spin text-[var(--color-accent)]" />
                                    {progress?.message || t("settings.model.emotion_model.downloading", { defaultValue: "正在下载模型组件..." })}
                                </span>
                                <span className="font-mono text-[var(--color-text-muted)]">
                                    {progressPercent != null ? `${progressPercent}%` : ""}
                                </span>
                            </div>

                            <div className="w-full h-2 bg-black/40 rounded-full overflow-hidden border border-white/5">
                                <motion.div
                                    className="h-full bg-gradient-to-r from-[var(--color-accent)] to-emerald-400 rounded-full"
                                    animate={{ width: progressPercent != null ? `${progressPercent}%` : "50%" }}
                                    transition={{ ease: "easeOut", duration: 0.3 }}
                                />
                            </div>

                            <div className="flex justify-between text-[11px] text-[var(--color-text-muted)] font-mono">
                                <span>{progress?.current_file || "model.onnx"}</span>
                                <span>
                                    {formatBytes(progress?.downloaded_bytes ?? 0)}
                                    {progress?.total_bytes ? ` / ${formatBytes(progress.total_bytes)}` : ""}
                                </span>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Operations Row */}
                <div className="flex flex-wrap items-center justify-between gap-3 pt-1">
                    <div className="flex flex-wrap items-center gap-2.5">
                        {!status?.installed ? (
                            <>
                                <button
                                    onClick={handleDownload}
                                    disabled={isDownloading || isActionLoading}
                                    className="flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-semibold text-black bg-[var(--color-accent)] hover:opacity-95 active:scale-95 disabled:opacity-50 transition-all shadow-sm shadow-[var(--color-accent)]/20"
                                >
                                    <Download size={14} />
                                    {t("settings.model.emotion_model.download_btn", {
                                        defaultValue: "下载模型 (~25MB)",
                                    })}
                                </button>

                                <button
                                    onClick={() => handleManualImport({ directory: false })}
                                    disabled={isDownloading || isActionLoading}
                                    className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-xs font-medium text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 active:scale-95 disabled:opacity-50 transition-all"
                                >
                                    <FolderArchive size={14} />
                                    {t("settings.model.emotion_model.import_btn", {
                                        defaultValue: "手动导入包体",
                                    })}
                                </button>

                                <button
                                    onClick={() => handleManualImport({ directory: true })}
                                    disabled={isDownloading || isActionLoading}
                                    className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-xs font-medium text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 active:scale-95 disabled:opacity-50 transition-all"
                                >
                                    <FolderInput size={14} />
                                    {t("settings.model.emotion_model.import_dir_btn", {
                                        defaultValue: "导入模型目录",
                                    })}
                                </button>

                                <button
                                    onClick={handleOpenDirectory}
                                    className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/5 transition-all"
                                    title={t("settings.model.emotion_model.open_dir_btn", { defaultValue: "打开存储目录" })}
                                >
                                    <FolderOpen size={14} />
                                    {t("settings.model.emotion_model.open_dir_btn", { defaultValue: "打开目录" })}
                                </button>

                                <button
                                    onClick={() => void refreshStatus()}
                                    className="p-2 rounded-lg text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/5 transition-all"
                                    title="刷新状态"
                                >
                                    <RefreshCw size={13} />
                                </button>
                            </>
                        ) : (
                            <>
                                {!status.is_valid ? (
                                    <button
                                        onClick={handleDownload}
                                        disabled={isDownloading || isActionLoading}
                                        className="flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-semibold text-white bg-rose-600 hover:bg-rose-500 active:scale-95 disabled:opacity-50 transition-all shadow-sm shadow-rose-600/20"
                                    >
                                        <RefreshCw size={13} className={isDownloading ? "animate-spin" : ""} />
                                        {t("settings.model.emotion_model.repair_btn", {
                                            defaultValue: "修复模型",
                                        })}
                                    </button>
                                ) : (
                                    /* Start / Stop Toggle */
                                    <button
                                        onClick={handleToggleActive}
                                        disabled={isActionLoading || isDownloading}
                                        className={clsx(
                                            "flex items-center gap-2 px-3.5 py-2 rounded-lg text-xs font-semibold transition-all border",
                                            status.is_active
                                                ? "bg-emerald-500/15 border-emerald-500/40 text-emerald-300 hover:bg-emerald-500/25"
                                                : "bg-white/5 border-white/10 text-[var(--color-text-secondary)] hover:bg-white/10"
                                        )}
                                    >
                                        <Power size={13} className={status.is_active ? "text-emerald-400" : "text-slate-400"} />
                                        {status.is_active
                                            ? t("settings.model.emotion_model.btn_stop", { defaultValue: "停用端侧感知" })
                                            : t("settings.model.emotion_model.btn_start", { defaultValue: "启动端侧感知" })}
                                    </button>
                                )}

                                <button
                                    onClick={() => handleManualImport({ directory: false })}
                                    disabled={isActionLoading || isDownloading}
                                    className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/10 transition-colors"
                                    title={t("settings.model.emotion_model.reimport_file_btn", { defaultValue: "重新导入文件 (.zip/.onnx)" })}
                                >
                                    <FolderArchive size={13} />
                                    {t("settings.model.emotion_model.reimport_file_btn", { defaultValue: "重新导入文件" })}
                                </button>

                                <button
                                    onClick={() => handleManualImport({ directory: true })}
                                    disabled={isActionLoading || isDownloading}
                                    className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/10 transition-colors"
                                    title={t("settings.model.emotion_model.reimport_dir_btn", { defaultValue: "重新导入目录" })}
                                >
                                    <FolderInput size={13} />
                                    {t("settings.model.emotion_model.reimport_dir_btn", { defaultValue: "重新导入目录" })}
                                </button>

                                <button
                                    onClick={handleOpenDirectory}
                                    className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] bg-white/5 hover:bg-white/10 border border-white/5 transition-colors"
                                >
                                    <FolderOpen size={13} />
                                    {t("settings.model.emotion_model.open_dir_btn", { defaultValue: "打开目录" })}
                                </button>

                                {/* Uninstall button */}
                                {!showUninstallConfirm ? (
                                    <button
                                        onClick={() => setShowUninstallConfirm(true)}
                                        disabled={isActionLoading || isDownloading}
                                        className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs text-[var(--color-text-muted)] hover:text-rose-400 hover:bg-rose-500/10 border border-transparent hover:border-rose-500/20 transition-colors"
                                    >
                                        <Trash2 size={13} />
                                        {t("settings.model.emotion_model.uninstall_btn", { defaultValue: "卸载模型" })}
                                    </button>
                                ) : (
                                    <div className="flex items-center gap-2 bg-rose-500/10 border border-rose-500/30 px-3 py-1.5 rounded-lg">
                                        <span className="text-xs text-rose-300 font-medium">
                                            {t("settings.model.emotion_model.confirm_prompt", { defaultValue: "确认卸载?" })}
                                        </span>
                                        <button
                                            onClick={handleUninstall}
                                            disabled={isActionLoading}
                                            className="px-2.5 py-1 text-xs rounded bg-rose-500 text-white font-medium hover:bg-rose-600 active:scale-95"
                                        >
                                            {t("common.confirm", { defaultValue: "确认" })}
                                        </button>
                                        <button
                                            onClick={() => setShowUninstallConfirm(false)}
                                            className="px-2.5 py-1 text-xs rounded bg-white/10 text-[var(--color-text-secondary)] hover:bg-white/15"
                                        >
                                            {t("common.cancel", { defaultValue: "取消" })}
                                        </button>
                                    </div>
                                )}
                            </>
                        )}
                    </div>

                    {status?.installed && (
                        <div className="text-[11px] text-[var(--color-text-muted)] font-mono">
                            {status.install_dir ? `已安装于: ${status.install_dir}` : "模型已就绪"}
                        </div>
                    )}
                </div>

                {/* Emotion Playground (Available when model is installed) */}
                {status?.installed && (
                    <div className="mt-4 pt-4 border-t border-[var(--color-border)]/50 space-y-3">
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-1.5">
                                <Sparkles size={14} className="text-[var(--color-accent)]" />
                                <span className={labelClasses}>
                                    {t("settings.model.emotion_model.playground_title", {
                                        defaultValue: "情感推理试炼场 (Live Playground)",
                                    })}
                                </span>
                            </div>
                            <span className="text-[11px] text-[var(--color-text-muted)]">
                                {t("settings.model.emotion_model.playground_desc", {
                                    defaultValue: "测试中文 8 维情感概率分布与 Live2D 表情联动",
                                })}
                            </span>
                        </div>

                        {/* Sample Prompt Chips */}
                        <div className="flex flex-wrap gap-1.5">
                            {samplePrompts.map((sample, idx) => (
                                <button
                                    key={idx}
                                    type="button"
                                    onClick={() => {
                                        setTestInput(sample);
                                        void handleTestInference(sample);
                                    }}
                                    className="text-[11px] px-2.5 py-1 rounded-full bg-white/5 hover:bg-[var(--color-accent)]/15 hover:text-[var(--color-accent)] text-[var(--color-text-secondary)] border border-white/5 hover:border-[var(--color-accent)]/30 transition-all text-left"
                                >
                                    {sample}
                                </button>
                            ))}
                        </div>

                        {/* Input and Action */}
                        <div className="flex gap-2">
                            <input
                                type="text"
                                value={testInput}
                                onChange={(e) => setTestInput(e.target.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                        e.preventDefault();
                                        void handleTestInference();
                                    }
                                }}
                                placeholder={t("settings.model.emotion_model.playground_placeholder", {
                                    defaultValue: "输入任意文本进行情感分析...",
                                })}
                                className={clsx(inputClasses, "py-2 text-xs")}
                            />
                            <button
                                onClick={() => handleTestInference()}
                                disabled={isInferring || !testInput.trim()}
                                className="flex items-center gap-1.5 px-4 py-2 rounded-md bg-[var(--color-accent)]/20 hover:bg-[var(--color-accent)]/30 text-[var(--color-accent)] border border-[var(--color-accent)]/30 font-medium text-xs whitespace-nowrap active:scale-95 disabled:opacity-50 transition-all"
                            >
                                {isInferring ? (
                                    <Loader2 size={13} className="animate-spin" />
                                ) : (
                                    <Send size={13} />
                                )}
                                {t("settings.model.emotion_model.playground_test_btn", {
                                    defaultValue: "测算情感",
                                })}
                            </button>
                        </div>

                        {/* Inference Results Visualization */}
                        <AnimatePresence>
                            {inferenceResult && (
                                <motion.div
                                    initial={{ opacity: 0, y: 5 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    exit={{ opacity: 0, y: -5 }}
                                    className="p-3.5 rounded-lg bg-black/30 border border-white/10 space-y-3"
                                >
                                    {/* Top summary row */}
                                    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-white/10 pb-2.5 text-xs">
                                        <div className="flex items-center gap-3">
                                            <div>
                                                <span className="text-[var(--color-text-muted)] mr-1.5">
                                                    {t("settings.model.emotion_model.dominant_label", { defaultValue: "主导情绪:" })}
                                                </span>
                                                <span className="font-bold text-[var(--color-accent)] text-sm">
                                                    {inferenceResult.label_zh}
                                                </span>
                                                <span className="text-[11px] font-mono text-[var(--color-text-muted)] ml-1">
                                                    ({inferenceResult.dominant_emotion})
                                                </span>
                                            </div>

                                            <div className="text-[11px] text-[var(--color-text-muted)]">
                                                <span>置信度: </span>
                                                <span className="font-mono text-emerald-400 font-semibold">
                                                    {(inferenceResult.confidence * 100).toFixed(1)}%
                                                </span>
                                            </div>
                                        </div>

                                        <div className="flex items-center gap-3">
                                            <span className="text-[11px] font-mono text-[var(--color-text-muted)]">
                                                耗时: {inferenceResult.latency_ms.toFixed(1)}ms
                                            </span>

                                            {inferenceResult.mapped_cue && (
                                                <button
                                                    type="button"
                                                    onClick={() => handlePlayMappedCue(inferenceResult.mapped_cue!)}
                                                    className="flex items-center gap-1.5 px-2.5 py-1 rounded bg-[var(--color-accent)]/15 border border-[var(--color-accent)]/30 text-[var(--color-accent)] text-[11px] font-semibold hover:bg-[var(--color-accent)]/25 active:scale-95 transition-all"
                                                >
                                                    <Play size={10} fill="currentColor" />
                                                    触发表情: {inferenceResult.mapped_cue}
                                                </button>
                                            )}
                                        </div>
                                    </div>

                                    {/* 8-Emotion Probability Bars */}
                                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 pt-1">
                                        {inferenceResult.probabilities.map((item, idx) => {
                                            const isTop = item.label === inferenceResult.dominant_emotion;
                                            const pct = Math.round(item.score * 100);
                                            return (
                                                <div key={idx} className="space-y-1">
                                                    <div className="flex justify-between items-center text-[11px]">
                                                        <span className={clsx(isTop ? "font-bold text-[var(--color-text-primary)]" : "text-[var(--color-text-secondary)]")}>
                                                            {item.label_zh}
                                                        </span>
                                                        <span className="font-mono text-[var(--color-text-muted)] text-[10px]">
                                                            {pct}%
                                                        </span>
                                                    </div>
                                                    <div className="w-full h-1.5 bg-black/40 rounded-full overflow-hidden border border-white/5">
                                                        <div
                                                            className={clsx(
                                                                "h-full rounded-full transition-all duration-300",
                                                                isTop
                                                                    ? "bg-[var(--color-accent)]"
                                                                    : "bg-white/20"
                                                            )}
                                                            style={{ width: `${Math.max(2, pct)}%` }}
                                                        />
                                                    </div>
                                                </div>
                                            );
                                        })}
                                    </div>
                                </motion.div>
                            )}
                        </AnimatePresence>
                    </div>
                )}
            </div>
        </div>
    );
}
