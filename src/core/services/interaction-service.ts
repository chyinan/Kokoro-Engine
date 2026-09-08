// pattern: Imperative Shell

/**
 * InteractionService — LLM-driven touch reaction system.
 *
 * Detects gesture types (tap / long_press / rapid_tap) and delegates
 * all personality-aware reactions to the backend LLM pipeline.
 *
 */
import type { CueName } from "../../features/live2d/Live2DController";
import { streamChat, cancelChatTurn, onChatTurnFinish, onChatTurnStart, isChatBusy, isChatTurnBusyError, getMemoryEmbeddingModelStatus } from "../../lib/kokoro-bridge";
import { emit, listen } from "@tauri-apps/api/event";
import { requestMemoryModelDialog } from "../../lib/memory-model-gate";

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

    // Busy state: prevents overlapping LLM calls from touch
    private isChatBusy = false;
    private activeClientRequestId: string | null = null;
    private pendingGesture: { gesture: GestureEvent; controller: ControllerProxy } | null = null;
    private unlistenChatDone: (() => void) | null = null;
    private unlistenChatStart: (() => void) | null = null;
    private unlistenChatAccepted: (() => void) | null = null;
    private unlistenChatRejected: (() => void) | null = null;
    private lastCompletedClientRequestId: string | null = null;
    private pendingHandshakes = new Map<string, (val: { accepted: true; conversation_id?: string } | { accepted: false; reason?: string; timeout?: boolean }) => void>();

    constructor() {
        // Listen for turn-start to know when any chat turn begins (ChatPanel, PetWindow, etc.)
        onChatTurnStart((event) => {
            if (this.activeClientRequestId && event.client_request_id === this.activeClientRequestId) {
                return;
            }
            this.isChatBusy = true;
        }).then(fn => { this.unlistenChatStart = fn; });

        // Listen for acceptance from ChatPanel
        listen<{ client_request_id?: string; conversation_id?: string }>("interaction-trigger-accepted", (event) => {
            const payload = (event as any)?.payload ?? event;
            const reqId = payload?.client_request_id;
            if (reqId && this.pendingHandshakes.has(reqId)) {
                this.pendingHandshakes.get(reqId)!({
                    accepted: true,
                    conversation_id: payload?.conversation_id,
                });
                this.pendingHandshakes.delete(reqId);
            }
        }).then(fn => { this.unlistenChatAccepted = fn; });

        // Listen for rejection from ChatPanel if it was busy
        listen<{ client_request_id?: string; reason?: string }>("interaction-trigger-rejected", (event) => {
            const payload = (event as any)?.payload ?? event;
            const reqId = payload?.client_request_id;
            if (reqId && this.pendingHandshakes.has(reqId)) {
                this.pendingHandshakes.get(reqId)!({
                    accepted: false,
                    reason: payload?.reason,
                });
                this.pendingHandshakes.delete(reqId);
            }
            if (reqId && reqId !== this.activeClientRequestId) {
                return;
            }
            if (this.activeClientRequestId && reqId === this.activeClientRequestId) {
                this.activeClientRequestId = null;
                cancelChatTurn(reqId, "interaction_trigger_rejected").catch(() => {});
            }
            this.isChatBusy = true;
        }).then(fn => { this.unlistenChatRejected = fn; });

        // Listen for turn-finish to know when LLM finishes responding
        onChatTurnFinish((event) => {
            if (this.activeClientRequestId !== null) {
                if (event.client_request_id !== this.activeClientRequestId) {
                    return;
                }
                this.completeInteractionTurn(event.client_request_id);
                return;
            }
            if (event.client_request_id !== null && event.client_request_id !== undefined
                && event.client_request_id === this.lastCompletedClientRequestId) {
                return;
            }
            this.isChatBusy = false;
            this.processPendingGesture();
        }).then(fn => { this.unlistenChatDone = fn; });
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
        const busy = this.isChatBusy || (await isChatBusy().catch(() => false));
        if (busy) {
            this.isChatBusy = true;
            this.pendingGesture = { gesture, controller: _controller };
            const event: InteractionEvent = {
                hitArea: gesture.hitArea,
                gesture: gesture.gesture,
                isCombo: gesture.gesture === "rapid_tap",
            };
            this.broadcast(event);
            return event;
        }

        this.isChatBusy = true;

        try {
            const status = await getMemoryEmbeddingModelStatus();
            if (!status.installed) {
                requestMemoryModelDialog();
                this.isChatBusy = false;
                const event: InteractionEvent = {
                    hitArea: gesture.hitArea,
                    gesture: gesture.gesture,
                    isCombo: gesture.gesture === "rapid_tap",
                };
                this.broadcast(event);
                return event;
            }
        } catch (err) {
            console.error("[InteractionService] Failed to query memory model status:", err);
            requestMemoryModelDialog();
            this.isChatBusy = false;
            const event: InteractionEvent = {
                hitArea: gesture.hitArea,
                gesture: gesture.gesture,
                isCombo: gesture.gesture === "rapid_tap",
            };
            this.broadcast(event);
            return event;
        }

        // Format message based on gesture type
        const message = this.formatGestureMessage(gesture);
        const clientRequestId = `interaction_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
        this.activeClientRequestId = clientRequestId;

        const handshakePromise = new Promise<{ accepted: true; conversation_id?: string } | { accepted: false; reason?: string; timeout?: boolean }>((resolve) => {
            this.pendingHandshakes.set(clientRequestId, resolve);
        });

        let timeoutId: ReturnType<typeof setTimeout> | null = null;
        const timeoutPromise = new Promise<{ accepted: false; timeout: true }>((resolve) => {
            timeoutId = setTimeout(() => resolve({ accepted: false, timeout: true }), 1000);
        });

        // Notify ChatPanel to start streaming (same pattern as proactive-trigger)
        await emit("interaction-trigger", {
            gesture: gesture.gesture,
            hitArea: gesture.hitArea,
            client_request_id: clientRequestId,
        });

        const handshake = await Promise.race([handshakePromise, timeoutPromise]);
        if (timeoutId) clearTimeout(timeoutId);
        this.pendingHandshakes.delete(clientRequestId);

        if (!handshake.accepted) {
            console.warn("[InteractionService] Interaction trigger rejected or timed out:", handshake);
            if (this.activeClientRequestId === clientRequestId) {
                this.activeClientRequestId = null;
            }
            if ("timeout" in handshake && handshake.timeout) {
                this.isChatBusy = false;
                emit("interaction-trigger-failed", {
                    client_request_id: clientRequestId,
                    error: "handshake_timeout",
                }).catch(() => {});
            } else {
                this.isChatBusy = true;
                this.pendingGesture = { gesture, controller: _controller };
            }
            const event: InteractionEvent = {
                hitArea: gesture.hitArea,
                gesture: gesture.gesture,
                isCombo: gesture.gesture === "rapid_tap",
            };
            this.broadcast(event);
            return event;
        }

        try {
            const response = await streamChat({
                message,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
                client_request_id: clientRequestId,
                conversation_id: handshake.conversation_id,
                hidden: true,
            });
            if (response?.status && response.status !== "completed" && response.status !== "cancelled") {
                throw new Error(`chat turn ${response.status}`);
            }
            // The backend response is the authoritative fallback when the
            // terminal event was dropped by the event bridge.
            this.completeInteractionTurn(clientRequestId);
        } catch (err) {
            console.error("[InteractionService] Failed to trigger LLM:", err);
            this.pendingHandshakes.delete(clientRequestId);
            if (timeoutId) clearTimeout(timeoutId);
            const ownsActiveTurn = this.activeClientRequestId === clientRequestId;
            if (ownsActiveTurn) {
                this.activeClientRequestId = null;
                if (isChatTurnBusyError(err)) {
                    // If rejected because chat turn is busy, queue this gesture for later
                    this.isChatBusy = true;
                    this.pendingGesture = { gesture, controller: _controller };
                } else {
                    this.isChatBusy = false;
                }
            }
            emit("interaction-trigger-failed", {
                client_request_id: clientRequestId,
                error: err instanceof Error ? err.message : String(err),
            }).catch(() => {});
        }

        const event: InteractionEvent = {
            hitArea: gesture.hitArea,
            gesture: gesture.gesture,
            isCombo: gesture.gesture === "rapid_tap",
        };

        this.broadcast(event);
        return event;
    }

    private completeInteractionTurn(clientRequestId: string): void {
        if (this.activeClientRequestId !== clientRequestId) return;
        this.activeClientRequestId = null;
        this.lastCompletedClientRequestId = clientRequestId;
        this.isChatBusy = false;
        this.processPendingGesture();
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
        if (!this.pendingGesture) return;
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
        this.unlistenChatStart?.();
        this.unlistenChatAccepted?.();
        this.unlistenChatRejected?.();
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
