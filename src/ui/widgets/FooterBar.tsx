import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import {
    Smile, Frown, Zap, Meh,
    Brain, Heart, Sparkles, AlertTriangle, PartyPopper, Angry
} from "lucide-react";
import { setExpression, getEngineInfo, onChatExpression } from "../../lib/kokoro-bridge";
import type { EmotionState } from "../../features/live2d/Live2DController";
import { useTranslation } from "react-i18next";

// ── Emotion Definitions ────────────────────────────

interface EmotionDef {
    id: EmotionState;
    icon: React.ComponentType<{ size?: number; strokeWidth?: number; className?: string }>;
    // label removed, will use translation
    color: string; // tailwind-ish color for active state
}

const emotions: EmotionDef[] = [
    { id: "neutral", icon: Meh, color: "text-gray-400" },
    { id: "happy", icon: Smile, color: "text-emerald-400" },
    { id: "excited", icon: PartyPopper, color: "text-yellow-400" },
    { id: "smug", icon: Sparkles, color: "text-amber-400" },
    { id: "shy", icon: Heart, color: "text-pink-400" },
    { id: "thinking", icon: Brain, color: "text-blue-400" },
    { id: "surprised", icon: Zap, color: "text-purple-400" },
    { id: "worried", icon: AlertTriangle, color: "text-orange-400" },
    { id: "sad", icon: Frown, color: "text-sky-400" },
    { id: "angry", icon: Angry, color: "text-red-400" },
];

// ── Mood helpers ───────────────────────────────────

/** Returns a gradient stop color based on mood value (0.0=bad → 1.0=great) */
function moodToColor(mood: number): string {
    if (mood >= 0.8) return "#34d399"; // emerald
    if (mood >= 0.6) return "#a3e635"; // lime
    if (mood >= 0.4) return "#facc15"; // yellow
    if (mood >= 0.2) return "#fb923c"; // orange
    return "#f87171";                   // red
}

/** Height of the invisible hover trigger zone at the bottom of the screen */
const TRIGGER_ZONE_HEIGHT = 12;
/** Delay (ms) before hiding the footer after mouse leaves */
const HIDE_DELAY = 400;

// ── Component ──────────────────────────────────────

export default function FooterBar() {
    const { t } = useTranslation();
    const [activeEmotion, setActiveEmotion] = useState<EmotionState>("neutral");
    const [mood, setMood] = useState(0.5);
    const [engineVersion, setEngineVersion] = useState("");
    const [visible, setVisible] = useState(false);
    const [isAutoMode, setIsAutoMode] = useState(true); // LLM auto-controls expression
    const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Clear any pending hide timer
    const cancelHide = useCallback(() => {
        if (hideTimerRef.current) {
            clearTimeout(hideTimerRef.current);
            hideTimerRef.current = null;
        }
    }, []);

    // Schedule hiding with a delay
    const scheduleHide = useCallback(() => {
        cancelHide();
        hideTimerRef.current = setTimeout(() => {
            setVisible(false);
        }, HIDE_DELAY);
    }, [cancelHide]);

    // Mouse enters trigger zone or footer content → show
    const handleMouseEnter = useCallback(() => {
        cancelHide();
        setVisible(true);
    }, [cancelHide]);

    // Mouse leaves trigger zone or footer content → schedule hide
    const handleMouseLeave = useCallback(() => {
        scheduleHide();
    }, [scheduleHide]);

    // Cleanup timer on unmount
    useEffect(() => {
        return () => { cancelHide(); };
    }, [cancelHide]);

    useEffect(() => {
        getEngineInfo()
            .then(info => setEngineVersion(info.version))
            .catch(() => setEngineVersion("—"));
    }, []);

    // Subscribe to LLM expression events
    useEffect(() => {
        let unlisten: (() => void) | undefined;

        onChatExpression((data) => {
            if (isAutoMode) {
                setActiveEmotion(data.expression as EmotionState);
                setMood(data.mood);
            }
        }).then(fn => { unlisten = fn; });

        return () => { unlisten?.(); };
    }, [isAutoMode]);

    const handleExpressionClick = useCallback(async (emotion: EmotionState) => {
        try {
            await setExpression(emotion);
            setActiveEmotion(emotion);
            // When user manually clicks, disable auto mode temporarily
            setIsAutoMode(false);
        } catch (e) {
            console.error("[FooterBar] Failed to set expression:", e);
        }
    }, []);

    const handleAutoToggle = useCallback(() => {
        setIsAutoMode(prev => !prev);
    }, []);

    const activeEmotionDef = emotions.find(e => e.id === activeEmotion) ?? emotions[0];

    return (
        <div className="relative w-full h-full" style={{ pointerEvents: "none" }}>
            {/* Invisible hover trigger zone — always present at bottom */}
            <div
                onMouseEnter={handleMouseEnter}
                onMouseLeave={handleMouseLeave}
                style={{
                    position: "absolute",
                    left: 0,
                    right: 0,
                    bottom: 0,
                    height: `${TRIGGER_ZONE_HEIGHT}px`,
                    pointerEvents: "auto",
                    zIndex: 5,
                }}
            />

            {/* Footer content */}
            <AnimatePresence>
                {visible && (
                    <motion.div
                        initial={{ y: 60 }}
                        animate={{ y: 0 }}
                        exit={{ y: 60 }}
                        transition={{ type: "spring", stiffness: 400, damping: 30 }}
                        onMouseEnter={handleMouseEnter}
                        onMouseLeave={handleMouseLeave}
                        className={clsx(
                            "relative flex items-center justify-between px-4 h-full w-full",
                            "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                            "border-t border-[var(--color-border)]"
                        )}
                        style={{ pointerEvents: "auto" }}
                    >
                        {/* Left: Mood indicator + Active emotion */}
                        <div className="flex items-center gap-3 shrink-0 z-10">
                            {/* Mood dot */}
                            <div className="flex flex-col items-center gap-1">
                                <div
                                    className="w-3 h-3 rounded-full shadow-lg transition-colors duration-500"
                                    style={{ backgroundColor: moodToColor(mood) }}
                                    title={`Mood: ${Math.round(mood * 100)}%`}
                                />
                                <span className="text-[8px] font-mono text-[var(--color-text-muted)] leading-none">
                                    {Math.round(mood * 100)}
                                </span>
                            </div>

                            {/* Active emotion display */}
                            <div className="flex items-center gap-1.5">
                                <activeEmotionDef.icon
                                    size={18}
                                    strokeWidth={1.5}
                                    className={clsx(activeEmotionDef.color, "transition-colors duration-300")}
                                />
                                <span className={clsx(
                                    "font-heading text-xs font-semibold tracking-wider uppercase",
                                    activeEmotionDef.color, "transition-colors duration-300"
                                )}>
                                    {t(`footer.emotions.${activeEmotionDef.id}`)}
                                </span>
                            </div>

                            {/* Auto/Manual toggle */}
                            <button
                                onClick={handleAutoToggle}
                                className={clsx(
                                    "px-2 py-0.5 rounded text-[9px] font-mono uppercase tracking-wider transition-colors border",
                                    isAutoMode
                                        ? "border-[var(--color-accent)]/30 text-[var(--color-accent)] bg-[var(--color-accent-subtle)]"
                                        : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
                                )}
                                title={isAutoMode ? t("footer.mode.auto_desc") : t("footer.mode.manual_desc")}
                            >
                                {isAutoMode ? t("footer.mode.auto") : t("footer.mode.manual")}
                            </button>
                        </div>

                        {/* Center: Expression grid — absolutely centered */}
                        <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-1">
                            {emotions.map(({ id, icon: Icon, color }) => (
                                <motion.button
                                    key={id}
                                    whileHover={{ scale: 1.15 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleExpressionClick(id)}
                                    className={clsx(
                                        "p-1.5 rounded-md transition-all duration-200",
                                        activeEmotion === id
                                            ? `${color} bg-white/10 shadow-sm`
                                            : "text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:bg-white/5"
                                    )}
                                    aria-label={`${t("footer.set_expression")}${t(`footer.emotions.${id}`)}`}
                                    title={t(`footer.emotions.${id}`)}
                                >
                                    <Icon size={14} strokeWidth={1.5} />
                                </motion.button>
                            ))}
                        </div>

                        {/* Right: Engine info */}
                        <div className="flex items-center gap-2 shrink-0 z-10">
                            <span className="font-mono text-[10px] text-[var(--color-text-muted)]">
                                {engineVersion && `v${engineVersion}`}
                            </span>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
