import { useRef, useCallback, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

// ── Types ──────────────────────────────────────────

type TypingSpeed = "Instant" | "Fast" | "Normal" | "Slow" | "Thinking";

interface TypingEvent {
    duration_ms: number;
    speed: TypingSpeed;
}

/** Per-character delay (ms) for each speed tier */
const SPEED_DELAYS: Record<TypingSpeed, { base: number; variance: number }> = {
    Instant: { base: 8, variance: 4 },
    Fast: { base: 18, variance: 8 },
    Normal: { base: 30, variance: 15 },
    Slow: { base: 55, variance: 25 },
    Thinking: { base: 70, variance: 35 },
};

/** Characters that introduce a natural micro-pause */
const PAUSE_CHARS = new Set([".", "!", "?", ",", ";", ":", "—", "…", "\n", "。", "！", "？", "，", "；"]);
const LONG_PAUSE_CHARS = new Set([".", "!", "?", "\n", "。", "！", "？"]);

/** Character adjacencies for "typo" simulation — nearby keys */
const TYPO_NEIGHBORS: Record<string, string> = {
    a: "sq", b: "vn", c: "xv", d: "sf", e: "wr", f: "dg", g: "fh",
    h: "gj", i: "uo", j: "hk", k: "jl", l: "k;", m: "n,", n: "bm",
    o: "ip", p: "o[", q: "wa", r: "et", s: "ad", t: "ry", u: "yi",
    v: "cb", w: "qe", x: "zc", y: "tu", z: "xs",
};

/** Emotions that can trigger correction behavior */
const HESITANT_EMOTIONS = new Set(["shy", "worried", "nervous", "sad", "thinking"]);

// ── Hook ──────────────────────────────────────────

interface UseTypingRevealOptions {
    /** Callback to update displayed text per tick */
    onReveal: (visibleText: string) => void;
    /** Whether the hook is active (streaming in progress) */
    active: boolean;
}

/**
 * useTypingReveal — buffers incoming delta text and reveals it
 * character-by-character at variable speed based on emotion.
 *
 * Usage:
 *   const { pushDelta, flush, reset } = useTypingReveal({ onReveal, active });
 *   // In chat delta handler: pushDelta(delta)
 *   // On stream end: flush()
 *   // On new message: reset()
 */
export function useTypingReveal({ onReveal, active }: UseTypingRevealOptions) {
    const bufferRef = useRef("");       // Pending unrevealed text
    const revealedRef = useRef("");     // Already revealed text
    const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const speedRef = useRef<TypingSpeed>("Normal");
    const emotionRef = useRef("calm");
    const correctionCooldown = useRef(0); // Prevent correction spam
    const activeRef = useRef(active);

    // Keep activeRef in sync with the prop
    useEffect(() => { activeRef.current = active; }, [active]);

    // Listen for chat-typing events from backend to set speed
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<TypingEvent>("chat-typing", (ev) => {
            speedRef.current = ev.payload.speed;
        }).then(fn => { unlisten = fn; });

        return () => unlisten?.();
    }, []);

    // Listen for expression events to track current emotion
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<{ expression: string }>("chat-expression", (ev) => {
            emotionRef.current = ev.payload.expression;
        }).then(fn => { unlisten = fn; });

        return () => unlisten?.();
    }, []);

    // ── Helpers ─────────────────────────

    const getCharDelay = useCallback(() => {
        const { base, variance } = SPEED_DELAYS[speedRef.current] || SPEED_DELAYS.Normal;
        return base + (Math.random() - 0.5) * variance;
    }, []);

    const shouldCorrect = useCallback(() => {
        // Only correct for hesitant emotions, and only ~5% at word boundaries
        if (!HESITANT_EMOTIONS.has(emotionRef.current)) return false;
        if (correctionCooldown.current > 0) return false;
        return Math.random() < 0.05;
    }, []);

    const generateTypo = useCallback((char: string): string => {
        const lower = char.toLowerCase();
        const neighbors = TYPO_NEIGHBORS[lower];
        if (!neighbors) return char; // No neighbor map for this char
        const typoChar = neighbors[Math.floor(Math.random() * neighbors.length)];
        return char === char.toUpperCase() ? typoChar.toUpperCase() : typoChar;
    }, []);

    // ── Core reveal loop ─────────────────

    const tick = useCallback(() => {
        if (bufferRef.current.length === 0) {
            timerRef.current = null;
            return;
        }

        const nextChar = bufferRef.current[0];
        bufferRef.current = bufferRef.current.slice(1);
        correctionCooldown.current = Math.max(0, correctionCooldown.current - 1);

        // Check for correction simulation at word boundaries
        if (nextChar === " " && shouldCorrect() && bufferRef.current.length >= 2) {
            // Simulate typing 1-2 wrong chars, then backspacing
            const wrongCount = 1 + Math.floor(Math.random() * 2);
            const wrongChars = bufferRef.current.slice(0, wrongCount)
                .split("").map(c => generateTypo(c)).join("");

            correctionCooldown.current = 20; // Cooldown: ~20 chars before next correction

            // Phase 1: Show space + wrong chars
            revealedRef.current += nextChar + wrongChars;
            onReveal(revealedRef.current);

            // Phase 2: After a pause, remove wrong chars
            timerRef.current = setTimeout(() => {
                revealedRef.current = revealedRef.current.slice(0, -(wrongChars.length));
                onReveal(revealedRef.current);

                // Phase 3: Continue with correct chars after brief pause
                timerRef.current = setTimeout(() => {
                    tick();
                }, 120 + Math.random() * 80);
            }, 300 + Math.random() * 200);
            return;
        }

        // Normal reveal
        revealedRef.current += nextChar;
        onReveal(revealedRef.current);

        // Calculate next delay
        let delay = getCharDelay();
        if (LONG_PAUSE_CHARS.has(nextChar)) {
            delay += 150 + Math.random() * 100; // Sentence-end pause
        } else if (PAUSE_CHARS.has(nextChar)) {
            delay += 60 + Math.random() * 40;   // Comma pause
        }

        timerRef.current = setTimeout(tick, delay);
    }, [onReveal, getCharDelay, shouldCorrect, generateTypo]);

    // ── Public API ──────────────────────

    const pushDelta = useCallback((delta: string) => {
        bufferRef.current += delta;
        // Start ticking if not already — use activeRef to avoid stale closure
        if (!timerRef.current && activeRef.current) {
            timerRef.current = setTimeout(tick, getCharDelay());
        }
    }, [tick, getCharDelay]);

    const flush = useCallback(() => {
        // Immediately reveal all remaining buffered text
        if (timerRef.current) {
            clearTimeout(timerRef.current);
            timerRef.current = null;
        }
        if (bufferRef.current.length > 0) {
            revealedRef.current += bufferRef.current;
            bufferRef.current = "";
            onReveal(revealedRef.current);
        }
    }, [onReveal]);

    const reset = useCallback(() => {
        if (timerRef.current) {
            clearTimeout(timerRef.current);
            timerRef.current = null;
        }
        bufferRef.current = "";
        revealedRef.current = "";
        speedRef.current = "Normal";
        correctionCooldown.current = 0;
    }, []);

    // Cleanup on unmount
    useEffect(() => {
        return () => {
            if (timerRef.current) clearTimeout(timerRef.current);
        };
    }, []);

    return { pushDelta, flush, reset };
}
