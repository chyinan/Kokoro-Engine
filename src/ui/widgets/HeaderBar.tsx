import { useState, useEffect } from "react";
import { motion } from "framer-motion";
import { clsx } from "clsx";
import { Settings, Cpu } from "lucide-react";
import { getCharacterState, getEngineInfo } from "../../lib/kokoro-bridge";
import type { CharacterState, EngineInfo } from "../../lib/kokoro-bridge";
import { useTranslation } from "react-i18next";

interface HeaderBarProps {
    onSettingsClick?: () => void;
}

export default function HeaderBar({ onSettingsClick }: HeaderBarProps) {
    const { t } = useTranslation();
    const [character, setCharacter] = useState<CharacterState | null>(null);
    const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
    const [online, setOnline] = useState(false);

    useEffect(() => {
        const fetchState = async () => {
            try {
                const [charState, info] = await Promise.all([
                    getCharacterState(),
                    getEngineInfo(),
                ]);
                setCharacter(charState);
                setEngineInfo(info);
                setOnline(true);
            } catch {
                setOnline(false);
            }
        };
        fetchState();
        const interval = setInterval(fetchState, 10000);
        return () => clearInterval(interval);
    }, []);

    // Mood-based accent color (§2.2 Character Mood Colors)
    const getMoodColor = (mood: number): string => {
        if (mood < 0.2) return "#8b9cf7";      // Sad — soft lavender
        if (mood < 0.4) return "#a78bfa";      // Melancholy — muted violet
        if (mood < 0.6) return "#00f0ff";      // Neutral — default cyan
        if (mood < 0.8) return "#34d399";      // Happy — warm teal
        return "#fbbf24";                       // Joyful — soft gold
    };

    const moodColor = character ? getMoodColor(character.mood) : "var(--color-accent)";

    return (
        <motion.div
            initial={{ opacity: 0, y: -10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3 }}
            className={clsx(
                "flex items-center justify-between px-5 h-full w-full",
                "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                "border-b border-[var(--color-border)]"
            )}
            style={{ pointerEvents: "auto" }}
        >
            {/* Left: App Title */}
            <div className="flex items-center gap-3">
                <Cpu size={18} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                <h1 className="font-heading text-lg font-bold tracking-widest uppercase text-[var(--color-accent)] drop-shadow-[var(--glow-accent)]">
                    {engineInfo?.name || "KOKORO ENGINE"}
                </h1>
                {engineInfo && (
                    <span className="text-[var(--color-text-muted)] text-xs font-mono">
                        v{engineInfo.version}
                    </span>
                )}
            </div>

            {/* Center: Character Info */}
            <div className="flex items-center gap-3">
                {character && (
                    <>
                        <span className="font-heading text-sm font-semibold tracking-wider uppercase text-[var(--color-text-secondary)]">
                            {character.name}
                        </span>
                        {/* Mood indicator dot */}
                        <motion.div
                            animate={{ backgroundColor: moodColor }}
                            transition={{ duration: 0.5, ease: "easeInOut" }}
                            className="w-2.5 h-2.5 rounded-full"
                            style={{ boxShadow: `0 0 8px ${moodColor}` }}
                            title={`Mood: ${Math.round(character.mood * 100)}%`}
                        />
                        <span className="text-[var(--color-text-muted)] text-xs">
                            {character.current_expression}
                        </span>
                    </>
                )}
            </div>

            {/* Right: Status & Settings */}
            <div className="flex items-center gap-4">
                {/* Online/Offline indicator */}
                <div className="flex items-center gap-2">
                    <div className={clsx(
                        "w-2 h-2 rounded-full",
                        online
                            ? "bg-emerald-500 shadow-[var(--glow-success)]"
                            : "bg-amber-500"
                    )} />
                    <span className="font-heading text-xs font-semibold tracking-wider uppercase text-[var(--color-text-muted)]">
                        {online ? t("header.status.online") : t("header.status.offline")}
                    </span>
                </div>

                {/* Settings button */}
                <motion.button
                    whileHover={{ scale: 1.1 }}
                    whileTap={{ scale: 0.95 }}
                    onClick={onSettingsClick}
                    className="p-2 rounded-md text-[var(--color-text-secondary)] hover:text-[var(--color-accent)] transition-colors"
                    aria-label={t("header.settings")}
                >
                    <Settings size={18} strokeWidth={1.5} />
                </motion.button>
            </div>
        </motion.div>
    );
}
