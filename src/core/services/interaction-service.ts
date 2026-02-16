/**
 * InteractionService — Handles physical touch/click reactions on the Live2D model.
 *
 * Maps hit area taps to emotion changes, motion playback, and spoken lines.
 * Supports combo detection (rapid taps) and cooldown to prevent spam.
 * Also provides preset action sequences (chained animations).
 */
import type { EmotionState, ActionIntent } from "../../features/live2d/Live2DController";

import { streamChat } from "../../lib/kokoro-bridge";

// ── Types ──────────────────────────────────────────

interface TouchReaction {
    emotion: EmotionState;
    action?: ActionIntent;
}

interface ActionStep {
    emotion?: EmotionState;
    action?: ActionIntent;
    delayMs: number;
}

interface ActionSequence {
    name: string;
    steps: ActionStep[];
}

export interface InteractionEvent {
    hitArea: string;
    emotion: EmotionState;
    action?: ActionIntent;
    isCombo: boolean;   // Whether this was triggered by rapid tapping
}

// ── Reaction Maps ──────────────────────────────────

const TOUCH_REACTIONS: Record<string, TouchReaction[]> = {
    Head: [
        { emotion: "happy", action: "nod" },
        { emotion: "happy", action: "nod" },
    ],
    Body: [
        { emotion: "surprised", action: "surprise" },
        { emotion: "shy", action: "shy" },
    ],
    Face: [
        { emotion: "shy", action: "shy" },
        { emotion: "happy", action: "nod" },
    ],
};

// Combo reactions: triggered after rapid consecutive taps on same area
const COMBO_REACTIONS: Record<string, TouchReaction> = {
    Head: { emotion: "shy", action: "shy" },
    Body: { emotion: "angry", action: "shake" },
    Face: { emotion: "angry", action: "shake" },
};

// Fallback for unknown hit areas
const DEFAULT_REACTION: TouchReaction = {
    emotion: "surprised",
    action: "surprise",
};

// ── Preset Sequences ───────────────────────────────

const PRESET_SEQUENCES: ActionSequence[] = [
    {
        name: "greeting",
        steps: [
            { emotion: "happy", action: "wave", delayMs: 500 },
            { emotion: "happy", action: "nod", delayMs: 800 },
        ],
    },
    {
        name: "tantrum",
        steps: [
            { emotion: "angry", action: "shake", delayMs: 300 },
            { emotion: "angry", action: "shake", delayMs: 300 },
            { emotion: "sad", delayMs: 500 },
        ],
    },
    {
        name: "celebrate",
        steps: [
            { emotion: "excited", action: "cheer", delayMs: 400 },
            { emotion: "happy", action: "dance", delayMs: 600 },
            { emotion: "happy", action: "wave", delayMs: 500 },
        ],
    },
    {
        name: "sleepy",
        steps: [
            { emotion: "thinking", action: "idle", delayMs: 800 },
            { emotion: "neutral", action: "nod", delayMs: 1000 },
        ],
    },
];

// ── Service ────────────────────────────────────────

type ReactionCallback = (event: InteractionEvent) => void;
type ControllerProxy = {
    setEmotion: (emotion: EmotionState) => void;
    playActionMotion: (action: ActionIntent) => void;
};

export class InteractionService {
    private cooldownMs = 500;
    private comboThresholdMs = 1500;   // Max gap between taps to count as combo
    private comboTriggerCount = 3;     // Taps needed to trigger combo reaction

    private lastTapTime = 0;
    private lastHitArea = "";
    private consecutiveTaps = 0;
    private listeners: ReactionCallback[] = [];

    /**
     * Handle a hit area tap from the Live2D model.
     * Returns the interaction event, also broadcasts to listeners.
     */
    async handleTouch(hitArea: string, controller: ControllerProxy): Promise<InteractionEvent | null> {
        const now = Date.now();

        // Cooldown check
        if (now - this.lastTapTime < this.cooldownMs) {
            return null;
        }

        // Combo tracking
        if (hitArea === this.lastHitArea && now - this.lastTapTime < this.comboThresholdMs) {
            this.consecutiveTaps++;
        } else {
            this.consecutiveTaps = 1;
        }

        this.lastTapTime = now;
        this.lastHitArea = hitArea;

        // Determine reaction
        let reaction: TouchReaction;
        let isCombo = false;

        if (this.consecutiveTaps >= this.comboTriggerCount && COMBO_REACTIONS[hitArea]) {
            reaction = COMBO_REACTIONS[hitArea];
            isCombo = true;
            this.consecutiveTaps = 0; // Reset combo counter
        } else {
            const candidates = TOUCH_REACTIONS[hitArea] || [DEFAULT_REACTION];
            reaction = candidates[Math.floor(Math.random() * candidates.length)];
        }

        // Apply physical reaction (motion/emotion) immediately
        controller.setEmotion(reaction.emotion);
        if (reaction.action) {
            controller.playActionMotion(reaction.action);
        }

        // Trigger LLM to respond to the touch
        // We send a descriptive action message.
        // The message format is "(User touches your [Area])"
        const actionDescription = isCombo
            ? `(User repeatedly touches your ${hitArea})`
            : `(User touches your ${hitArea})`;

        try {
            await streamChat({
                message: actionDescription,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
            });
        } catch (err) {
            console.error("[InteractionService] Failed to trigger LLM:", err);
        }

        // Build event
        const event: InteractionEvent = {
            hitArea,
            emotion: reaction.emotion,
            action: reaction.action,
            isCombo,
        };

        // Broadcast
        this.broadcast(event);

        return event;
    }

    /**
     * Play a preset action sequence (chained motions + emotions).
     */
    async playSequence(name: string, controller: ControllerProxy): Promise<void> {
        const sequence = PRESET_SEQUENCES.find(s => s.name === name);
        if (!sequence) {
            console.warn(`[Interaction] Unknown sequence: ${name}`);
            return;
        }

        console.log(`[Interaction] Playing sequence: ${name}`);

        for (const step of sequence.steps) {
            if (step.emotion) {
                controller.setEmotion(step.emotion);
            }
            if (step.action) {
                controller.playActionMotion(step.action);
            }
            await new Promise(resolve => setTimeout(resolve, step.delayMs));
        }
    }

    /**
     * Get all available preset sequence names.
     */
    getSequenceNames(): string[] {
        return PRESET_SEQUENCES.map(s => s.name);
    }

    /**
     * Register a listener for interaction events.
     */
    onReaction(callback: ReactionCallback): () => void {
        this.listeners.push(callback);
        return () => {
            this.listeners = this.listeners.filter(l => l !== callback);
        };
    }

    private broadcast(event: InteractionEvent): void {
        for (const cb of this.listeners) {
            try {
                cb(event);
            } catch (err) {
                console.error("[Interaction] Listener error:", err);
            }
        }
    }
}

export const interactionService = new InteractionService();
