import { useState, useEffect, useRef } from "react";
import { clsx } from "clsx";
import { useTranslation, Trans } from "react-i18next";
import {
    Upload,
    FolderOpen,
    Trash2,
    Shuffle,
    Sparkles,
    X,
    CheckCircle2,
    AlertCircle,
    Database,
} from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { labelClasses, sectionHeadingClasses } from "../../styles/settings-primitives";
import type { BackgroundControls } from "../SettingsPanel";
import type { BackgroundConfig } from "../../hooks/useBackgroundSlideshow";

export interface BackgroundTabProps {
    bgConfig: BackgroundConfig;
    onBgConfigChange: (update: Partial<BackgroundConfig>) => void;
    backgroundControls: BackgroundControls;
}

export default function BackgroundTab({
    bgConfig,
    onBgConfigChange,
    backgroundControls: bg,
}: BackgroundTabProps) {
    const { t } = useTranslation();
    const fileInputRef = useRef<HTMLInputElement>(null);
    const folderInputRef = useRef<HTMLInputElement>(null);

    const [confirmClearing, setConfirmClearing] = useState(false);
    const [feedback, setFeedback] = useState<{ type: "success" | "error"; message: string } | null>(null);
    const feedbackTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    useEffect(() => {
        return () => {
            if (feedbackTimerRef.current) clearTimeout(feedbackTimerRef.current);
            if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
        };
    }, []);

    const showFeedback = (type: "success" | "error", message: string) => {
        if (feedbackTimerRef.current) clearTimeout(feedbackTimerRef.current);
        setFeedback({ type, message });
        feedbackTimerRef.current = setTimeout(() => {
            setFeedback(null);
            feedbackTimerRef.current = null;
        }, 3500);
    };

    const handleImport = async (fileList: FileList | null) => {
        if (!fileList || fileList.length === 0) return;
        try {
            const count = await bg.importFiles(fileList);
            if (count > 0) {
                showFeedback("success", t("settings.background.library.imported", { count }));
            } else {
                showFeedback("error", t("settings.background.library.no_valid_images"));
            }
        } catch (e) {
            showFeedback("error", String(e));
        }
    };

    const handleRemoveImage = async (index: number) => {
        try {
            await bg.removeImage(index);
            showFeedback("success", t("settings.background.library.deleted"));
        } catch (e) {
            showFeedback("error", String(e));
        }
    };

    const handleClearClick = async () => {
        if (!confirmClearing) {
            setConfirmClearing(true);
            if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
            confirmTimerRef.current = setTimeout(() => {
                setConfirmClearing(false);
                confirmTimerRef.current = null;
            }, 4000);
            return;
        }

        // Confirmed clear
        setConfirmClearing(false);
        if (confirmTimerRef.current) {
            clearTimeout(confirmTimerRef.current);
            confirmTimerRef.current = null;
        }
        try {
            await bg.clearImages();
            showFeedback("success", t("settings.background.library.cleared"));
        } catch (e) {
            showFeedback("error", String(e));
        }
    };

    return (
        <div className="space-y-6">
            {/* Hidden file inputs */}
            <input
                ref={fileInputRef}
                type="file"
                multiple
                accept="image/png,image/jpeg,image/webp,image/gif,image/bmp"
                className="hidden"
                onChange={async (e) => {
                    await handleImport(e.target.files);
                    e.target.value = "";
                }}
            />
            <input
                ref={folderInputRef}
                type="file"
                // @ts-expect-error webkitdirectory is non-standard but widely supported
                webkitdirectory=""
                multiple
                className="hidden"
                onChange={async (e) => {
                    await handleImport(e.target.files);
                    e.target.value = "";
                }}
            />

            {/* ── Section 1: Image Asset Library (Direct storage, immediate effect) ── */}
            <div className="p-4 bg-black/25 rounded-xl border border-[var(--color-border)] space-y-4">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <Database size={15} className="text-[var(--color-accent)]" />
                        <h3 className={sectionHeadingClasses}>
                            {t("settings.background.library.title")}
                        </h3>
                    </div>
                    <span className="text-[10px] px-2 py-0.5 rounded-full font-mono font-medium tracking-wide bg-[var(--color-accent)]/10 text-[var(--color-accent)] border border-[var(--color-accent)]/20">
                        {t("settings.background.library.badge")}
                    </span>
                </div>

                <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                    {t("settings.background.library.notice")}
                </p>

                {/* Import buttons and clear action */}
                <div className="flex flex-wrap items-center gap-2 pt-1">
                    <motion.button
                        whileHover={{ scale: 1.02 }}
                        whileTap={{ scale: 0.98 }}
                        onClick={() => fileInputRef.current?.click()}
                        className={clsx(
                            "flex items-center gap-2 px-3.5 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                        )}
                    >
                        <Upload size={13} strokeWidth={1.5} />
                        {t("settings.background.import.files")}
                    </motion.button>

                    <motion.button
                        whileHover={{ scale: 1.02 }}
                        whileTap={{ scale: 0.98 }}
                        onClick={() => folderInputRef.current?.click()}
                        className={clsx(
                            "flex items-center gap-2 px-3.5 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                        )}
                    >
                        <FolderOpen size={13} strokeWidth={1.5} />
                        {t("settings.background.import.folder")}
                    </motion.button>

                    {bg.imageCount > 0 && (
                        <div className="flex items-center gap-1.5 ml-auto">
                            <motion.button
                                whileHover={{ scale: 1.02 }}
                                whileTap={{ scale: 0.98 }}
                                onClick={handleClearClick}
                                className={clsx(
                                    "flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase transition-colors",
                                    confirmClearing
                                        ? "bg-red-500/20 text-red-400 border border-red-500/50 hover:bg-red-500/30"
                                        : "border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-red-500/40 hover:text-red-400"
                                )}
                            >
                                <Trash2 size={13} strokeWidth={1.5} />
                                {confirmClearing
                                    ? t("settings.background.library.clear_confirm")
                                    : t("settings.background.import.clear")}
                            </motion.button>
                            {confirmClearing && (
                                <motion.button
                                    initial={{ opacity: 0, scale: 0.9 }}
                                    animate={{ opacity: 1, scale: 1 }}
                                    onClick={() => {
                                        setConfirmClearing(false);
                                        if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
                                    }}
                                    className="px-2 py-1 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]"
                                >
                                    {t("settings.background.library.clear_cancel")}
                                </motion.button>
                            )}
                        </div>
                    )}
                </div>

                {/* Immediate feedback notification banner */}
                <AnimatePresence>
                    {feedback && (
                        <motion.div
                            initial={{ opacity: 0, y: -4 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -4 }}
                            className={clsx(
                                "flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-medium",
                                feedback.type === "success"
                                    ? "bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                                    : "bg-red-500/15 border border-red-500/30 text-red-300"
                            )}
                        >
                            {feedback.type === "success" ? (
                                <CheckCircle2 size={14} className="shrink-0 text-emerald-400" />
                            ) : (
                                <AlertCircle size={14} className="shrink-0 text-red-400" />
                            )}
                            <span className="flex-1">{feedback.message}</span>
                        </motion.div>
                    )}
                </AnimatePresence>

                {/* Loaded image count indicator */}
                <div className="text-xs text-[var(--color-text-muted)] pt-1 flex justify-between items-center">
                    <span>{t("settings.background.import.count", { count: bg.imageCount })}</span>
                </div>

                {/* Image preview grid */}
                {bg.imageCount > 0 ? (
                    <div className="grid grid-cols-4 gap-2 pt-1 max-h-64 overflow-y-auto pr-1">
                        {bg.images.map((url, i) => (
                            <div
                                key={i}
                                className="relative group rounded-lg overflow-hidden border border-[var(--color-border)] aspect-square bg-black/40"
                            >
                                <img src={url} alt="" className="w-full h-full object-cover" />
                                <motion.button
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleRemoveImage(i)}
                                    className="absolute top-1 right-1 p-1 rounded-full bg-black/75 text-red-400 opacity-0 group-hover:opacity-100 transition-opacity"
                                    aria-label="Remove image"
                                >
                                    <X size={12} strokeWidth={2} />
                                </motion.button>
                            </div>
                        ))}
                    </div>
                ) : (
                    <div className="text-center py-6 border border-dashed border-[var(--color-border)] rounded-lg text-xs text-[var(--color-text-muted)]">
                        {t("settings.background.library.empty")}
                    </div>
                )}
            </div>

            {/* ── Section 2: Display & Slideshow Preferences (Governed by global draft / Cancel) ── */}
            <div className="space-y-4 pt-1">
                <div className="flex items-center gap-2">
                    <h3 className={sectionHeadingClasses}>
                        {t("settings.background.preferences.title")}
                    </h3>
                </div>

                {/* Enable toggle */}
                <div className="flex items-center justify-between">
                    <label className={labelClasses}>{t("settings.background.enable")}</label>
                    <button
                        onClick={() => onBgConfigChange({ enabled: !bgConfig.enabled })}
                        className={clsx(
                            "w-10 h-6 rounded-full transition-colors relative",
                            bgConfig.enabled
                                ? "bg-[var(--color-accent)]"
                                : "bg-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: bgConfig.enabled ? 18 : 2 }}
                            transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            className="absolute top-1 w-4 h-4 rounded-full bg-white"
                        />
                    </button>
                </div>

                {/* Background Mode */}
                <div className="space-y-2">
                    <label className={labelClasses}>{t("settings.background.mode.label")}</label>
                    <div className="grid grid-cols-3 gap-2">
                        {(["slideshow", "generated", "static"] as const).map((mode) => (
                            <button
                                key={mode}
                                onClick={() => onBgConfigChange({ mode })}
                                className={clsx(
                                    "px-3 py-2 text-xs border rounded-lg transition-colors uppercase font-bold tracking-wider",
                                    (bgConfig.mode || "slideshow") === mode
                                        ? "bg-[var(--color-accent)] border-[var(--color-accent)] text-black"
                                        : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-secondary)]"
                                )}
                            >
                                {t(`settings.background.mode.${mode}`)}
                            </button>
                        ))}
                    </div>
                </div>

                {/* Mode-specific settings */}
                {(bgConfig.mode === "slideshow" || !bgConfig.mode) && (
                    <div className="space-y-5 p-4 bg-black/20 rounded-lg border border-[var(--color-border)]">
                        {/* Rotation Mode */}
                        <div className="flex items-center justify-between">
                            <label className={labelClasses}>
                                {t("settings.background.slideshow.rotation.label")}
                            </label>
                            <button
                                onClick={() =>
                                    onBgConfigChange({
                                        rotation: bgConfig.rotation === "sequential" ? "random" : "sequential",
                                    })
                                }
                                className="flex items-center gap-2 text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] transition-colors"
                            >
                                <Shuffle
                                    size={14}
                                    className={clsx(
                                        "transition-transform",
                                        bgConfig.rotation === "random" && "text-[var(--color-accent)]"
                                    )}
                                />
                                {bgConfig.rotation === "sequential"
                                    ? t("settings.background.slideshow.rotation.sequential")
                                    : t("settings.background.slideshow.rotation.random")}
                            </button>
                        </div>

                        {/* Interval */}
                        <div>
                            <div className="flex justify-between mb-2">
                                <label className={labelClasses}>
                                    {t("settings.background.slideshow.interval")}
                                </label>
                                <span className="text-xs text-[var(--color-text-secondary)]">
                                    {bgConfig.interval}s
                                </span>
                            </div>
                            <input
                                type="range"
                                min="5"
                                max="300"
                                step="5"
                                value={bgConfig.interval}
                                onChange={(e) => onBgConfigChange({ interval: Number(e.target.value) })}
                                className="w-full accent-[var(--color-accent)]"
                            />
                        </div>
                    </div>
                )}

                {bgConfig.mode === "generated" && (
                    <div className="p-4 bg-[var(--color-accent-subtle)]/10 border border-[var(--color-accent)]/20 rounded-lg">
                        <div className="flex items-start gap-3">
                            <Sparkles className="text-[var(--color-accent)] mt-0.5" size={16} />
                            <div>
                                <p className="text-sm font-semibold text-[var(--color-text-primary)] mb-1">
                                    {t("settings.background.generated.title")}
                                </p>
                                <p className="text-xs text-[var(--color-text-secondary)]">
                                    <Trans
                                        i18nKey="settings.background.generated.desc"
                                        components={{ strong: <strong />, br: <br /> }}
                                    />
                                </p>
                            </div>
                        </div>
                    </div>
                )}

                {/* Blur Settings (Global) */}
                <div>
                    <div className="flex items-center justify-between mb-2">
                        <label className={labelClasses}>{t("settings.background.blur.label")}</label>
                        <button
                            onClick={() => onBgConfigChange({ blur: !bgConfig.blur })}
                            className={clsx(
                                "w-10 h-6 rounded-full transition-colors relative",
                                bgConfig.blur
                                    ? "bg-[var(--color-accent)]"
                                    : "bg-[var(--color-border)]"
                            )}
                        >
                            <motion.div
                                animate={{ x: bgConfig.blur ? 18 : 2 }}
                                transition={{ type: "spring", stiffness: 500, damping: 30 }}
                                className="absolute top-1 w-4 h-4 rounded-full bg-white"
                            />
                        </button>
                    </div>
                    {bgConfig.blur && (
                        <div className="mt-3">
                            <div className="flex justify-between mb-2">
                                <label className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wider">
                                    {t("settings.background.blur.intensity")}
                                </label>
                                <span className="text-xs text-[var(--color-text-secondary)]">
                                    {bgConfig.blurAmount}px
                                </span>
                            </div>
                            <input
                                type="range"
                                min="2"
                                max="30"
                                step="1"
                                value={bgConfig.blurAmount}
                                onChange={(e) => onBgConfigChange({ blurAmount: Number(e.target.value) })}
                                className="w-full accent-[var(--color-accent)]"
                            />
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
}
