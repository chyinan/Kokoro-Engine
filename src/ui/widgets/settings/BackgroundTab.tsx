import { useRef } from "react";
import { clsx } from "clsx";
import { useTranslation, Trans } from "react-i18next";
import { Upload, FolderOpen, Trash2, Shuffle, Sparkles, X } from "lucide-react";
import { motion } from "framer-motion";
import { labelClasses } from "../../styles/settings-primitives";
import type { BackgroundControls } from "../SettingsPanel";
import type { BackgroundConfig } from "../../hooks/useBackgroundSlideshow";

export interface BackgroundTabProps {
    bgConfig: BackgroundConfig;
    onBgConfigChange: (update: Partial<BackgroundConfig>) => void;
    backgroundControls: BackgroundControls;
}

export default function BackgroundTab({
    bgConfig, onBgConfigChange, backgroundControls: bg,
}: BackgroundTabProps) {
    const { t } = useTranslation();
    const fileInputRef = useRef<HTMLInputElement>(null);
    const folderInputRef = useRef<HTMLInputElement>(null);

    return (
        <div className="space-y-5">
            {/* Hidden file inputs */}
            <input
                ref={fileInputRef}
                type="file"
                multiple
                accept="image/png,image/jpeg,image/webp,image/gif,image/bmp"
                className="hidden"
                onChange={e => { if (e.target.files) bg.importFiles(e.target.files); e.target.value = ""; }}
            />
            <input
                ref={folderInputRef}
                type="file"
                // @ts-expect-error webkitdirectory is non-standard but widely supported
                webkitdirectory=""
                multiple
                accept="image/*"
                className="hidden"
                onChange={e => { if (e.target.files) bg.importFiles(e.target.files); e.target.value = ""; }}
            />

            {/* Import buttons */}
            <div>
                <label className={labelClasses}>{t("settings.background.import.label")}</label>
                <div className="flex gap-2">
                    <motion.button
                        whileHover={{ scale: 1.02 }}
                        whileTap={{ scale: 0.98 }}
                        onClick={() => fileInputRef.current?.click()}
                        className={clsx(
                            "flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                        )}
                    >
                        <Upload size={14} strokeWidth={1.5} />
                        {t("settings.background.import.files")}
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.02 }}
                        whileTap={{ scale: 0.98 }}
                        onClick={() => folderInputRef.current?.click()}
                        className={clsx(
                            "flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase",
                            "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                            "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                        )}
                    >
                        <FolderOpen size={14} strokeWidth={1.5} />
                        {t("settings.background.import.folder")}
                    </motion.button>
                    {bg.imageCount > 0 && (
                        <motion.button
                            whileHover={{ scale: 1.02 }}
                            whileTap={{ scale: 0.98 }}
                            onClick={bg.clearImages}
                            className="flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase text-[var(--color-text-muted)] hover:text-[var(--color-error)] transition-colors"
                        >
                            <Trash2 size={14} strokeWidth={1.5} />
                            {t("settings.background.import.clear")}
                        </motion.button>
                    )}
                </div>
                <p className="text-xs text-[var(--color-text-muted)] mt-2">
                    {t("settings.background.import.count", { count: bg.imageCount })}
                </p>
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
                    {(["slideshow", "generated", "static"] as const).map(mode => (
                        <button
                            key={mode}
                            onClick={() => onBgConfigChange({ mode })}
                            className={clsx(
                                "px-3 py-2 text-xs border rounded-lg transition-colors uppercase font-bold tracking-wider",
                                (bgConfig.mode || "slideshow") === mode
                                    ? "bg-[var(--color-accent)] border-[var(--color-accent)] text-white"
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
                        <label className={labelClasses}>{t("settings.background.slideshow.rotation.label")}</label>
                        <button
                            onClick={() => onBgConfigChange({ rotation: bgConfig.rotation === "sequential" ? "random" : "sequential" })}
                            className="flex items-center gap-2 text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] transition-colors"
                        >
                            <Shuffle size={14} className={clsx("transition-transform", bgConfig.rotation === "random" && "text-[var(--color-accent)]")} />
                            {bgConfig.rotation === "sequential"
                                ? t("settings.background.slideshow.rotation.sequential")
                                : t("settings.background.slideshow.rotation.random")}
                        </button>
                    </div>

                    {/* Interval */}
                    <div>
                        <div className="flex justify-between mb-2">
                            <label className={labelClasses}>{t("settings.background.slideshow.interval")}</label>
                            <span className="text-xs text-[var(--color-text-secondary)]">{bgConfig.interval}s</span>
                        </div>
                        <input
                            type="range"
                            min="5"
                            max="300"
                            step="5"
                            value={bgConfig.interval}
                            onChange={e => onBgConfigChange({ interval: Number(e.target.value) })}
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
                                <Trans i18nKey="settings.background.generated.desc" components={{ strong: <strong />, br: <br /> }} />
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
                            <label className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wider">{t("settings.background.blur.intensity")}</label>
                            <span className="text-xs text-[var(--color-text-secondary)]">{bgConfig.blurAmount}px</span>
                        </div>
                        <input
                            type="range"
                            min="2"
                            max="30"
                            step="1"
                            value={bgConfig.blurAmount}
                            onChange={e => onBgConfigChange({ blurAmount: Number(e.target.value) })}
                            className="w-full accent-[var(--color-accent)]"
                        />
                    </div>
                )}
            </div>

            <div className="border-t border-[var(--color-border)] pt-4" />

            {/* Image preview grid */}
            {bg.imageCount > 0 && (
                <div>
                    <label className={labelClasses}>{t("settings.background.preview.label")}</label>
                    <div className="grid grid-cols-4 gap-2">
                        {bg.images.map((url, i) => (
                            <div key={i} className="relative group rounded-lg overflow-hidden border border-[var(--color-border)] aspect-square">
                                <img src={url} alt="" className="w-full h-full object-cover" />
                                <motion.button
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => bg.removeImage(i)}
                                    className="absolute top-1 right-1 p-1 rounded-full bg-black/70 text-red-400 opacity-0 group-hover:opacity-100 transition-opacity"
                                    aria-label="Remove image"
                                >
                                    <X size={12} strokeWidth={2} />
                                </motion.button>
                            </div>
                        ))}
                    </div>
                </div>
            )}
        </div>
    );
}
