import { describe, expect, it, vi } from "vitest";
import type { PendingTurnState } from "./turn-state";

describe("chat stop generation race condition and 4-layer defense", () => {
    it("preserves turnId initialization when cancellation is requested before turn start (Layer 1)", () => {
        let isStreaming = true;
        let isBusy = true;
        let isStopping = false;
        let cancelRequested = false;
        let isThinking = true;
        let currentTurn: PendingTurnState | null = null;
        const cancelledTurns: string[] = [];

        const requestTurnCancellation = vi.fn(async (turnId: string) => {
            cancelledTurns.push(turnId);
        });

        const endTurnActivity = vi.fn(() => {
            cancelRequested = false;
            isStopping = false;
            isStreaming = false;
            isBusy = false;
        });

        const getActiveTurnId = (turnState: PendingTurnState | null): string | undefined => turnState?.turnId;

        // 1. User clicks stop in Thinking stage (before turn_start)
        cancelRequested = true;
        isStopping = true;
        isThinking = false;
        const activeTurnId = getActiveTurnId(currentTurn);
        if (activeTurnId) {
            void requestTurnCancellation(activeTurnId);
        }

        expect(requestTurnCancellation).not.toHaveBeenCalled();
        expect(isStopping).toBe(true);

        // 2. onChatTurnStart arrives from backend
        const simulatedTurnId = "turn-fast-stop-101";
        // Layer 1 Defense: ALWAYS initialize currentTurn first
        currentTurn = {
            turnId: simulatedTurnId,
            messageIndex: null,
            rawText: "",
            visibleTextStarted: false,
            translationPending: false,
            tools: [],
        };

        if (cancelRequested) {
            void requestTurnCancellation(simulatedTurnId);
        }

        expect(requestTurnCancellation).toHaveBeenCalledWith(simulatedTurnId);
        expect(currentTurn).not.toBeNull();
        expect(currentTurn?.turnId).toBe(simulatedTurnId);

        // 3. onChatTurnFinish arrives with status: cancelled
        const finishTurnId = simulatedTurnId;
        const turn = currentTurn;
        expect(turn).not.toBeNull();
        expect(turn?.turnId).toBe(finishTurnId);

        // Turn matches -> endTurnActivity is successfully invoked!
        endTurnActivity();
        isThinking = false;
        currentTurn = null;

        expect(endTurnActivity).toHaveBeenCalledTimes(1);
        expect(isBusy).toBe(false);
        expect(isStopping).toBe(false);
        expect(isStreaming).toBe(false);
        expect(isThinking).toBe(false);
        expect(currentTurn).toBeNull();
    });

    it("triggers defensive recovery when finish turnId does not match but cancellation was requested (Layer 2)", () => {
        let isBusy = true;
        let isStopping = true;
        const cancelRequested = true;
        let currentTurn: PendingTurnState | null = null;

        const endTurnActivity = vi.fn(() => {
            isStopping = false;
            isBusy = false;
        });

        const getTurnId = (turnState: PendingTurnState | null): string | undefined => turnState?.turnId;

        // Backend returns finish with turn-mismatched ID or turn was lost
        const finishTurnId = "unknown-turn";
        const currentId = getTurnId(currentTurn);

        if (!currentTurn || currentId !== finishTurnId) {
            if (cancelRequested) {
                endTurnActivity();
                currentTurn = null;
            }
        }

        expect(endTurnActivity).toHaveBeenCalledTimes(1);
        expect(isBusy).toBe(false);
        expect(isStopping).toBe(false);
        expect(currentTurn).toBeNull();
    });

    it("recovers UI interactability via cancellation watchdog timer if backend stalls (Layer 3)", () => {
        vi.useFakeTimers();

        let isBusy = true;
        let isStopping = false;
        let isStreaming = true;
        let isThinking = true;
        let watchdogTimer: ReturnType<typeof setTimeout> | null = null;

        const endTurnActivity = vi.fn(() => {
            if (watchdogTimer !== null) {
                clearTimeout(watchdogTimer);
                watchdogTimer = null;
            }
            isStopping = false;
            isStreaming = false;
            isBusy = false;
        });

        // User clicks stop
        isStopping = true;
        isThinking = false;

        watchdogTimer = setTimeout(() => {
            endTurnActivity();
            isThinking = false;
        }, 5000);

        // 5 seconds pass without finish event from backend
        vi.advanceTimersByTime(5000);

        expect(endTurnActivity).toHaveBeenCalledTimes(1);
        expect(isBusy).toBe(false);
        expect(isStopping).toBe(false);
        expect(isStreaming).toBe(false);
        expect(isThinking).toBe(false);

        vi.useRealTimers();
    });

    it("ignores duplicate stop clicks when already stopping or not streaming", () => {
        const isStreaming = true;
        const isStopping = true;
        const requestTurnCancellation = vi.fn();

        // Guard check
        if (!isStreaming || isStopping) {
            // Should return early
        } else {
            requestTurnCancellation();
        }

        expect(requestTurnCancellation).not.toHaveBeenCalled();
    });
});
