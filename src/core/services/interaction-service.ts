/**
 * InteractionService — LLM-driven touch reaction system.
 *
 * Detects gesture types (tap / long_press / rapid_tap) and delegates
 * all personality-aware reactions to the backend LLM pipeline.
 * A lightweight "surprised" confirmation animation bridges the latency gap.
 *
 * Also provides preset action sequences (chained animations).
 */
import type { EmotionState, ActionIntent } from "../../features/live2d/Live2DController";
import { streamChat, onChatDone } from "../../lib/kokoro-bridge";

// ── Types ──────────────────────────────────────────

export type GestureType = "tap" | "long_press" | "rapid_tap";

export interface GestureEvent {
    hitArea: string;
    gesture: GestureType;
    consecutiveTaps: number;
}

export interface InteractionEvent {
    hitArea: string;
    gesture: GestureType;
    isCombo: boolean;
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
    private comboThresholdMs = 1500;
    private comboTriggerCount = 3;

    private lastTapTime = 0;
    private lastHitArea = "";
    private consecutiveTaps = 0;
    private listeners: ReactionCallback[] = [];

    // Busy state: prevents overlapping LLM calls from touch
    private isChatBusy = false;
    private pendingGesture: { gesture: GestureEvent; controller: ControllerProxy } | null = null;
    private unlistenChatDone: (() => void) | null = null;

    constructor() {
        // Listen for chat-done to know when LLM finishes responding
        onChatDone(() => {
            this.isChatBusy = false;
            this.processPendingGesture();
        }).then(fn => { this.unlistenChatDone = fn; });
    }

    /**
     * Handle a gesture event from the Live2D viewer.
     * Plays a brief confirmation animation, then sends to LLM.
     */
    async handleGesture(gesture: GestureEvent, controller: ControllerProxy): Promise<InteractionEvent | null> {
        const now = Date.now();

        // Cooldown check
        if (now - this.lastTapTime < this.cooldownMs) {
            return null;
        }

        // Rapid-tap tracking for "tap" gestures
        if (gesture.gesture === "tap") {
            if (gesture.hitArea === this.lastHitArea && now - this.lastTapTime < this.comboThresholdMs) {
                this.consecutiveTaps++;
            } else {
                this.consecutiveTaps = 1;
            }

            // Upgrade to rapid_tap if threshold reached
            if (this.consecutiveTaps >= this.comboTriggerCount) {
                gesture = {
                    ...gesture,
                    gesture: "rapid_tap",
                    consecutiveTaps: this.consecutiveTaps,
                };
                this.consecutiveTaps = 0;
            }
        } else {
            this.consecutiveTaps = 0;
        }

        this.lastTapTime = now;
        this.lastHitArea = gesture.hitArea;

        // Play lightweight confirmation animation (expression only, no motion)
        controller.setEmotion("surprised");

        // If LLM is busy, queue this gesture (keep only the latest)
        if (this.isChatBusy) {
            this.pendingGesture = { gesture, controller };
            // Still broadcast the event so listeners know a touch happened
            const event: InteractionEvent = {
                hitArea: gesture.hitArea,
                gesture: gesture.gesture,
                isCombo: gesture.gesture === "rapid_tap",
            };
            this.broadcast(event);
            return event;
        }

        return this.sendGestureToLLM(gesture, controller);
    }

    private async sendGestureToLLM(gesture: GestureEvent, _controller: ControllerProxy): Promise<InteractionEvent> {
        this.isChatBusy = true;

        // Format message based on gesture type
        const message = this.formatGestureMessage(gesture);

        try {
            await streamChat({
                message,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
                hidden: true,
            });
        } catch (err) {
            console.error("[InteractionService] Failed to trigger LLM:", err);
            this.isChatBusy = false;
        }

        const event: InteractionEvent = {
            hitArea: gesture.hitArea,
            gesture: gesture.gesture,
            isCombo: gesture.gesture === "rapid_tap",
        };

        this.broadcast(event);
        return event;
    }

    private formatGestureMessage(gesture: GestureEvent): string {
        let action: string;
        switch (gesture.gesture) {
            case "tap":
                action = `(User taps your ${gesture.hitArea})`;
                break;
            case "long_press":
                action = `(User holds your ${gesture.hitArea})`;
                break;
            case "rapid_tap":
                action = `(User rapidly pokes your ${gesture.hitArea} ${gesture.consecutiveTaps} times)`;
                break;
        }

        // Reinforce response language so LLM doesn't get pulled into English
        const lang = localStorage.getItem("kokoro_response_language");
        if (lang) {
            action += `\n[Respond in ${lang}]`;
        }
        return action;
    }

    private processPendingGesture(): void {
        if (!this.pendingGesture) return;
        const { gesture, controller } = this.pendingGesture;
        this.pendingGesture = null;
        this.sendGestureToLLM(gesture, controller);
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

    destroy(): void {
        this.unlistenChatDone?.();
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
