// pattern: Imperative Shell

/**
 * InteractionService — LLM-driven touch reaction system.
 *
 * Detects gesture types (tap / long_press / rapid_tap) and delegates
 * all personality-aware reactions to the backend LLM pipeline.
 *
 */
import type { CueName } from "../../features/live2d/Live2DController";
import { isChatTurnBusyError } from "../../lib/kokoro-bridge";
import { chatApplication, submitChat } from "../../lib/chat/application";

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

// ── Service ────────────────────────────────────────

/**
 * Normalize hit area names to natural English descriptions for LLM messages.
 * Handles both legacy HitArea names (e.g. "Body", "Head") and new region
 * descriptions that already come through as natural names.
 */
const HIT_AREA_DESCRIPTIONS: Record<string, string> = {
    // Legacy HitArea names (from model3.json)
    Body: "body",
    Head: "head",
    Face: "face",
    // Already-natural names pass through as-is
};

function describeHitArea(hitArea: string): string {
    return HIT_AREA_DESCRIPTIONS[hitArea] ?? hitArea;
}

type ReactionCallback = (event: InteractionEvent) => void;
type ControllerProxy = {
    playCue: (cue: CueName) => void;
    resolveInteractionSemanticCue: (gesture: GestureType, hitArea: string) => CueName | null;
};

export class InteractionService {
    private cooldownMs = 500;
    private comboThresholdMs = 1500;
    private comboTriggerCount = 3;

    private lastTapTime = 0;
    private lastHitArea = "";
    private consecutiveTaps = 0;
    private listeners: ReactionCallback[] = [];

    private pendingGesture: { gesture: GestureEvent; controller: ControllerProxy } | null = null;
    private activeClientRequestId: string | null = null;
    private retryTimer: ReturnType<typeof setTimeout> | null = null;
    private unlisten: (() => void) | null = null;

    constructor() {
        this.unlisten = chatApplication.subscribe(() => {
            if (!chatApplication.getSnapshot().busy && !this.activeClientRequestId) this.processPendingGesture();
        });
    }

    /**
     * Handle a gesture event from the Live2D viewer.
     * The frontend only reports the gesture; any visual reaction should come
     * from configured cues or the backend response, not fixed cue names.
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

        // Semantic motion fallback: immediately play standard mapped cues (e.g. Head -> nod/smile)
        const mappedCue = controller.resolveInteractionSemanticCue(gesture.gesture, gesture.hitArea);
        if (mappedCue) {
            controller.playCue(mappedCue);
        }

        return this.sendGestureToLLM(gesture, controller);
    }

    async triggerInteraction(gesture: GestureEvent, controller: ControllerProxy): Promise<InteractionEvent | null> {
        return this.handleGesture(gesture, controller);
    }

    private async sendGestureToLLM(gesture: GestureEvent, _controller: ControllerProxy): Promise<InteractionEvent> {
        const event: InteractionEvent = {
            hitArea: gesture.hitArea, gesture: gesture.gesture,
            isCombo: gesture.gesture === "rapid_tap",
        };
        const clientRequestId = `interaction_${crypto.randomUUID()}`;
        this.activeClientRequestId = clientRequestId;
        try {
            await submitChat({ message: this.formatGestureMessage(gesture), client_request_id: clientRequestId, hidden: true });
        } catch (error) {
            // Retain the pre-existing single coalesced gesture retry. The backend
            // rejects contention atomically; this service never pre-approves chat.
            if (isChatTurnBusyError(error)) this.pendingGesture = { gesture, controller: _controller };
            else console.error("[InteractionService] Failed to trigger LLM:", error);
        } finally {
            if (this.activeClientRequestId === clientRequestId) this.activeClientRequestId = null;
            // The idle notification can precede this request's busy rejection.
            // Yield with the normal cooldown so stale idle snapshots cannot spin.
            if (this.unlisten && this.pendingGesture && !chatApplication.getSnapshot().busy && !this.retryTimer) {
                this.retryTimer = setTimeout(() => {
                    this.retryTimer = null;
                    this.processPendingGesture();
                }, this.cooldownMs);
            }
        }
        this.broadcast(event);
        return event;
    }

    private formatGestureMessage(gesture: GestureEvent): string {
        const area = describeHitArea(gesture.hitArea);
        let action = "";
        switch (gesture.gesture) {
            case "tap":
                action = `(User gently poked your ${area})`;
                break;
            case "long_press":
                action = `(User pressed and held their finger on your ${area})`;
                break;
            case "rapid_tap":
                action = `(User rapidly pokes your ${area} ${gesture.consecutiveTaps} times)`;
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
        if (!this.unlisten || !this.pendingGesture || this.activeClientRequestId || chatApplication.getSnapshot().busy) return;
        if (this.retryTimer) clearTimeout(this.retryTimer);
        this.retryTimer = null;
        const { gesture, controller } = this.pendingGesture;
        this.pendingGesture = null;
        this.sendGestureToLLM(gesture, controller);
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
        this.unlisten?.();
        this.unlisten = null;
        this.pendingGesture = null;
        if (this.retryTimer) clearTimeout(this.retryTimer);
        this.retryTimer = null;
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
