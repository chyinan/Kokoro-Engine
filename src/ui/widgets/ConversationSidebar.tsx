// pattern: Imperative Shell

import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Plus, Trash2, History, X, Check, Pencil, Pin } from "lucide-react";
import { 
    listConversations, 
    deleteConversation, 
    renameConversation, 
    updateConversationState,
    getConversationDisplayTitle, 
    hasPinnedConversationState 
} from "../../lib/kokoro-bridge";
import type { Conversation } from "../../lib/kokoro-bridge";
import { useTranslation } from "react-i18next";

type ConversationSidebarProps = {
    open: boolean;
    onClose: () => void;
    characterId: string;
    activeConversationId: string | null;
    onStartEmptyConversation: () => void;
    onSelectConversation: (conversationId: string | null) => Promise<void>;
};

export default function ConversationSidebar({
    open,
    onClose,
    characterId,
    activeConversationId,
    onStartEmptyConversation,
    onSelectConversation,
}: ConversationSidebarProps) {
    const { t } = useTranslation();
    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [editingId, setEditingId] = useState<string | null>(null);
    const [editTitle, setEditTitle] = useState("");
    const editInputRef = useRef<HTMLInputElement>(null);

    const refresh = useCallback(async () => {
        try {
            const list = await listConversations(characterId);
            setConversations(list);
        } catch (err) {
            console.error("[ConversationSidebar] Failed to list conversations:", err);
        }
    }, [characterId]);

    // 置顶优先排序：已固定的会话始终排在最前面，同级别按更新时间倒序
    const sortedConversations = useMemo(() => {
        return [...conversations].sort((a, b) => {
            const aPinned = hasPinnedConversationState(a.pinned_state) ? 1 : 0;
            const bPinned = hasPinnedConversationState(b.pinned_state) ? 1 : 0;
            if (aPinned !== bPinned) {
                return bPinned - aPinned;
            }
            return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
        });
    }, [conversations]);

    const handleTogglePin = async (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        const isCurrentlyPinned = hasPinnedConversationState(conv.pinned_state);
        const nextPinnedState = isCurrentlyPinned
            ? "{}"
            : JSON.stringify({ pinned: true, pinned_at: new Date().toISOString() });

        // 乐观更新
        setConversations(prev =>
            prev.map(c => (c.id === conv.id ? { ...c, pinned_state: nextPinnedState } : c))
        );

        try {
            await updateConversationState(conv.id, { pinned_state: nextPinnedState });
            await refresh();
        } catch (err) {
            console.error("[ConversationSidebar] Failed to toggle pin:", err);
            await refresh();
        }
    };

    useEffect(() => {
        setConversations([]);
        if (open) void refresh();
    }, [open, refresh]);

    useEffect(() => {
        if (editingId && editInputRef.current) {
            editInputRef.current.focus();
            editInputRef.current.select();
        }
    }, [editingId]);

    const handleCreate = async () => {
        onClose();
        onStartEmptyConversation();
    };

    const handleDelete = async (e: React.MouseEvent, id: string) => {
        e.stopPropagation();
        if (!confirm(t("chat.history.confirmDelete"))) return;
        try {
            await deleteConversation(id);
            if (activeConversationId === id) {
                await onSelectConversation(null);
            }
            await refresh();
        } catch (err) {
            console.error("[ConversationSidebar] Failed to delete conversation:", err);
        }
    };

    const handleRenameStart = (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        setEditingId(conv.id);
        setEditTitle(conv.title);
    };

    const handleRenameConfirm = async (id: string) => {
        if (!editingId) return;
        const trimmed = editTitle.trim();
        setEditingId(null);
        if (!trimmed) return;
        try {
            await renameConversation(id, trimmed);
            await refresh();
        } catch (err) {
            console.error("[ConversationSidebar] Failed to rename conversation:", err);
        }
    };

    const handleRenameKeyDown = (e: React.KeyboardEvent, id: string) => {
        if (e.key === "Enter") {
            void handleRenameConfirm(id);
        } else if (e.key === "Escape") {
            setEditingId(null);
        }
    };

    const handleLoad = async (id: string) => {
        if (editingId) return;
        if (id === activeConversationId) {
            onClose();
            return;
        }
        await onSelectConversation(id);
        onClose();
    };

    const formatTime = (iso: string) => {
        try {
            const d = new Date(iso);
            const now = new Date();
            const isToday = d.toDateString() === now.toDateString();
            if (isToday) {
                return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
            }
            return d.toLocaleDateString([], { month: "numeric", day: "numeric" });
        } catch {
            return "";
        }
    };

    return (
        <AnimatePresence>
            {open && (
                <motion.div
                    initial={{ x: "100%" }}
                    animate={{ x: 0 }}
                    exit={{ x: "100%" }}
                    transition={{ type: "spring", damping: 28, stiffness: 300 }}
                    className="absolute inset-y-0 right-0 w-72 bg-[var(--color-bg-secondary)] border-l border-[var(--color-border)] shadow-2xl z-30 flex flex-col backdrop-blur-md"
                >
                    {/* Header */}
                    <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border)]">
                        <div className="flex items-center gap-2 text-sm font-medium text-[var(--color-text-primary)]">
                            <History size={16} strokeWidth={1.5} />
                            <span>{t("chat.history.title")}</span>
                        </div>
                        <button
                            onClick={onClose}
                            className="p-1 rounded text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-white/5 transition-colors"
                        >
                            <X size={16} strokeWidth={1.5} />
                        </button>
                    </div>

                    {/* New chat button */}
                    <div className="p-2">
                        <button
                            onClick={handleCreate}
                            className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg bg-[var(--color-accent)]/10 hover:bg-[var(--color-accent)]/20 text-[var(--color-accent)] text-xs font-medium transition-colors border border-[var(--color-accent)]/20"
                        >
                            <Plus size={14} strokeWidth={1.5} />
                            {t("chat.history.newChat")}
                        </button>
                    </div>

                    {/* Conversation list */}
                    <div className="flex-1 overflow-y-auto px-2 pb-2 space-y-1 scrollable">
                        {sortedConversations.length === 0 ? (
                            <div className="text-center text-xs text-[var(--color-text-muted)] py-8">
                                {t("chat.history.empty")}
                            </div>
                        ) : (
                            sortedConversations.map(conv => (
                                <div
                                    key={conv.id}
                                    onClick={() => handleLoad(conv.id)}
                                    className={clsx(
                                        "group flex items-center gap-2 px-3 py-2.5 rounded-lg cursor-pointer transition-colors",
                                        activeConversationId === conv.id
                                            ? "bg-[var(--color-accent)]/10 border border-[var(--color-accent)]/30"
                                            : "hover:bg-white/5 border border-transparent"
                                    )}
                                >
                                    <div className="flex-1 min-w-0">
                                        {editingId === conv.id ? (
                                            <div className="flex items-center gap-1">
                                                <input
                                                    ref={editInputRef}
                                                    value={editTitle}
                                                    onChange={e => setEditTitle(e.target.value)}
                                                    onKeyDown={e => handleRenameKeyDown(e, conv.id)}
                                                    onBlur={() => handleRenameConfirm(conv.id)}
                                                    className="flex-1 bg-black/40 border border-[var(--color-border)] text-xs text-[var(--color-text-primary)] rounded px-1.5 py-0.5 focus:outline-none focus:border-[var(--color-accent)]"
                                                    onClick={e => e.stopPropagation()}
                                                />
                                                <button
                                                    onClick={(e) => {
                                                        e.stopPropagation();
                                                        void handleRenameConfirm(conv.id);
                                                    }}
                                                    className="p-1 text-[var(--color-accent)] hover:opacity-80"
                                                >
                                                    <Check size={12} strokeWidth={2} />
                                                </button>
                                            </div>
                                        ) : (
                                            <>
                                                <div className="text-xs text-[var(--color-text-primary)] truncate">
                                                    {getConversationDisplayTitle(conv)}
                                                </div>
                                                <div className="mt-0.5 flex items-center gap-1 text-[10px] text-[var(--color-text-muted)]">
                                                    <span>{formatTime(conv.updated_at)}</span>
                                                    {conv.topic.trim() && (
                                                        <span className="truncate max-w-[110px]">· {conv.topic}</span>
                                                    )}
                                                    {hasPinnedConversationState(conv.pinned_state) && (
                                                        <span className="inline-flex items-center gap-0.5 text-[var(--color-accent)] font-medium">
                                                            <Pin size={9} strokeWidth={1.5} className="fill-current" />
                                                            {t("chat.history.pinned")}
                                                        </span>
                                                    )}
                                                </div>
                                            </>
                                        )}
                                    </div>
                                    {editingId !== conv.id && (
                                        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                                            <button
                                                onClick={(e) => handleTogglePin(e, conv)}
                                                className={clsx(
                                                    "p-1 rounded transition-colors",
                                                    hasPinnedConversationState(conv.pinned_state)
                                                        ? "text-[var(--color-accent)] hover:text-[var(--color-text-muted)]"
                                                        : "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                                )}
                                                title={hasPinnedConversationState(conv.pinned_state) ? t("chat.history.unpin") : t("chat.history.pin")}
                                            >
                                                <Pin size={12} strokeWidth={1.5} className={hasPinnedConversationState(conv.pinned_state) ? "fill-current" : ""} />
                                            </button>
                                            <button
                                                onClick={(e) => handleRenameStart(e, conv)}
                                                className="p-1 rounded text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                                                title={t("chat.history.rename")}
                                            >
                                                <Pencil size={12} strokeWidth={1.5} />
                                            </button>
                                            <button
                                                onClick={(e) => handleDelete(e, conv.id)}
                                                className="p-1 rounded text-[var(--color-text-muted)] hover:text-[var(--color-error)] transition-colors"
                                                title={t("chat.history.delete")}
                                            >
                                                <Trash2 size={12} strokeWidth={1.5} />
                                            </button>
                                        </div>
                                    )}
                                </div>
                            ))
                        )}
                    </div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
