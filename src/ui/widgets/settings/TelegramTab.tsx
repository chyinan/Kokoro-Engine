import { useState, useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { clsx } from "clsx";
import { Bot, Shield, Volume2, Loader2, Play, Square, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
    saveTelegramConfig,
    startTelegramBot, stopTelegramBot, getTelegramStatus, listCharacters,
} from "../../../lib/kokoro-bridge";
import type { TelegramConfig, TelegramStatus, CharacterRecord } from "../../../lib/kokoro-bridge";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";
import { Select } from "@/components/ui/select";

interface TelegramTabProps {
    config: TelegramConfig | null;
    onUpdate: (patch: Partial<TelegramConfig>) => void;
}

export default function TelegramTab({ config, onUpdate }: TelegramTabProps) {
    const { t } = useTranslation();
    const [status, setStatus] = useState<TelegramStatus | null>(null);
    const [loading, setLoading] = useState(true);
    const [dirty, setDirty] = useState(false);
    const [chatIdInput, setChatIdInput] = useState("");
    const [characters, setCharacters] = useState<CharacterRecord[]>([]);
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        loadStatus();
        pollRef.current = setInterval(() => {
            getTelegramStatus().then(setStatus).catch(() => {});
        }, 5000);
        return () => {
            if (pollRef.current) clearInterval(pollRef.current);
        };
    }, []);

    const loadStatus = async () => {
        try {
            const [st, chars] = await Promise.all([
                getTelegramStatus(),
                listCharacters(),
            ]);
            setStatus(st);
            setCharacters(chars);
        } catch (e) {
            console.error("[TelegramTab] Failed to load status:", e);
        } finally {
            setLoading(false);
        }
    };

    const update = (patch: Partial<TelegramConfig>) => {
        onUpdate(patch);
        setDirty(true);
    };

    const handleStart = async () => {
        try {
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
        if (!config) return;
        const id = parseInt(chatIdInput.trim(), 10);
        if (isNaN(id)) return;
        if (config.allowed_chat_ids.includes(id)) return;
        update({ allowed_chat_ids: [...config.allowed_chat_ids, id] });
        setChatIdInput("");
    };

    const removeChatId = (id: number) => {
        if (!config) return;
        update({ allowed_chat_ids: config.allowed_chat_ids.filter(c => c !== id) });
    };

    if (loading || !config) {
        return (
            <div className="flex items-center justify-center py-12">
                <Loader2 size={20} className="animate-spin text-[var(--color-text-muted)]" />
            </div>
        );
    }

    const isRunning = status?.running ?? false;

    return (
        <div className="space-y-6">
            {/* Status & Controls */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <Bot size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div>
                        <div className="text-sm font-heading font-semibold">{t("telegram.title")}</div>
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {isRunning ? t("telegram.status.running") : t("telegram.status.stopped")}
                        </div>
                    </div>
                    <div className={clsx(
                        "w-2 h-2 rounded-full",
                        isRunning ? "bg-[var(--color-accent)]" : "bg-[var(--color-text-muted)]"
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
                            <Square size={12} /> {t("telegram.stop")}
                        </motion.button>
                    ) : (
                        <motion.button
                            whileTap={{ scale: 0.95 }}
                            onClick={handleStart}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-heading
                                bg-[var(--color-accent)]/20 text-[var(--color-accent)] border border-[var(--color-accent)]/30
                                hover:bg-[var(--color-accent)]/30 transition-colors"
                        >
                            <Play size={12} /> {t("telegram.start")}
                        </motion.button>
                    )}
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={loadStatus}
                        className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
                    >
                        <RefreshCw size={14} />
                    </motion.button>
                </div>
            </div>

            {/* Enable on startup */}
            <div className="flex items-center justify-between">
                <div className="text-sm text-[var(--color-text-secondary)]">{t("telegram.auto_start")}</div>
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
                <label className={labelClasses}>{t("telegram.bot_token.label")}</label>
                <input
                    type="password"
                    value={config.bot_token ?? ""}
                    onChange={e => update({ bot_token: e.target.value || undefined })}
                    placeholder={t("telegram.bot_token.placeholder")}
                    className={inputClasses}
                />
                <div className="text-xs text-[var(--color-text-muted)] mt-1">
                    {t("telegram.bot_token.hint", { env: config.bot_token_env ?? "TELEGRAM_BOT_TOKEN" })}
                </div>
            </div>

            {/* Allowed Chat IDs */}
            <div>
                <label className={labelClasses}>
                    <Shield size={12} className="inline mr-1" />
                    {t("telegram.whitelist.label")}
                </label>
                <div className="text-xs text-[var(--color-text-muted)] mb-2">
                    {t("telegram.whitelist.desc")}
                </div>
                <div className="flex gap-2 mb-2">
                    <input
                        type="text"
                        value={chatIdInput}
                        onChange={e => setChatIdInput(e.target.value)}
                        onKeyDown={e => e.key === "Enter" && addChatId()}
                        placeholder={t("telegram.whitelist.placeholder")}
                        className={clsx(inputClasses, "flex-1")}
                    />
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={addChatId}
                        className="px-3 py-2 rounded-md text-xs font-heading
                            bg-[var(--color-bg-surface)] border border-[var(--color-border)]
                            hover:border-[var(--color-accent)] transition-colors"
                    >
                        {t("telegram.whitelist.add")}
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

            {/* Character ID */}
            <div>
                <label className={labelClasses}>{t("telegram.character_id.label")}</label>
                <Select
                    value={config.character_id ?? ""}
                    onChange={v => update({ character_id: v || undefined })}
                    options={[
                        { value: "", label: t("telegram.character_id.auto") },
                        ...characters.map(char => ({
                            value: char.id,
                            label: char.name,
                        })),
                    ]}
                />
                <div className="text-xs text-[var(--color-text-muted)] mt-1">
                    {t("telegram.character_id.hint")}
                </div>
            </div>

            {/* Voice Reply */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                    <Volume2 size={14} className="text-[var(--color-text-muted)]" />
                    <div className="text-sm text-[var(--color-text-secondary)]">{t("telegram.voice_reply")}</div>
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
        </div>
    );
}
