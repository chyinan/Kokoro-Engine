import { useState, useEffect } from "react";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { FolderOpen, RefreshCw, AlertCircle, Trash2, Check } from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { open } from "@tauri-apps/plugin-dialog";
import { exists } from "@tauri-apps/plugin-fs";
import { labelClasses } from "../../styles/settings-primitives";
import { importLive2dZip, listLive2dModels, deleteLive2dModel } from "../../../lib/kokoro-bridge";
import type { Live2dModelInfo } from "../../../lib/kokoro-bridge";
import type { Live2DDisplayMode } from "../../../features/live2d/Live2DViewer";

export interface ModelTabProps {
    displayMode: Live2DDisplayMode;
    onDisplayModeChange: (mode: Live2DDisplayMode) => void;
    customModelPath: string | null;
    onCustomModelPathChange: (path: string | null) => void;
}

export default function ModelTab({
    displayMode, onDisplayModeChange,
    customModelPath, onCustomModelPathChange,
}: ModelTabProps) {
    const { t } = useTranslation();
    const [isImporting, setIsImporting] = useState(false);
    const [models, setModels] = useState<Live2dModelInfo[]>([]);
    const [isLoadingModels, setIsLoadingModels] = useState(false);

    // Fetch available models on mount
    useEffect(() => {
        fetchModels();
    }, []);

    const fetchModels = async () => {
        setIsLoadingModels(true);
        try {
            const list = await listLive2dModels();
            setModels(list);
        } catch (e) {
            console.error("Failed to list Live2D models:", e);
        } finally {
            setIsLoadingModels(false);
        }
    };

    const handleImportModel = async () => {
        try {
            const selected = await open({
                multiple: false,
                filters: [
                    { name: 'Live2D Package (zip)', extensions: ['zip'] },
                    { name: 'Live2D Model', extensions: ['model3.json'] },
                ]
            });

            if (selected && typeof selected === 'string') {
                if (selected.toLowerCase().endsWith('.zip')) {
                    setIsImporting(true);
                    try {
                        const modelPath = await importLive2dZip(selected);
                        onCustomModelPathChange(modelPath);
                        // Refresh model list after import
                        await fetchModels();
                    } catch (e) {
                        console.error("Failed to import Live2D zip:", e);
                    } finally {
                        setIsImporting(false);
                    }
                } else {
                    const fileExists = await exists(selected);
                    if (fileExists) {
                        onCustomModelPathChange(selected);
                    } else {
                        console.error("Selected file does not exist:", selected);
                    }
                }
            }
        } catch (err) {
            console.error("Failed to open file dialog:", err);
        }
    };

    const handleDeleteModel = async (modelName: string, modelPath: string) => {
        try {
            await deleteLive2dModel(modelName);
            // If we deleted the currently active model, switch to default
            if (customModelPath === modelPath) {
                onCustomModelPathChange(null);
            }
            // Refresh list
            await fetchModels();
        } catch (e) {
            console.error("Failed to delete model:", e);
        }
    };

    const isSelected = (path: string | null) => {
        return customModelPath === path;
    };

    const displayModes = [
        { mode: "full" as Live2DDisplayMode, label: t("settings.model.display_mode.full"), desc: t("settings.model.display_mode.full_desc") },
        { mode: "upper-thigh" as Live2DDisplayMode, label: t("settings.model.display_mode.upper_thigh"), desc: t("settings.model.display_mode.upper_thigh_desc") },
        { mode: "upper" as Live2DDisplayMode, label: t("settings.model.display_mode.upper"), desc: t("settings.model.display_mode.upper_desc") },
    ];

    return (
        <div className="space-y-5">
            <div>
                <label className={labelClasses}>{t("settings.model.display_mode.label")}</label>
                <p className="text-xs text-[var(--color-text-muted)] mb-3">
                    {t("settings.model.display_mode.desc")}
                </p>
                <div className="grid grid-cols-3 gap-3">
                    {displayModes.map(({ mode, label, desc }) => (
                        <motion.button
                            key={mode}
                            whileHover={{ scale: 1.02 }}
                            whileTap={{ scale: 0.97 }}
                            onClick={() => onDisplayModeChange(mode)}
                            className={clsx(
                                "flex flex-col items-center gap-2 p-4 rounded-xl border text-center transition-colors",
                                displayMode === mode
                                    ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40 text-[var(--color-accent)]"
                                    : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:border-[var(--color-text-muted)]"
                            )}
                        >
                            {/* Visual indicator */}
                            <div className="h-14 flex items-center justify-center">
                                <div className={clsx(
                                    "w-8 rounded-sm border-2 transition-colors",
                                    displayMode === mode ? "border-[var(--color-accent)]" : "border-[var(--color-border)]",
                                    mode === "full" ? "h-14" : mode === "upper-thigh" ? "h-10" : "h-7"
                                )} />
                            </div>
                            <span className="text-sm font-heading font-semibold tracking-wider uppercase">{label}</span>
                            <span className="text-[10px] text-[var(--color-text-muted)]">{desc}</span>
                        </motion.button>
                    ))}
                </div>
            </div>

            {/* Model List Section */}
            <div>
                <div className="flex items-center justify-between mb-2">
                    <label className={labelClasses}>{t("settings.model.list.label")}</label>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.9 }}
                        onClick={fetchModels}
                        disabled={isLoadingModels}
                        className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-accent)] hover:bg-[var(--color-accent-subtle)] transition-colors"
                        title="Refresh model list"
                    >
                        <RefreshCw size={14} strokeWidth={1.5} className={isLoadingModels ? "animate-spin" : ""} />
                    </motion.button>
                </div>
                <div className="space-y-2 max-h-[220px] overflow-y-auto scrollable pr-1">
                    {/* Default Model (always present) */}

                    <div
                        className={clsx(
                            "flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-all",
                            isSelected(null)
                                ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40"
                                : "bg-black/20 border-[var(--color-border)] hover:border-[var(--color-text-muted)]"
                        )}
                    >
                        <motion.button
                            whileHover={{ scale: 1.01 }}
                            whileTap={{ scale: 0.99 }}
                            onClick={() => onCustomModelPathChange(null)}
                            className="flex items-center gap-3 flex-1 min-w-0 text-left"
                        >
                            {/* Selection indicator */}
                            <div className={clsx(
                                "w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-colors",
                                isSelected(null)
                                    ? "border-[var(--color-accent)] bg-[var(--color-accent)]"
                                    : "border-[var(--color-border)]"
                            )}>
                                {isSelected(null) && <Check size={12} strokeWidth={3} className="text-black" />}
                            </div>
                            <div className="flex-1 min-w-0">
                                <span className={clsx(
                                    "text-sm font-medium truncate block",
                                    isSelected(null) ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                )}>
                                    {t("settings.model.list.default")}
                                </span>
                                <span className="text-[10px] text-[var(--color-text-muted)]">{t("settings.model.list.builtin")}</span>
                            </div>
                        </motion.button>
                    </div>

                    {/* Imported Models */}
                    <AnimatePresence>
                        {models.map((model) => (
                            <motion.div
                                key={model.name}
                                initial={{ opacity: 0, y: -10 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className={clsx(
                                    "flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-all",
                                    isSelected(model.path)
                                        ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40"
                                        : "bg-black/20 border-[var(--color-border)] hover:border-[var(--color-text-muted)]"
                                )}
                            >
                                <motion.button
                                    whileHover={{ scale: 1.01 }}
                                    whileTap={{ scale: 0.99 }}
                                    onClick={() => onCustomModelPathChange(model.path)}
                                    className="flex items-center gap-3 flex-1 min-w-0 text-left"
                                >
                                    {/* Selection indicator */}
                                    <div className={clsx(
                                        "w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-colors",
                                        isSelected(model.path)
                                            ? "border-[var(--color-accent)] bg-[var(--color-accent)]"
                                            : "border-[var(--color-border)]"
                                    )}>
                                        {isSelected(model.path) && <Check size={12} strokeWidth={3} className="text-black" />}
                                    </div>
                                    <div className="flex-1 min-w-0">
                                        <span className={clsx(
                                            "text-sm font-medium truncate block",
                                            isSelected(model.path) ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                        )}>
                                            {model.name}
                                        </span>
                                        <span className="text-[10px] text-[var(--color-text-muted)] truncate block">{model.path}</span>
                                    </div>
                                </motion.button>
                                {/* Delete button */}
                                <motion.button
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleDeleteModel(model.name, model.path)}
                                    className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-error)] hover:bg-[var(--color-error)]/10 transition-colors flex-shrink-0"
                                    title={`Delete ${model.name}`}
                                >
                                    <Trash2 size={14} strokeWidth={1.5} />
                                </motion.button>
                            </motion.div>
                        ))}
                    </AnimatePresence>

                    {models.length === 0 && !isLoadingModels && (
                        <p className="text-xs text-[var(--color-text-muted)] text-center py-2 italic">
                            {t("settings.model.list.empty")}
                        </p>
                    )}
                </div>

                {/* Import button */}
                <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    onClick={handleImportModel}
                    disabled={isImporting}
                    className={clsx(
                        "w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase mt-3",
                        "bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                        isImporting && "opacity-60 cursor-not-allowed"
                    )}
                >
                    {isImporting ? (
                        <RefreshCw size={16} strokeWidth={1.5} className="animate-spin" />
                    ) : (
                        <FolderOpen size={16} strokeWidth={1.5} />
                    )}
                    {isImporting ? t("settings.model.import.loading") : t("settings.model.import.button")}
                </motion.button>

                <div className="mt-3 flex items-start gap-2 text-[10px] text-[var(--color-text-muted)] bg-[var(--color-accent-subtle)]/50 p-2 rounded">
                    <AlertCircle size={12} className="mt-0.5 text-[var(--color-accent)]" />
                    <p>
                        {t("settings.model.import.hint")}
                    </p>
                </div>
            </div>
        </div>
    );
}
