import { useState, useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { clsx } from "clsx";
import { Bot, Shield, Volume2, Loader2, Play, Square, RefreshCw } from "lucide-react";
import {
    getTelegramConfig, saveTelegramConfig,
    startTelegramBot, stopTelegramBot, getTelegramStatus,
} from "../../../lib/kokoro-bridge";
import type { TelegramConfig, TelegramStatus } from "../../../lib/kokoro-bridge";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";

export default function TelegramTab() {
    const [config, setConfig] = useState<TelegramConfig | null>(null);
    const [status, setStatus] = useState<TelegramStatus | null>(null);
    const [loading, setLoading] = useState(true);
    const [dirty, setDirty] = useState(false);
    const [chatIdInput, setChatIdInput] = useState("");
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        loadAll();
        // Poll status every 5s
        pollRef.current = setInterval(() => {
            getTelegramStatus().then(setStatus).catch(() => {});
        }, 5000);
        return () => {
            if (pollRef.current) clearInterval(pollRef.current);
        };
    }, []);

    const loadAll = async () => {
        try {
            const [cfg, st] = await Promise.all([getTelegramConfig(), getTelegramStatus()]);
            setConfig(cfg);
            setStatus(st);
        } catch (e) {
            console.error("[TelegramTab] Failed to load:", e);
        } finally {
            setLoading(false);
        }
    };

    const update = (patch: Partial<TelegramConfig>) => {
        if (!config) return;
        setConfig({ ...config, ...patch });
        setDirty(true);
    };

    const handleSave = async () => {
        if (!config) return;
        try {
            await saveTelegramConfig(config);
            setDirty(false);
        } catch (e) {
            console.error("[TelegramTab] Failed to save:", e);
        }
    };

    const handleStart = async () => {
        try {
            // Save first if dirty
            if (dirty && config) {
                await saveTelegramConfig(config);
                setDirty(false);
            }
            await startTelegramBot();
            const st = await getTelegramStatus();
            setStatus(st);
        } catch (e) {
            console.error("[TelegramTab] Start failed:", e);
        }
    };

    const handleStop = async () => {
        try {
            await stopTelegramBot();
            const st = await getTelegramStatus();
            setStatus(st);
        } catch (e) {
            console.error("[TelegramTab] Stop failed:", e);
        }
    };

    const addChatId = () => {
        const id = parseInt(chatIdInput.trim(), 10);
        if (isNaN(id) || !config) return;
        if (config.allowed_chat_ids.includes(id)) return;
        update({ allowed_chat_ids: [...config.allowed_chat_ids, id] });
        setChatIdInput("");
    };

    const removeChatId = (id: number) => {
        if (!config) return;
        update({ allowed_chat_ids: config.allowed_chat_ids.filter(c => c !== id) });
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center py-12">
                <Loader2 size={20} className="animate-spin text-[var(--color-text-muted)]" />
            </div>
        );
    }

    if (!config) return null;

    const isRunning = status?.running ?? false;

    return (
        <div className="space-y-6">
            {/* Status & Controls */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <Bot size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div>
                        <div className="text-sm font-heading font-semibold">Telegram Bot</div>
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {isRunning ? "Running" : "Stopped"}
                        </div>
                    </div>
                    <div className={clsx(
                        "w-2 h-2 rounded-full",
                        isRunning ? "bg-green-400" : "bg-[var(--color-text-muted)]"
                    )} />
                </div>
                <div className="flex items-center gap-2">
                    {isRunning ? (
                        <motion.button
                            whileTap={{ scale: 0.95 }}
                            onClick={handleStop}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-heading
                                bg-red-500/20 text-red-400 border border-red-500/30 hover:bg-red-500/30 transition-colors"
                        >
                            <Square size={12} /> Stop
                        </motion.button>
                    ) : (
                        <motion.button
                            whileTap={{ scale: 0.95 }}
                            onClick={handleStart}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-heading
                                bg-[var(--color-accent)]/20 text-[var(--color-accent)] border border-[var(--color-accent)]/30
                                hover:bg-[var(--color-accent)]/30 transition-colors"
                        >
                            <Play size={12} /> Start
                        </motion.button>
                    )}
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={loadAll}
                        className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
                    >
                        <RefreshCw size={14} />
                    </motion.button>
                </div>
            </div>

            {/* Enable on startup */}
            <div className="flex items-center justify-between">
                <div className="text-sm text-[var(--color-text-secondary)]">Auto-start on launch</div>
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

            {/* Bot Token */}
            <div>
                <label className={labelClasses}>Bot Token</label>
                <input
                    type="password"
                    value={config.bot_token ?? ""}
                    onChange={e => update({ bot_token: e.target.value || undefined })}
                    placeholder="123456:ABC-DEF..."
                    className={inputClasses}
                />
                <div className="text-xs text-[var(--color-text-muted)] mt-1">
                    Get from @BotFather on Telegram. Or set env var: {config.bot_token_env ?? "TELEGRAM_BOT_TOKEN"}
                </div>
            </div>

            {/* Allowed Chat IDs */}
            <div>
                <label className={labelClasses}>
                    <Shield size={12} className="inline mr-1" />
                    Allowed Chat IDs
                </label>
                <div className="text-xs text-[var(--color-text-muted)] mb-2">
                    Only these chat IDs can interact with the bot. Empty = reject all.
                </div>
                <div className="flex gap-2 mb-2">
                    <input
                        type="text"
                        value={chatIdInput}
                        onChange={e => setChatIdInput(e.target.value)}
                        onKeyDown={e => e.key === "Enter" && addChatId()}
                        placeholder="Chat ID (number)"
                        className={clsx(inputClasses, "flex-1")}
                    />
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={addChatId}
                        className="px-3 py-2 rounded-md text-xs font-heading
                            bg-[var(--color-bg-surface)] border border-[var(--color-border)]
                            hover:border-[var(--color-accent)] transition-colors"
                    >
                        Add
                    </motion.button>
                </div>
                {config.allowed_chat_ids.length > 0 && (
                    <div className="flex flex-wrap gap-2">
                        {config.allowed_chat_ids.map(id => (
                            <span
                                key={id}
                                className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-xs
                                    bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                            >
                                {id}
                                <button
                                    onClick={() => removeChatId(id)}
                                    className="text-[var(--color-text-muted)] hover:text-red-400 transition-colors ml-1"
                                >
                                    ×
                                </button>
                            </span>
                        ))}
                    </div>
                )}
            </div>

            {/* Voice Reply */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                    <Volume2 size={14} className="text-[var(--color-text-muted)]" />
                    <div className="text-sm text-[var(--color-text-secondary)]">Send voice replies (TTS)</div>
                </div>
                <motion.button
                    whileTap={{ scale: 0.95 }}
                    onClick={() => update({ send_voice_reply: !config.send_voice_reply })}
                    className={clsx(
                        "w-12 h-6 rounded-full relative transition-colors duration-200",
                        config.send_voice_reply
                            ? "bg-[var(--color-accent)]"
                            : "bg-[var(--color-bg-surface)] border border-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: config.send_voice_reply ? 24 : 2 }}
                        transition={{ type: "spring", stiffness: 500, damping: 30 }}
                        className={clsx(
                            "w-5 h-5 rounded-full absolute top-0.5",
                            config.send_voice_reply ? "bg-black" : "bg-[var(--color-text-muted)]"
                        )}
                    />
                </motion.button>
            </div>

            {/* Save Button */}
            {dirty && (
                <motion.button
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    whileTap={{ scale: 0.97 }}
                    onClick={handleSave}
                    className="w-full py-2.5 rounded-md text-sm font-heading font-semibold
                        bg-[var(--color-accent)] text-black hover:brightness-110 transition-all"
                >
                    Save
                </motion.button>
            )}
        </div>
    );
}
