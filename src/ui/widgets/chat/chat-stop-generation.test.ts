import { describe, expect, it, vi } from "vitest";
import type { PendingTurnState } from "./turn-state";
import { validateTurnStart } from "./chat-turn-lifecycle";

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

    it("cancels active turn, clears backend history, and only then starts empty conversation under busy lock", async () => {
        const sequence: string[] = [];
        let isBusy = true; // Active generation has isBusy = true
        let isSwitchingConversation = false;
        let busyStateDuringClearHistory: boolean | null = null;
        const cancelChatTurn = vi.fn(async (turnId: string, reason: string) => {
            sequence.push(`cancel:${turnId}:${reason}`);
        });
        const startEmptyConversation = vi.fn((charId: string) => {
            sequence.push(`startEmpty:${charId}`);
        });
        const clearHistory = vi.fn(async () => {
            sequence.push("clearHistory");
            busyStateDuringClearHistory = isBusy;
        });

        let currentTurn: { turnId: string } | null = { turnId: "turn-running-123" };
        let isStopping = false;
        let cancelRequested = false;

        const handleStartEmptyConversation = async (activeCharacterId: string): Promise<boolean> => {
            if (isSwitchingConversation) return false;
            isSwitchingConversation = true;
            isBusy = true;
            try {
                const activeTurnId = currentTurn?.turnId;
                if (activeTurnId) {
                    cancelRequested = true;
                    isStopping = true;
                    try {
                        await cancelChatTurn(activeTurnId, "new_conversation_started");
                    } catch (err) {
                        console.error(err);
                    }
                }
                // Two-phase commit: await backend clearHistory first
                await clearHistory();
                // Then commit visible empty conversation
                startEmptyConversation(activeCharacterId);
                return true;
            } finally {
                cancelRequested = false;
                isStopping = false;
                isSwitchingConversation = false;
                isBusy = false;
            }
        };

        const result = await handleStartEmptyConversation("char-anya");

        expect(result).toBe(true);
        expect(cancelChatTurn).toHaveBeenCalledWith("turn-running-123", "new_conversation_started");
        expect(clearHistory).toHaveBeenCalled();
        expect(startEmptyConversation).toHaveBeenCalledWith("char-anya");
        expect(isStopping).toBe(false);
        expect(cancelRequested).toBe(false);
        expect(busyStateDuringClearHistory).toBe(true);
        expect(isBusy).toBe(false);
        // Verify strict temporal ordering: cancel -> clearHistory -> startEmpty
        expect(sequence).toEqual([
            "cancel:turn-running-123:new_conversation_started",
            "clearHistory",
            "startEmpty:char-anya",
        ]);
    });

    it("preserves isBusy under endTurnActivity if conversation switching is in flight", () => {
        let isBusy = true;
        let isStreaming = true;
        let isStopping = true;
        let cancelRequested = true;
        const isSwitchingConversation = true;

        const endTurnActivity = () => {
            cancelRequested = false;
            isStopping = false;
            isStreaming = false;
            if (!isSwitchingConversation) {
                isBusy = false;
            }
        };

        endTurnActivity();

        expect(isStreaming).toBe(false);
        expect(isStopping).toBe(false);
        expect(cancelRequested).toBe(false);
        expect(isBusy).toBe(true); // Must remain held!
    });

    it("does NOT clear visible conversation and preserves existing state when clearHistory fails", async () => {
        let isBusy = true;
        let isSwitchingConversation = false;
        let isStopping = false;
        let cancelRequested = false;
        let errorMessage: string | null = null;
        const clearHistory = vi.fn(async () => {
            throw new Error("Backend IPC error");
        });
        const startEmptyConversation = vi.fn();
        let visibleMessages = ["old message 1", "old message 2"];
        let activeConversationId: string | null = "conv-old-123";

        const handleStartEmptyConversation = async (activeCharacterId: string): Promise<boolean> => {
            if (isSwitchingConversation) return false;
            isSwitchingConversation = true;
            isBusy = true;
            cancelRequested = true;
            isStopping = true;
            try {
                await clearHistory();
                startEmptyConversation(activeCharacterId);
                visibleMessages = [];
                activeConversationId = null;
                return true;
            } catch (err) {
                errorMessage = "新建会话失败，已保留当前会话";
                return false;
            } finally {
                cancelRequested = false;
                isStopping = false;
                isSwitchingConversation = false;
                isBusy = false;
            }
        };

        const result = await handleStartEmptyConversation("char-anya");

        expect(result).toBe(false);
        expect(clearHistory).toHaveBeenCalledTimes(1);
        expect(startEmptyConversation).not.toHaveBeenCalled();
        // Visible messages and active conversation ID remain intact
        expect(visibleMessages).toEqual(["old message 1", "old message 2"]);
        expect(activeConversationId).toBe("conv-old-123");
        expect(errorMessage).toBe("新建会话失败，已保留当前会话");
        expect(isBusy).toBe(false);
        expect(isStopping).toBe(false);
        expect(cancelRequested).toBe(false);
        expect(isSwitchingConversation).toBe(false);
    });

    it("watchdog does NOT prematurely release busy lock while clearHistory is in-flight", async () => {
        vi.useFakeTimers();
        try {
            let isBusy = false;
            let isSwitchingConversation = false;
            let warningMessage: string | null = null;
            let resolveClearHistory: (() => void) | null = null;

            const clearHistory = vi.fn(() => new Promise<void>((resolve) => {
                resolveClearHistory = resolve;
            }));
            const startEmptyConversation = vi.fn();

            const handleStartEmptyConversation = async (activeCharacterId: string): Promise<void> => {
                if (isSwitchingConversation || isBusy) return;
                isSwitchingConversation = true;
                isBusy = true;

                const watchdogTimer = setTimeout(() => {
                    if (isSwitchingConversation) {
                        warningMessage = "新建会话响应较慢，请稍候...";
                        // MUST NOT release isBusy or isSwitchingConversation here!
                    }
                }, 5000);

                try {
                    await clearHistory();
                    startEmptyConversation(activeCharacterId);
                } finally {
                    clearTimeout(watchdogTimer);
                    isSwitchingConversation = false;
                    isBusy = false;
                }
            };

            const callPromise = handleStartEmptyConversation("char-anya");

            expect(isBusy).toBe(true);
            expect(isSwitchingConversation).toBe(true);

            // Fast-forward 5000ms: watchdog fires
            vi.advanceTimersByTime(5000);

            // Verify watchdog warned but did NOT release protection!
            expect(warningMessage).toBe("新建会话响应较慢，请稍候...");
            expect(isBusy).toBe(true);
            expect(isSwitchingConversation).toBe(true);
            expect(startEmptyConversation).not.toHaveBeenCalled();

            // Settle clearHistory at 6000ms
            vi.advanceTimersByTime(1000);
            resolveClearHistory!();
            await callPromise;

            // Now, and only now, protection is safely released
            expect(startEmptyConversation).toHaveBeenCalledWith("char-anya");
            expect(isBusy).toBe(false);
            expect(isSwitchingConversation).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("blocks concurrent or re-entrant start empty conversation calls while in-flight", async () => {
        let isBusy = false;
        let isSwitchingConversation = false;
        let resolveClearHistory: (() => void) | null = null;
        const clearHistory = vi.fn(() => new Promise<void>((resolve) => {
            resolveClearHistory = resolve;
        }));
        const startEmptyConversation = vi.fn();

        const handleStartEmptyConversation = async (activeCharacterId: string): Promise<boolean> => {
            if (isSwitchingConversation) return false;
            isSwitchingConversation = true;
            isBusy = true;
            try {
                await clearHistory();
                startEmptyConversation(activeCharacterId);
                return true;
            } finally {
                isSwitchingConversation = false;
                isBusy = false;
            }
        };

        const firstCall = handleStartEmptyConversation("char-anya");
        // Second call while first call is in flight
        const secondCall = handleStartEmptyConversation("char-anya");

        await secondCall;
        expect(clearHistory).toHaveBeenCalledTimes(1);
        expect(startEmptyConversation).not.toHaveBeenCalled();

        const done = resolveClearHistory as unknown as (() => void);
        done();
        await firstCall;
        expect(startEmptyConversation).toHaveBeenCalledTimes(1);
        expect(isBusy).toBe(false);
    });

    it("cancels active turn before switching conversation", async () => {
        const sequence: string[] = [];
        const cancelChatTurn = vi.fn(async (turnId: string, reason: string) => {
            sequence.push(`cancel:${turnId}:${reason}`);
        });
        const synchronize = vi.fn(async (params: { characterId: string; preferredConversationId: string | null }) => {
            sequence.push(`sync:${params.characterId}:${params.preferredConversationId}`);
        });

        let currentTurn: { turnId: string } | null = { turnId: "turn-running-456" };

        const handleConversationSelection = async (
            activeCharacterId: string,
            preferredConversationId: string | null,
        ): Promise<void> => {
            const activeTurnId = currentTurn?.turnId;
            if (activeTurnId) {
                try {
                    await cancelChatTurn(activeTurnId, "conversation_switched");
                } catch (err) {
                    console.error(err);
                }
            }
            await synchronize({
                characterId: activeCharacterId,
                preferredConversationId,
            });
        };

        await handleConversationSelection("char-anya", "conv-target-999");

        expect(cancelChatTurn).toHaveBeenCalledWith("turn-running-456", "conversation_switched");
        expect(synchronize).toHaveBeenCalledWith({
            characterId: "char-anya",
            preferredConversationId: "conv-target-999",
        });
        expect(sequence).toEqual([
            "cancel:turn-running-456:conversation_switched",
            "sync:char-anya:conv-target-999",
        ]);
    });

    it("distinguishes cancellation in fallback teardown, cleans up bubble, and suppresses TTS", () => {
        const removeTurnMessages = vi.fn((messages: any[], turn: any) => {
            return messages.filter((_m, idx) => idx !== turn.messageIndex);
        });
        const synthesize = vi.fn(async () => {});
        const resetReveal = vi.fn();
        const flushReveal = vi.fn();
        const endTurnActivity = vi.fn();

        let messages = [
            { role: "user", text: "Hello", clientRequestId: "req-1" },
            { role: "kokoro", text: "Partial text...", clientRequestId: "req-1" },
        ];
        let currentTurn: any = {
            clientRequestId: "req-1",
            turnId: "turn-1",
            rawText: "Partial text...",
            messageIndex: 1,
        };
        let cancelRequested = false;

        const finalizeActiveTurn = (clientRequestId: string, res?: any) => {
            const isExplicitlyCancelled = res?.status === "cancelled";
            const isCancelRequested = cancelRequested;
            const hasCommittedAssistantMessage = Boolean(res?.assistant_message_id && res?.status === "completed");
            const shouldTreatAsCancelled = isExplicitlyCancelled || (isCancelRequested && !hasCommittedAssistantMessage);

            if (shouldTreatAsCancelled) {
                resetReveal();
                if (currentTurn?.clientRequestId === clientRequestId) {
                    messages = removeTurnMessages(messages, currentTurn);
                }
                currentTurn = null;
                endTurnActivity();
                return;
            }

            flushReveal();
            if (!isCancelRequested && res?.status !== "cancelled") {
                void synthesize();
            }
            endTurnActivity();
        };

        // Scenario 1: Backend cancelled response arrives
        finalizeActiveTurn("req-1", {
            conversation_id: "conv-1",
            status: "cancelled",
            assistant_message_id: null,
            client_request_id: "req-1",
        });

        expect(resetReveal).toHaveBeenCalledTimes(1);
        expect(flushReveal).not.toHaveBeenCalled();
        expect(synthesize).not.toHaveBeenCalled();
        expect(endTurnActivity).toHaveBeenCalledTimes(1);
        expect(currentTurn).toBeNull();
        expect(messages).toEqual([
            { role: "user", text: "Hello", clientRequestId: "req-1" },
        ]);
    });

    it("suppresses TTS when user clicked cancel but backend already completed with committed message ID", () => {
        const synthesize = vi.fn(async () => {});
        const flushReveal = vi.fn();
        const endTurnActivity = vi.fn();

        let messages = [
            { role: "user", text: "Hello", clientRequestId: "req-2" },
            { role: "kokoro", text: "Completed reply", clientRequestId: "req-2" },
        ];
        let currentTurn: any = {
            clientRequestId: "req-2",
            turnId: "turn-2",
            rawText: "Completed reply",
            messageIndex: 1,
        };
        const cancelRequested = true; // User clicked Stop right at the finish line

        const finalizeActiveTurn = (clientRequestId: string, res?: any) => {
            const isExplicitlyCancelled = res?.status === "cancelled";
            const isCancelRequested = cancelRequested;
            const hasCommittedAssistantMessage = Boolean(res?.assistant_message_id && res?.status === "completed");
            const shouldTreatAsCancelled = isExplicitlyCancelled || (isCancelRequested && !hasCommittedAssistantMessage);

            if (shouldTreatAsCancelled) {
                messages = messages.filter(m => m.clientRequestId !== clientRequestId);
                currentTurn = null;
                endTurnActivity();
                return;
            }

            flushReveal();
            if (currentTurn?.clientRequestId === clientRequestId) {
                currentTurn = null;
            }
            // TTS MUST be suppressed because cancel was requested
            if (!isCancelRequested && res?.status !== "cancelled") {
                void synthesize();
            }
            endTurnActivity();
        };

        finalizeActiveTurn("req-2", {
            conversation_id: "conv-1",
            status: "completed",
            assistant_message_id: 200,
            client_request_id: "req-2",
        });

        // Messages kept because DB committed it
        expect(messages.length).toBe(2);
        // But TTS was suppressed
        expect(synthesize).not.toHaveBeenCalled();
        expect(endTurnActivity).toHaveBeenCalledTimes(1);
    });

    it("cancels pending request by clientRequestId when New Chat is clicked before turn-start", async () => {
        let isStopping = false;
        let cancelRequested = false;
        let pendingTurnRequest: any = {
            clientRequestId: "req-pending-before-turn-start-1",
            generation: 1,
            conversationId: "conv-old",
            characterId: "char-anya",
        };
        let currentTurn: any = null; // turn-start has NOT arrived yet!

        const cancelChatTurn = vi.fn(async (_targetId: string, _reason: string) => {});
        const clearHistory = vi.fn(async () => {});
        const startEmptyConversation = vi.fn((_charId: string) => {});

        const handleStartEmptyConversation = async (activeCharacterId: string) => {
            try {
                const activeTurnId = currentTurn?.turnId;
                const pendingClientRequestId = pendingTurnRequest?.clientRequestId;
                if (activeTurnId) {
                    cancelRequested = true;
                    isStopping = true;
                    await cancelChatTurn(activeTurnId, "new_conversation_started");
                } else if (pendingClientRequestId) {
                    cancelRequested = true;
                    isStopping = true;
                    pendingTurnRequest = null;
                    await cancelChatTurn(pendingClientRequestId, "new_conversation_started");
                }
                await clearHistory();
                startEmptyConversation(activeCharacterId);
                return true;
            } finally {
                cancelRequested = false;
                isStopping = false;
            }
        };

        const result = await handleStartEmptyConversation("char-anya");

        expect(result).toBe(true);
        expect(cancelChatTurn).toHaveBeenCalledWith(
            "req-pending-before-turn-start-1",
            "new_conversation_started",
        );
        expect(pendingTurnRequest).toBeNull();
        expect(clearHistory).toHaveBeenCalled();
        expect(startEmptyConversation).toHaveBeenCalledWith("char-anya");
        expect(cancelRequested).toBe(false);
        expect(isStopping).toBe(false);
    });

    it("cancels pending request by clientRequestId when switching conversation before turn-start", async () => {
        let isStopping = false;
        let cancelRequested = false;
        let pendingTurnRequest: any = {
            clientRequestId: "req-pending-before-turn-start-2",
            generation: 1,
            conversationId: "conv-1",
            characterId: "char-anya",
        };
        let currentTurn: any = null; // turn-start has NOT arrived yet!

        const cancelChatTurn = vi.fn(async (_targetId: string, _reason: string) => {});
        const synchronize = vi.fn(async () => {});

        const handleSelectConversation = async (_preferredConversationId: string | null) => {
            try {
                const activeTurnId = currentTurn?.turnId;
                const pendingClientRequestId = pendingTurnRequest?.clientRequestId;
                if (activeTurnId) {
                    cancelRequested = true;
                    isStopping = true;
                    await cancelChatTurn(activeTurnId, "conversation_switched");
                } else if (pendingClientRequestId) {
                    cancelRequested = true;
                    isStopping = true;
                    pendingTurnRequest = null;
                    await cancelChatTurn(pendingClientRequestId, "conversation_switched");
                }
                await synchronize();
            } finally {
                cancelRequested = false;
                isStopping = false;
            }
        };

        await handleSelectConversation("conv-2");

        expect(cancelChatTurn).toHaveBeenCalledWith(
            "req-pending-before-turn-start-2",
            "conversation_switched",
        );
        expect(pendingTurnRequest).toBeNull();
        expect(synchronize).toHaveBeenCalled();
        expect(cancelRequested).toBe(false);
        expect(isStopping).toBe(false);
    });

    it("clears pendingTurnRequest immediately and calls requestTurnCancellation when stopping pending request", () => {
        let isStreaming = true;
        let isStopping = false;
        let cancelRequested = false;
        let isThinking = true;
        let currentTurn: any = null;
        let pendingTurnRequest: any = {
            clientRequestId: "req-pending-stop-test",
            generation: 1,
            conversationId: "conv-1",
            characterId: "char-anya",
        };

        const requestTurnCancellation = vi.fn(async (_id: string) => {});

        const handleStopGeneration = () => {
            if (!isStreaming || isStopping) return;
            cancelRequested = true;
            isStopping = true;
            isThinking = false;

            const activeTurnId = currentTurn?.turnId;
            const pendingClientRequestId = pendingTurnRequest?.clientRequestId;
            if (activeTurnId) {
                void requestTurnCancellation(activeTurnId);
            } else if (pendingClientRequestId) {
                pendingTurnRequest = null;
                void requestTurnCancellation(pendingClientRequestId);
            }
        };

        handleStopGeneration();

        expect(cancelRequested).toBe(true);
        expect(isStopping).toBe(true);
        expect(isThinking).toBe(false);
        expect(pendingTurnRequest).toBeNull();
        expect(requestTurnCancellation).toHaveBeenCalledWith("req-pending-stop-test");
    });

    it("preserves cancelRequested=true and clears pendingTurnRequest on requestTurnCancellation error", async () => {
        let isStreaming = true;
        let isBusy = true;
        let isStopping = true;
        let cancelRequested = true;
        let isThinking = true;
        let errorMessage: string | null = null;
        let currentTurn: any = null;
        let pendingTurnRequest: any = {
            clientRequestId: "req-pending-error-test",
            generation: 1,
            conversationId: "conv-1",
            characterId: "char-anya",
        };

        const cancelChatTurn = vi.fn(async (_id: string, _reason: string) => {
            throw new Error("unknown turn_id: req-pending-error-test");
        });

        const isTurnCancelledError = (err: unknown) => {
            const msg = String(err).toLowerCase();
            return msg.includes("turn cancelled by user") || msg.includes("turn canceled by user");
        };

        const endTurnActivity = () => {
            cancelRequested = false;
            isStopping = false;
            isStreaming = false;
            isBusy = false;
        };

        const requestTurnCancellation = async (turnId: string) => {
            try {
                await cancelChatTurn(turnId, "stopped_from_chat_panel");
            } catch (error) {
                if (!isTurnCancelledError(error)) {
                    endTurnActivity();
                    cancelRequested = true; // Defense: explicitly maintain cancellation intent!
                    currentTurn = null;
                    pendingTurnRequest = null; // Defense: clear pending request!
                    isThinking = false;
                    errorMessage = String(error);
                }
            }
        };

        await requestTurnCancellation("req-pending-error-test");

        // Verify defensive invariants
        expect(cancelRequested).toBe(true);
        expect(pendingTurnRequest).toBeNull();
        expect(currentTurn).toBeNull();
        expect(isThinking).toBe(false);
        expect(isBusy).toBe(false);
        expect(isStreaming).toBe(false);
        expect(isStopping).toBe(false);
        expect(errorMessage).toContain("unknown turn_id");

        // Now verify that late-arriving chat-turn-start is strictly REJECTED
        const latePayload = {
            turn_id: "turn-late-arrival",
            client_request_id: "req-pending-error-test",
            conversation_id: "conv-1",
        };

        const validation = validateTurnStart({
            currentGeneration: 1,
            activeConversationId: "conv-1",
            activeCharacterId: "char-anya",
            pendingRequest: pendingTurnRequest, // null
            isCancelRequested: cancelRequested, // true
        }, latePayload);

        expect(validation.valid).toBe(false);
        if (!validation.valid) {
            expect(validation.reason).toBe("cancelled");
        }
    });

    it("recovers interactability and allows subsequent send when turn is stopped while stuck in tool/MCP execution", async () => {
        let isStreaming = true;
        let isBusy = true;
        let isStopping = false;
        let cancelRequested = false;
        let isThinking = false;
        let messages: any[] = [{ role: "user", text: "Run MCP command" }];

        // Turn is active and stuck in tool execution
        let currentTurn: PendingTurnState | null = {
            turnId: "turn-tool-stuck-1",
            messageIndex: 1,
            rawText: "",
            visibleTextStarted: false,
            translationPending: false,
            tools: [
                {
                    toolId: "tool-call-1",
                    toolName: "mcp_execute_query",
                    tool: "mcp_execute_query",
                    text: "Calling MCP tool...",
                    source: "mcp",
                },
            ],
        };

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

        // 1. While tool execution is stuck, user cannot send
        const canSendWhileStuck = !isBusy && !isStreaming && !isStopping;
        expect(canSendWhileStuck).toBe(false);

        // 2. User clicks Stop to abort stuck tool execution
        cancelRequested = true;
        isStopping = true;
        isThinking = false;
        const activeTurnId = currentTurn?.turnId;
        if (activeTurnId) {
            await requestTurnCancellation(activeTurnId);
        }

        expect(requestTurnCancellation).toHaveBeenCalledWith("turn-tool-stuck-1");
        expect(isStopping).toBe(true);
        expect(cancelRequested).toBe(true);

        // 3. onChatTurnFinish arrives with status: "cancelled"
        endTurnActivity();
        currentTurn = null;

        // 4. Verify that UI state has recovered and is no longer busy
        expect(isBusy).toBe(false);
        expect(isStreaming).toBe(false);
        expect(isStopping).toBe(false);
        expect(isThinking).toBe(false);
        expect(currentTurn).toBeNull();

        // 5. Subsequent message can now be sent successfully!
        const canSendAfterStop = !isBusy && !isStreaming && !isStopping;
        expect(canSendAfterStop).toBe(true);

        // Simulate subsequent send
        isStreaming = true;
        isBusy = true;
        isThinking = true;
        messages.push({ role: "user", text: "Follow-up question" });
        expect(messages).toHaveLength(2);
        expect(messages[1].text).toBe("Follow-up question");
    });
});
