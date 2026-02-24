import { useEffect, useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { RefreshCw, Play, Eye, EyeOff, Package, AlertCircle, Import } from "lucide-react";
import { listMods, loadMod, installMod, type ModManifest } from "../../lib/kokoro-bridge";
import { open } from "@tauri-apps/plugin-dialog";
import { IframeSandbox } from "./IframeSandbox";
import { useTranslation, Trans } from "react-i18next";

export function ModList() {
    const { t } = useTranslation();
    const [mods, setMods] = useState<ModManifest[]>([]);
    const [loading, setLoading] = useState(false);
    const [activeUIMods, setActiveUIMods] = useState<string[]>([]);
    const [loadingMod, setLoadingMod] = useState<string | null>(null);
    const [status, setStatus] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);

    const showStatus = useCallback((text: string, type: "success" | "error" | "info" = "info") => {
        setStatus({ text, type });
        setTimeout(() => setStatus(null), 3000);
    }, []);

    const refreshMods = async () => {
        setLoading(true);
        try {
            const list = await listMods();
            setMods(list);
            setStatus(null);
        } catch (e) {
            console.error("Failed to list mods:", e);
            showStatus(t("mods.status.failed_list"), "error");
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        refreshMods();
    }, []);

    const handleLoad = async (id: string) => {
        setLoadingMod(id);
        try {
            await loadMod(id);
            showStatus(t("mods.status.loaded", { id }), "success");
        } catch (e) {
            console.error("Failed to load mod:", e);
            showStatus(t("mods.status.failed_load", { id }), "error");
        } finally {
            setLoadingMod(null);
        }
    };

    const handleImportMod = async () => {
        try {
            const selected = await open({
                multiple: false,
                filters: [{
                    name: 'Mod Archive',
                    extensions: ['zip']
                }]
            });

            if (selected && typeof selected === 'string') {
                showStatus(t("mods.status.installing"), "info");
                // installMod returns the manifest of the installed mod
                const manifest = await installMod(selected);
                showStatus(t("mods.status.installed", { name: manifest.name }), "success");
                refreshMods();
            }
        } catch (e) {
            console.error("Failed to import mod:", e);
            showStatus(typeof e === 'string' ? e : t("mods.status.failed_import"), "error");
        }
    };

    const toggleUI = (mod: ModManifest) => {
        if (!mod.ui_entry) return;
        if (activeUIMods.includes(mod.id)) {
            setActiveUIMods(prev => prev.filter(mid => mid !== mod.id));
        } else {
            setActiveUIMods(prev => [...prev, mod.id]);
        }
    };

    return (
        <div className={clsx(
            "flex flex-col h-full w-full",
            "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
            "border border-[var(--color-border)] rounded-xl shadow-lg",
            "relative overflow-hidden"
        )}>
            {/* Toast */}
            <AnimatePresence>
                {status && (
                    <motion.div
                        initial={{ opacity: 0, x: 20 }}
                        animate={{ opacity: 1, x: 0 }}
                        exit={{ opacity: 0, x: 20 }}
                        className={clsx(
                            "absolute top-2 right-2 z-[110] flex items-center gap-2 px-3 py-2 rounded-lg text-xs shadow-lg",
                            status.type === "error"
                                ? "bg-red-900/80 border border-red-500/50 text-red-200"
                                : status.type === "success"
                                    ? "bg-emerald-900/80 border border-emerald-500/50 text-emerald-200"
                                    : "bg-blue-900/80 border border-blue-500/50 text-blue-200"
                        )}
                    >
                        <AlertCircle size={14} strokeWidth={1.5} />
                        {status.text}
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
                <div className="flex items-center gap-2">
                    <Package size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <h2 className="font-heading text-lg font-bold tracking-widest uppercase text-[var(--color-accent)] drop-shadow-[var(--glow-accent)]">
                        {t("mods.title")}
                    </h2>
                </div>
                <div className="flex gap-2">
                    <motion.button
                        whileHover={{ scale: 1.05 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={handleImportMod}
                        className="flex items-center gap-1 px-3 py-1.5 rounded bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] hover:border-[var(--color-accent)] transition-colors"
                    >
                        <Import size={12} strokeWidth={1.5} />
                        {t("mods.actions.import")}
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={refreshMods}
                        disabled={loading}
                        className={clsx(
                            "p-2 rounded-md text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] transition-colors",
                            loading && "animate-spin"
                        )}
                        aria-label={t("mods.actions.refresh")}
                    >
                        <RefreshCw size={16} strokeWidth={1.5} />
                    </motion.button>
                </div>
            </div>

            {/* Mod List */}
            <div className="flex-1 overflow-y-auto p-4 space-y-3 scrollable">
                <AnimatePresence initial={false}>
                    {mods.map(mod => (
                        <motion.div
                            key={mod.id}
                            initial={{ opacity: 0, y: 10, scale: 0.95 }}
                            animate={{ opacity: 1, y: 0, scale: 1 }}
                            transition={{ duration: 0.3 }}
                            className={clsx(
                                "p-4 rounded-lg",
                                "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
                                "hover:border-[var(--color-border-accent)] transition-colors"
                            )}
                        >
                            <div className="flex justify-between items-start mb-2">
                                <div className="flex-1 min-w-0">
                                    <h3 className="font-heading font-semibold text-sm tracking-wide text-[var(--color-text-primary)] truncate">
                                        {mod.name}
                                    </h3>
                                    <p className="text-[10px] font-mono text-[var(--color-text-muted)] mt-0.5">
                                        v{mod.version}
                                    </p>
                                </div>
                                <div className="flex items-center gap-1.5 ml-2 shrink-0">
                                    {(mod.entry || (mod.scripts && mod.scripts.length > 0) || mod.theme || mod.layout || (mod.components && Object.keys(mod.components).length > 0)) && (
                                        <motion.button
                                            whileHover={{ scale: 1.05 }}
                                            whileTap={{ scale: 0.95 }}
                                            onClick={() => handleLoad(mod.id)}
                                            disabled={loadingMod === mod.id}
                                            className={clsx(
                                                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-heading font-semibold tracking-wider uppercase",
                                                "bg-[var(--color-accent)] text-black hover:bg-white transition-colors",
                                                loadingMod === mod.id && "opacity-50 cursor-not-allowed"
                                            )}
                                        >
                                            <Play size={12} strokeWidth={2} />
                                            {loadingMod === mod.id ? t("mods.actions.loading") : t("mods.actions.load")}
                                        </motion.button>
                                    )}
                                    {mod.ui_entry && (
                                        <motion.button
                                            whileHover={{ scale: 1.05 }}
                                            whileTap={{ scale: 0.95 }}
                                            onClick={() => toggleUI(mod)}
                                            className={clsx(
                                                "p-1.5 rounded-md transition-colors",
                                                activeUIMods.includes(mod.id)
                                                    ? "text-[var(--color-accent)] bg-[var(--color-accent-subtle)]"
                                                    : "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                            )}
                                            aria-label={activeUIMods.includes(mod.id) ? t("mods.actions.hide_ui") : t("mods.actions.show_ui")}
                                        >
                                            {activeUIMods.includes(mod.id) ? (
                                                <EyeOff size={14} strokeWidth={1.5} />
                                            ) : (
                                                <Eye size={14} strokeWidth={1.5} />
                                            )}
                                        </motion.button>
                                    )}
                                </div>
                            </div>

                            {mod.description && (
                                <p className="text-xs text-[var(--color-text-muted)] leading-relaxed mt-1">
                                    {mod.description}
                                </p>
                            )}

                            {/* Capability badges */}
                            <div className="flex flex-wrap gap-1.5 mt-2">
                                {mod.theme && (
                                    <span className="px-1.5 py-0.5 text-[9px] font-mono uppercase tracking-wider rounded bg-purple-900/40 border border-purple-500/30 text-purple-300">
                                        theme
                                    </span>
                                )}
                                {mod.layout && (
                                    <span className="px-1.5 py-0.5 text-[9px] font-mono uppercase tracking-wider rounded bg-blue-900/40 border border-blue-500/30 text-blue-300">
                                        layout
                                    </span>
                                )}
                                {mod.scripts && mod.scripts.length > 0 && (
                                    <span className="px-1.5 py-0.5 text-[9px] font-mono uppercase tracking-wider rounded bg-amber-900/40 border border-amber-500/30 text-amber-300">
                                        scripts
                                    </span>
                                )}
                                {mod.components && Object.keys(mod.components).length > 0 && (
                                    <span className="px-1.5 py-0.5 text-[9px] font-mono uppercase tracking-wider rounded bg-emerald-900/40 border border-emerald-500/30 text-emerald-300">
                                        {t("mods.badges.components", { count: Object.keys(mod.components).length })}
                                    </span>
                                )}
                            </div>

                            {/* Mod UI iframe */}
                            <AnimatePresence>
                                {activeUIMods.includes(mod.id) && mod.ui_entry && (
                                    <motion.div
                                        initial={{ height: 0, opacity: 0 }}
                                        animate={{ height: 256, opacity: 1 }}
                                        exit={{ height: 0, opacity: 0 }}
                                        transition={{ duration: 0.3 }}
                                        className="mt-3 rounded-md overflow-hidden border border-[var(--color-border)]"
                                    >
                                        <IframeSandbox
                                            id={mod.id}
                                            src={`mod://${mod.id}/${mod.ui_entry}`}
                                            permissions={mod.permissions ?? []}
                                        />
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </motion.div>
                    ))}
                </AnimatePresence>

                {/* Empty state */}
                {mods.length === 0 && !loading && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        className="flex flex-col items-center justify-center py-12 text-center"
                    >
                        <Package size={32} strokeWidth={1} className="text-[var(--color-text-muted)] mb-3 opacity-50" />
                        <p className="text-sm text-[var(--color-text-muted)]">{t("mods.empty.title")}</p>
                        <p className="text-xs text-[var(--color-text-muted)] mt-1 opacity-60">
                            <Trans i18nKey="mods.empty.description">
                                Add mods to the <span className="font-mono">mods/</span> directory
                            </Trans>
                        </p>
                    </motion.div>
                )}

                {/* Loading state */}
                {loading && (
                    <div className="flex items-center justify-center py-8">
                        <RefreshCw size={20} className="text-[var(--color-accent)] animate-spin" />
                    </div>
                )}
            </div>
        </div>
    );
};
