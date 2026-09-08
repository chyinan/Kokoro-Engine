// @vitest-environment jsdom

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { InteractionService, type GestureEvent } from "./interaction-service";
import * as bridge from "../../lib/kokoro-bridge";
import * as eventApi from "@tauri-apps/api/event";

const { listeners } = vi.hoisted(() => ({
    listeners: {} as Record<string, (event: any) => void>,
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (event: string, cb: any) => {
        listeners[event] = cb;
        return () => {};
    }),
    emit: vi.fn(async () => {}),
}));

vi.mock("../../lib/kokoro-bridge", () => ({
    streamChat: vi.fn(async () => ({ status: "completed" })),
    cancelChatTurn: vi.fn(async () => {}),
    onChatTurnStart: vi.fn(async (cb: any) => {
        listeners["chat-turn-start"] = cb;
        return () => {};
    }),
    onChatTurnFinish: vi.fn(async (cb: any) => {
        listeners["chat-turn-finish"] = cb;
        return () => {};
    }),
    isChatBusy: vi.fn(async () => false),
    isChatTurnBusyError: vi.fn((err: any) => {
        return typeof err === "object" && err !== null && String(err.message || "").includes("chat_turn_busy");
    }),
    getMemoryEmbeddingModelStatus: vi.fn(async () => ({ installed: true })),
}));

vi.mock("../../lib/memory-model-gate", () => ({
    requestMemoryModelDialog: vi.fn(),
}));

describe("InteractionService", () => {
    let service: InteractionService;
    let mockController: {
        playCue: ReturnType<typeof vi.fn>;
        resolveInteractionSemanticCue: ReturnType<typeof vi.fn>;
    };

    beforeEach(() => {
        vi.clearAllMocks();
        for (const key of Object.keys(listeners)) {
            delete listeners[key];
        }
        mockController = {
            playCue: vi.fn(),
            resolveInteractionSemanticCue: vi.fn(() => null),
        };
        vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
            if (event === "interaction-trigger") {
                const reqId = (payload as any)?.client_request_id;
                listeners["interaction-trigger-accepted"]?.({
                    payload: { client_request_id: reqId, conversation_id: "conv-mock" },
                });
            }
        });
        service = new InteractionService();
    });

    afterEach(() => {
        service.destroy();
    });

    it("triggers LLM and emits interaction-trigger when idle", async () => {
        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        const result = await service.triggerInteraction(gesture, mockController as any);
        expect(result).not.toBeNull();
        expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger", expect.objectContaining({
            gesture: "tap",
            hitArea: "head",
            client_request_id: expect.stringMatching(/^interaction_\d+_[a-z0-9]+$/),
        }));
        expect(bridge.streamChat).toHaveBeenCalledWith(expect.objectContaining({
            hidden: true,
            message: expect.stringContaining("head"),
        }));
    });

    it("queues gesture and skips streamChat if backend is busy", async () => {
        vi.mocked(bridge.isChatBusy).mockResolvedValueOnce(true);

        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        const result = await service.triggerInteraction(gesture, mockController as any);
        expect(result).not.toBeNull();
        expect(bridge.streamChat).not.toHaveBeenCalled();
        expect(eventApi.emit).not.toHaveBeenCalledWith("interaction-trigger", expect.anything());

        // When turn finishes, the pending gesture should be processed
        listeners["chat-turn-finish"]?.({ turn_id: "turn-1", status: "completed" });
        await new Promise(resolve => setTimeout(resolve, 20));
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);
    });

    it("queues gesture when chat-turn-start marks service as busy", async () => {
        // External turn started (e.g. ChatPanel or PetWindow)
        listeners["chat-turn-start"]?.({ turn_id: "turn-ext" });

        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        await service.triggerInteraction(gesture, mockController as any);
        expect(bridge.streamChat).not.toHaveBeenCalled();

        // Finish external turn -> queued gesture executes
        listeners["chat-turn-finish"]?.({ turn_id: "turn-ext", status: "completed" });
        await new Promise(resolve => setTimeout(resolve, 20));
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);
    });

    it("emits interaction-trigger-failed and queues gesture when streamChat fails with chat_turn_busy", async () => {
        vi.mocked(bridge.streamChat).mockRejectedValueOnce(new Error("chat_turn_busy: Turn lock active"));

        const gesture: GestureEvent = {
            hitArea: "body",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        await service.triggerInteraction(gesture, mockController as any);

        expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger-failed", expect.objectContaining({
            client_request_id: expect.stringMatching(/^interaction_\d+_[a-z0-9]+$/),
            error: expect.stringContaining("chat_turn_busy"),
        }));

        // After previous turn finishes, queued gesture retries
        listeners["chat-turn-finish"]?.({ turn_id: "turn-prior", status: "completed" });
        await new Promise(resolve => setTimeout(resolve, 20));
        expect(bridge.streamChat).toHaveBeenCalledTimes(2);
    });

    it("marks busy when interaction-trigger-rejected is received", async () => {
        await listeners["interaction-trigger-rejected"]?.({ reason: "busy" });

        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        await service.triggerInteraction(gesture, mockController as any);
        expect(bridge.streamChat).not.toHaveBeenCalled();
    });

    it("ignores unrelated chat-turn-finish while own interaction turn is active, and only unlocks on matching client_request_id", async () => {
        let capturedRequestId: string | undefined;
        vi.mocked(bridge.streamChat).mockImplementationOnce(async (req: any) => {
            capturedRequestId = req.client_request_id;
            return { status: "completed" } as any;
        });

        const gesture1: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        await service.triggerInteraction(gesture1, mockController as any);
        expect(capturedRequestId).toBeDefined();
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);

        // Wait past 500ms tap cooldown
        await new Promise(resolve => setTimeout(resolve, 550));

        // While turn 1 is active, user performs a second gesture -> queued in pendingGesture
        const gesture2: GestureEvent = {
            hitArea: "body",
            gesture: "tap",
            consecutiveTaps: 1,
        };
        await service.triggerInteraction(gesture2, mockController as any);
        // Should NOT trigger streamChat because service is busy with turn 1
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);

        // An unrelated turn finishes (e.g. cancelled turn or external turn from elsewhere)
        listeners["chat-turn-finish"]?.({
            turn_id: "other-turn",
            status: "completed",
            client_request_id: "unrelated_req_123",
        });
        await new Promise(resolve => setTimeout(resolve, 20));

        // Unrelated finish MUST NOT release the lock or execute the queued gesture!
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);

        // Also ignore finish event without client_request_id while own turn is in flight
        listeners["chat-turn-finish"]?.({
            turn_id: "other-turn-no-id",
            status: "completed",
        });
        await new Promise(resolve => setTimeout(resolve, 20));
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);

        // Now the finish event for our own interaction arrives
        listeners["chat-turn-finish"]?.({
            turn_id: "turn-own",
            status: "completed",
            client_request_id: capturedRequestId,
        });
        await new Promise(resolve => setTimeout(resolve, 20));

        // Turn lock is released and the queued gesture2 is now processed!
        expect(bridge.streamChat).toHaveBeenCalledTimes(2);
    });

    it("ignores interaction-trigger-rejected for different client_request_id", async () => {
        // Rejected event with an unrelated client_request_id should be ignored
        await listeners["interaction-trigger-rejected"]?.({
            payload: { client_request_id: "other_id", reason: "busy" },
        });

        // Service should not be marked busy if not matching
        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        await service.triggerInteraction(gesture, mockController as any);
        expect(bridge.streamChat).toHaveBeenCalledTimes(1);
    });

    it("does not call streamChat when interaction-trigger is rejected during handshake", async () => {
        vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
            if (event === "interaction-trigger") {
                const reqId = (payload as any)?.client_request_id;
                listeners["interaction-trigger-rejected"]?.({
                    payload: { client_request_id: reqId, reason: "busy" },
                });
            }
        });

        const gesture: GestureEvent = {
            hitArea: "head",
            gesture: "tap",
            consecutiveTaps: 1,
        };

        const result = await service.triggerInteraction(gesture, mockController as any);
        expect(result).not.toBeNull();
        expect(bridge.streamChat).not.toHaveBeenCalled();
    });

    it("emits interaction-trigger-failed and resets busy state when handshake times out after 1000ms", async () => {
        vi.useFakeTimers();
        try {
            vi.spyOn(eventApi, "emit").mockImplementation(async () => {});

            const gesture: GestureEvent = {
                hitArea: "head",
                gesture: "tap",
                consecutiveTaps: 1,
            };

            const triggerPromise = service.triggerInteraction(gesture, mockController as any);

            await vi.advanceTimersByTimeAsync(1000);
            await triggerPromise;

            expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger-failed", expect.objectContaining({
                client_request_id: expect.stringMatching(/^interaction_\d+_[a-z0-9]+$/),
                error: "handshake_timeout",
            }));
            expect(bridge.streamChat).not.toHaveBeenCalled();

            // Next gesture should NOT be blocked by isChatBusy!
            vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
                if (event === "interaction-trigger") {
                    const reqId = (payload as any)?.client_request_id;
                    listeners["interaction-trigger-accepted"]?.({
                        payload: { client_request_id: reqId, conversation_id: "conv-mock" },
                    });
                }
            });

            // Fast forward past cooldown
            await vi.advanceTimersByTimeAsync(600);

            await service.triggerInteraction(gesture, mockController as any);
            expect(bridge.streamChat).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    it("ignores late interaction-trigger-accepted arriving after handshake timeout", async () => {
        vi.useFakeTimers();
        try {
            let capturedReqId: string | undefined;
            vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
                if (event === "interaction-trigger") {
                    capturedReqId = (payload as any)?.client_request_id;
                }
            });

            const gesture: GestureEvent = {
                hitArea: "head",
                gesture: "tap",
                consecutiveTaps: 1,
            };

            const triggerPromise = service.triggerInteraction(gesture, mockController as any);
            await vi.advanceTimersByTimeAsync(1000);
            await triggerPromise;

            expect(bridge.streamChat).not.toHaveBeenCalled();

            // Late interaction-trigger-accepted arrives
            listeners["interaction-trigger-accepted"]?.({
                payload: { client_request_id: capturedReqId, conversation_id: "conv-mock" },
            });

            expect(bridge.streamChat).not.toHaveBeenCalled();
        } finally {
            vi.useRealTimers();
        }
    });
});
