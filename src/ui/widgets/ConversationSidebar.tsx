import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Plus, Trash2, History, X, Check, Pencil } from "lucide-react";
import { listConversations, loadConversation, deleteConversation, createConversation, renameConversation } from "../../lib/kokoro-bridge";
import type { Conversation, ConversationMessage } from "../../lib/kokoro-bridge";
import { useTranslation } from "react-i18next";

interface ChatMessage {
    role: "user" | "kokoro";
    text: string;
    images?: string[];
    translation?: string;
}

interface ConversationSidebarProps {
    open: boolean;
    onClose: () => void;
    onLoadMessages: (messages: ChatMessage[]) => void;
}

export default function ConversationSidebar({ open, onClose, onLoadMessages }: ConversationSidebarProps) {
    const { t } = useTranslation();
    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [activeId, setActiveId] = useState<string | null>(null);
    const [editingId, setEditingId] = useState<string | null>(null);
    const [editTitle, setEditTitle] = useState("");
    const editInputRef = useRef<HTMLInputElement>(null);

    const characterId = localStorage.getItem("kokoro_active_character_id") || "default";

    const refresh = useCallback(async () => {
        try {
            const list = await listConversations(characterId);
            setConversations(list);
        } catch (err) {
            console.error("[ConversationSidebar] Failed to list conversations:", err);
        }
    }, [characterId]);

    useEffect(() => {
        if (open) refresh();
    }, [open, refresh]);

    useEffect(() => {
        if (editingId && editInputRef.current) {
            editInputRef.current.focus();
            editInputRef.current.select();
        }
    }, [editingId]);

    const handleLoad = async (id: string) => {
        if (id === activeId) return;
        try {
            const msgs: ConversationMessage[] = await loadConversation(id);
            const chatMsgs: ChatMessage[] = msgs.map(m => ({
                role: m.role === "user" ? "user" : "kokoro",
                text: m.content,
            }));
            setActiveId(id);
            onLoadMessages(chatMsgs);
        } catch (err) {
            console.error("[ConversationSidebar] Failed to load conversation:", err);
        }
    };

    const handleDelete = async (e: React.MouseEvent, id: string) => {
        e.stopPropagation();
        if (!confirm(t("chat.history.confirmDelete"))) return;
        try {
            await deleteConversation(id);
            if (activeId === id) {
                setActiveId(null);
                onLoadMessages([]);
            }
            refresh();
        } catch (err) {
            console.error("[ConversationSidebar] Failed to delete:", err);
        }
    };

    const handleNew = async () => {
        try {
            await createConversation();
            setActiveId(null);
            onLoadMessages([]);
            refresh();
        } catch (err) {
            console.error("[ConversationSidebar] Failed to create:", err);
        }
    };

    const handleRenameStart = (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        setEditingId(conv.id);
        setEditTitle(conv.title);
    };

    const handleRenameConfirm = async (id: string) => {
        const trimmed = editTitle.trim();
        if (trimmed) {
            try {
                await renameConversation(id, trimmed);
                refresh();
            } catch (err) {
                console.error("[ConversationSidebar] Failed to rename:", err);
            }
        }
        setEditingId(null);
    };

    const handleRenameKeyDown = (e: React.KeyboardEvent, id: string) => {
        if (e.key === "Enter") handleRenameConfirm(id);
        if (e.key === "Escape") setEditingId(null);
    };

    const formatTime = (iso: string) => {
        try {
            const d = new Date(iso);
            const now = new Date();
            const diff = now.getTime() - d.getTime();
            const mins = Math.floor(diff / 60000);
            if (mins < 1) return t("settings.memory.time.just_now");
            if (mins < 60) return t("settings.memory.time.minutes_ago", { count: mins });
            const hours = Math.floor(mins / 60);
            if (hours < 24) return t("settings.memory.time.hours_ago", { count: hours });
            const days = Math.floor(hours / 24);
            return t("settings.memory.time.days_ago", { count: days });
        } catch {
            return iso;
        }
    };

    return (
        <AnimatePresence>
            {open && (
                <motion.div
                    initial={{ x: -280, opacity: 0 }}
                    animate={{ x: 0, opacity: 1 }}
                    exit={{ x: -280, opacity: 0 }}
                    transition={{ type: "spring", stiffness: 300, damping: 30 }}
                    className={clsx(
                        "absolute inset-y-0 left-0 z-[100] w-[260px]",
                        "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                        "border-r border-[var(--color-border)]",
                        "flex flex-col shadow-xl"
                    )}
                >
                    {/* Header */}
                    <div className="flex items-center justify-between px-3 py-3 border-b border-[var(--color-border)]">
                        <div className="flex items-center gap-2">
                            <History size={14} strokeWidth={1.5} className="text-[var(--color-text-muted)]" />
                            <span className="text-xs font-semibold tracking-wider uppercase text-[var(--color-text-secondary)]">
                                {t("chat.history.title")}
                            </span>
                        </div>
                        <button
                            onClick={onClose}
                            className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                        >
                            <X size={14} strokeWidth={1.5} />
                        </button>
                    </div>

                    {/* New chat button */}
                    <div className="px-3 py-2">
                        <button
                            onClick={handleNew}
                            className={clsx(
                                "w-full flex items-center gap-2 px-3 py-2 rounded-lg text-xs",
                                "border border-dashed border-[var(--color-border)]",
                                "text-[var(--color-text-muted)] hover:text-[var(--color-accent)] hover:border-[var(--color-accent)]/50",
                                "transition-colors"
                            )}
                        >
                            <Plus size={14} strokeWidth={1.5} />
                            {t("chat.history.newChat")}
                        </button>
                    </div>

                    {/* Conversation list */}
                    <div className="flex-1 overflow-y-auto px-2 pb-2 space-y-1 scrollable">
                        {conversations.length === 0 ? (
                            <div className="text-center text-xs text-[var(--color-text-muted)] py-8">
                                {t("chat.history.empty")}
                            </div>
                        ) : (
                            conversations.map(conv => (
                                <div
                                    key={conv.id}
                                    onClick={() => handleLoad(conv.id)}
                                    className={clsx(
                                        "group flex items-center gap-2 px-3 py-2.5 rounded-lg cursor-pointer transition-colors",
                                        activeId === conv.id
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
                                                    onClick={(e) => { e.stopPropagation(); handleRenameConfirm(conv.id); }}
                                                    className="p-0.5 text-emerald-400 hover:text-emerald-300"
                                                >
                                                    <Check size={12} strokeWidth={2} />
                                                </button>
                                            </div>
                                        ) : (
                                            <>
                                                <div className="text-xs text-[var(--color-text-primary)] truncate">
                                                    {conv.title}
                                                </div>
                                                <div className="text-[10px] text-[var(--color-text-muted)] mt-0.5">
                                                    {formatTime(conv.updated_at)}
                                                </div>
                                            </>
                                        )}
                                    </div>
                                    {editingId !== conv.id && (
                                        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
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
