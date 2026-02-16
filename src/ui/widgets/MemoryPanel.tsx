import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { Trash2, Pencil, Check, X, Search, Brain, ChevronDown, List, Calendar, Share2, UserCircle } from "lucide-react";
import { inputClasses } from "../styles/settings-primitives";
import { listMemories, updateMemory, deleteMemory } from "../../lib/kokoro-bridge";
import type { MemoryRecord } from "../../lib/kokoro-bridge";
import { MemoryTimeline } from "./memory/MemoryTimeline";
import { MemoryGraph } from "./memory/MemoryGraph";
import { characterDb, type CharacterProfile } from "../../lib/db";

interface MemoryPanelProps {
    characterId: string;
}

type ViewMode = "list" | "timeline" | "graph";

export default function MemoryPanel({ characterId }: MemoryPanelProps) {
    const { t } = useTranslation();
    const [view, setView] = useState<ViewMode>("list");
    const [memories, setMemories] = useState<MemoryRecord[]>([]);
    const [total, setTotal] = useState(0);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const [editingId, setEditingId] = useState<number | null>(null);
    const [editContent, setEditContent] = useState("");
    const [editImportance, setEditImportance] = useState(0.5);
    const [deletingId, setDeletingId] = useState<number | null>(null);
    const [page, setPage] = useState(0);
    const pageSize = 50; // Load more for graph/timeline

    // ── Character selector state ──
    const [characters, setCharacters] = useState<CharacterProfile[]>([]);
    const [selectedCharId, setSelectedCharId] = useState<string>(characterId);

    // Load character list for the dropdown
    useEffect(() => {
        characterDb.getAll().then((all) => {
            setCharacters(all);
            console.log("[MemoryPanel] Characters loaded:", all.map(c => ({ id: c.id, name: c.name })));
            console.log("[MemoryPanel] Prop characterId:", characterId);
            // If the provided characterId matches an existing character, keep it
            // Otherwise default to the first character
            if (!all.find((c) => String(c.id) === characterId) && all.length > 0) {
                setSelectedCharId(String(all[0].id));
            }
        }).catch((e) => console.error("[MemoryPanel] Failed to load characters:", e));
    }, [characterId]);

    // Reset page when switching characters
    useEffect(() => {
        setPage(0);
        setSearchQuery("");
    }, [selectedCharId]);

    const fetchMemories = useCallback(async () => {
        console.log("[MemoryPanel] fetchMemories called with selectedCharId:", selectedCharId);
        if (!selectedCharId) return;
        setLoading(true);
        try {
            const res = await listMemories(selectedCharId, pageSize, page * pageSize);
            setMemories(res.memories);
            setTotal(res.total);
        } catch (e) {
            console.error("[MemoryPanel] Failed to load memories:", e);
        } finally {
            setLoading(false);
        }
    }, [selectedCharId, page]);

    useEffect(() => {
        fetchMemories();
    }, [fetchMemories]);

    // Filter client-side by search query (simple text match)
    const filtered = searchQuery.trim()
        ? memories.filter((m) =>
            m.content.toLowerCase().includes(searchQuery.toLowerCase())
        )
        : memories;

    const startEdit = (mem: MemoryRecord) => {
        setEditingId(mem.id);
        setEditContent(mem.content);
        setEditImportance(mem.importance);
    };

    const saveEdit = async () => {
        if (editingId === null) return;
        try {
            await updateMemory(editingId, editContent, editImportance);
            setEditingId(null);
            fetchMemories();
        } catch (e) {
            console.error("[MemoryPanel] Failed to update memory:", e);
        }
    };

    const confirmDelete = async (id: number) => {
        try {
            await deleteMemory(id);
            setDeletingId(null);
            fetchMemories();
        } catch (e) {
            console.error("[MemoryPanel] Failed to delete memory:", e);
        }
    };

    const hasMore = (page + 1) * pageSize < total;

    // Helpers
    const getTimeAgo = (ts: number) => {
        const now = Date.now() / 1000;
        const diff = now - ts;
        if (diff < 60) return t("settings.memory.time.just_now");
        if (diff < 3600) return t("settings.memory.time.minutes_ago", { count: Math.floor(diff / 60) });
        if (diff < 86400) return t("settings.memory.time.hours_ago", { count: Math.floor(diff / 3600) });
        if (diff < 604800) return t("settings.memory.time.days_ago", { count: Math.floor(diff / 86400) });
        return new Date(ts * 1000).toLocaleDateString();
    };

    const getImportanceLabel = (v: number) => {
        if (v >= 0.7) return t("settings.memory.importance.high");
        if (v >= 0.4) return t("settings.memory.importance.med");
        return t("settings.memory.importance.low");
    };

    const getImportanceColor = (v: number) => {
        if (v >= 0.7) return "text-red-400 bg-red-500/15 border-red-500/30";
        if (v >= 0.4) return "text-amber-400 bg-amber-500/15 border-amber-500/30";
        return "text-emerald-400 bg-emerald-500/15 border-emerald-500/30";
    };

    return (
        <div className="space-y-4 h-full flex flex-col">
            {/* Header & Character Selector */}
            <div className="flex flex-col gap-4 shrink-0">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <Brain
                            size={16}
                            className="text-[var(--color-accent)]"
                        />
                        <span className="text-xs font-heading font-bold uppercase tracking-wider text-[var(--color-text-muted)]">
                            {t("settings.memory.title")}
                        </span>
                    </div>
                    <span className="text-[10px] font-mono text-[var(--color-text-muted)]">
                        {t("settings.memory.count", { count: total })}
                    </span>
                </div>

                {/* Character Selector */}
                {characters.length > 0 && (
                    <div className="relative">
                        <UserCircle
                            size={14}
                            className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)]"
                        />
                        <select
                            value={selectedCharId}
                            onChange={(e) => setSelectedCharId(e.target.value)}
                            className={clsx(inputClasses, "pl-9 py-2 appearance-none cursor-pointer")}
                        >
                            {characters.map((char) => (
                                <option key={char.id} value={String(char.id)}>
                                    {char.name}
                                    {String(char.id) === characterId ? ` ${t("settings.memory.active_char")}` : ""}
                                </option>
                            ))}
                        </select>
                        <ChevronDown
                            size={14}
                            className="absolute right-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] pointer-events-none"
                        />
                    </div>
                )}

                <div className="flex bg-black/20 rounded-lg p-1 gap-1 border border-white/5">
                    {[
                        { id: "list", label: t("settings.memory.tabs.list"), icon: List },
                        { id: "timeline", label: t("settings.memory.tabs.timeline"), icon: Calendar },
                        { id: "graph", label: t("settings.memory.tabs.graph"), icon: Share2 },
                    ].map(tab => (
                        <button
                            key={tab.id}
                            onClick={() => setView(tab.id as ViewMode)}
                            className={clsx(
                                "flex-1 flex items-center justify-center gap-2 py-1.5 rounded text-xs transition-colors",
                                view === tab.id
                                    ? "bg-[var(--color-accent)]/20 text-[var(--color-accent)] font-medium"
                                    : "text-[var(--color-text-muted)] hover:bg-white/5 hover:text-slate-200"
                            )}
                        >
                            <tab.icon size={12} />
                            {tab.label}
                        </button>
                    ))}
                </div>

                {/* Search */}
                <div className="relative">
                    <Search
                        size={14}
                        className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)]"
                    />
                    <input
                        type="text"
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        placeholder={t("settings.memory.search.placeholder")}
                        className={clsx(inputClasses, "pl-9 py-2")}
                    />
                </div>
            </div>

            {/* Content Area */}
            <div className="flex-1 overflow-y-auto min-h-0 relative scrollable pr-1">
                {loading && memories.length === 0 ? (
                    <div className="absolute inset-0 flex items-center justify-center">
                        <div className="text-[var(--color-text-muted)] text-sm animate-pulse">{t("settings.memory.loading")}</div>
                    </div>
                ) : filtered.length === 0 ? (
                    <div className="text-center py-12">
                        <Brain
                            size={32}
                            className="mx-auto mb-3 text-[var(--color-text-muted)] opacity-30"
                        />
                        <p className="text-sm text-[var(--color-text-muted)]">
                            {searchQuery
                                ? t("settings.memory.empty.search")
                                : t("settings.memory.empty.all")}
                        </p>
                    </div>
                ) : view === "timeline" ? (
                    <MemoryTimeline
                        memories={filtered}
                        onSelect={(mem) => {
                            setView("list");
                            setSearchQuery(mem.content.substring(0, 20)); // Quick hack to jump to it
                        }}
                    />
                ) : view === "graph" ? (
                    <MemoryGraph
                        memories={filtered}
                        onSelectKeyword={(kw) => {
                            setSearchQuery(kw);
                            setView("list");
                        }}
                    />
                ) : (
                    /* LIST VIEW */
                    <div className="space-y-2 pb-4">
                        <AnimatePresence mode="popLayout">
                            {filtered.map((mem) => (
                                <motion.div
                                    key={mem.id}
                                    layout
                                    initial={{ opacity: 0, y: 8 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    exit={{ opacity: 0, scale: 0.95 }}
                                    className={clsx(
                                        "group rounded-lg border p-3 transition-all",
                                        editingId === mem.id
                                            ? "border-[var(--color-accent)] bg-[var(--color-accent)]/5"
                                            : "border-[var(--color-border)] bg-black/20 hover:border-[var(--color-border-hover)]"
                                    )}
                                >
                                    {editingId === mem.id ? (
                                        /* ── Edit Mode ── */
                                        <div className="space-y-3">
                                            <textarea
                                                value={editContent}
                                                onChange={(e) =>
                                                    setEditContent(e.target.value)
                                                }
                                                rows={3}
                                                className={clsx(
                                                    inputClasses,
                                                    "resize-none text-xs"
                                                )}
                                                autoFocus
                                            />
                                            <div className="flex items-center gap-3">
                                                <label className="text-[10px] font-heading uppercase tracking-wider text-[var(--color-text-muted)]">
                                                    {t("settings.memory.edit.importance")}
                                                </label>
                                                <input
                                                    type="range"
                                                    min="0"
                                                    max="1"
                                                    step="0.1"
                                                    value={editImportance}
                                                    onChange={(e) =>
                                                        setEditImportance(
                                                            parseFloat(
                                                                e.target.value
                                                            )
                                                        )
                                                    }
                                                    className="flex-1 accent-[var(--color-accent)]"
                                                />
                                                <span
                                                    className={clsx(
                                                        "text-[10px] font-mono px-1.5 py-0.5 rounded border",
                                                        getImportanceColor(editImportance)
                                                    )}
                                                >
                                                    {editImportance.toFixed(1)}
                                                </span>
                                            </div>
                                            <div className="flex justify-end gap-2">
                                                <button
                                                    onClick={() =>
                                                        setEditingId(null)
                                                    }
                                                    className="p-1.5 rounded hover:bg-white/5 text-[var(--color-text-muted)]"
                                                >
                                                    <X size={14} />
                                                </button>
                                                <button
                                                    onClick={saveEdit}
                                                    className="p-1.5 rounded hover:bg-emerald-500/20 text-emerald-400"
                                                >
                                                    <Check size={14} />
                                                </button>
                                            </div>
                                        </div>
                                    ) : deletingId === mem.id ? (
                                        /* ── Delete Confirm ── */
                                        <div className="flex items-center justify-between">
                                            <span className="text-xs text-red-400">
                                                {t("settings.memory.delete.confirm")}
                                            </span>
                                            <div className="flex gap-2">
                                                <button
                                                    onClick={() =>
                                                        setDeletingId(null)
                                                    }
                                                    className="px-2 py-1 text-[10px] rounded border border-[var(--color-border)] text-[var(--color-text-muted)] hover:bg-white/5"
                                                >
                                                    {t("common.actions.cancel")}
                                                </button>
                                                <button
                                                    onClick={() =>
                                                        confirmDelete(mem.id)
                                                    }
                                                    className="px-2 py-1 text-[10px] rounded border border-red-500/40 text-red-400 hover:bg-red-500/20"
                                                >
                                                    {t("common.actions.delete")}
                                                </button>
                                            </div>
                                        </div>
                                    ) : (
                                        /* ── View Mode ── */
                                        <div className="flex gap-3">
                                            <div className="flex-1 min-w-0">
                                                <p className="text-sm text-[var(--color-text-primary)] leading-relaxed break-words">
                                                    {mem.content}
                                                </p>
                                                <div className="flex items-center gap-2 mt-2">
                                                    <span
                                                        className={clsx(
                                                            "text-[9px] font-mono px-1.5 py-0.5 rounded border",
                                                            getImportanceColor(mem.importance)
                                                        )}
                                                    >
                                                        {getImportanceLabel(mem.importance)}
                                                    </span>
                                                    <span className="text-[10px] text-[var(--color-text-muted)]">
                                                        {getTimeAgo(mem.created_at)}
                                                    </span>
                                                </div>
                                            </div>
                                            <div className="flex flex-col gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                                                <button
                                                    onClick={() => startEdit(mem)}
                                                    className="p-1 rounded hover:bg-white/10 text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                                    title={t("common.actions.edit")}
                                                >
                                                    <Pencil size={12} />
                                                </button>
                                                <button
                                                    onClick={() =>
                                                        setDeletingId(mem.id)
                                                    }
                                                    className="p-1 rounded hover:bg-red-500/20 text-[var(--color-text-muted)] hover:text-red-400"
                                                    title={t("common.actions.delete")}
                                                >
                                                    <Trash2 size={12} />
                                                </button>
                                            </div>
                                        </div>
                                    )}
                                </motion.div>
                            ))}
                        </AnimatePresence>

                        {/* Load More */}
                        {hasMore && !searchQuery && (
                            <button
                                onClick={() => setPage((p) => p + 1)}
                                className="w-full py-2 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors flex items-center justify-center gap-1"
                            >
                                <ChevronDown size={12} />
                                {t("settings.memory.load_more")}
                            </button>
                        )}
                    </div>
                )}
            </div>
        </div>
    );
}
