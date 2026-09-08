// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { usePetChat, type PetChatState } from "./usePetChat";
import * as eventApi from "@tauri-apps/api/event";
import * as coreApi from "@tauri-apps/api/core";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
    emit: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(async () => {}),
}));

describe("usePetChat", () => {
    let container: HTMLDivElement | null = null;
    let root: Root | null = null;
    let hookState: PetChatState | null = null;
    let listeners: Record<string, (event: any) => void> = {};

    function TestHarness() {
        const hook = usePetChat();
        hookState = hook;
        return null;
    }

    beforeEach(async () => {
        vi.clearAllMocks();
        listeners = {};
        hookState = null;
        vi.spyOn(eventApi, "listen").mockImplementation((event: string, cb: any) => {
            listeners[event] = cb;
            return Promise.resolve(() => {});
        });
        vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
            if (event === "pet-chat-start") {
                const reqId = payload?.client_request_id;
                listeners["pet-chat-accepted"]?.({
                    payload: { client_request_id: reqId, conversation_id: "conv-mock" },
                });
            }
        });
        vi.spyOn(coreApi, "invoke").mockImplementation(async () => {});

        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);

        await act(async () => {
            root?.render(createElement(TestHarness));
        });
    });

    afterEach(() => {
        if (root && container) {
            act(() => {
                root?.unmount();
            });
            container.remove();
        }
        root = null;
        container = null;
        hookState = null;
    });

    it("generates client_request_id and includes it in pet-chat-start and stream_chat", async () => {
        expect(hookState).not.toBeNull();

        await act(async () => {
            await hookState?.sendMessage("Hello from pet");
        });

        expect(eventApi.emit).toHaveBeenCalledTimes(1);
        const emitArgs = (eventApi.emit as any).mock.calls[0];
        expect(emitArgs[0]).toBe("pet-chat-start");
        expect(emitArgs[1].message).toBe("Hello from pet");
        expect(emitArgs[1].client_request_id).toMatch(/^pet_\d+_[a-z0-9]+$/);

        expect(coreApi.invoke).toHaveBeenCalledWith("stream_chat", {
            request: expect.objectContaining({
                message: "Hello from pet",
                client_request_id: emitArgs[1].client_request_id,
            }),
        });
    });

    it("prevents sending another message while streaming", async () => {
        expect(hookState).not.toBeNull();

        await act(async () => {
            await hookState?.sendMessage("First pet message");
        });

        expect(hookState?.isStreaming).toBe(true);

        // Attempting to send while streaming should be ignored
        await act(async () => {
            await hookState?.sendMessage("Duplicate pet message");
        });

        expect(eventApi.emit).toHaveBeenCalledTimes(1);
    });

    it("ignores chat turn lifecycle events from other client requests", async () => {
        await act(async () => {
            await hookState?.sendMessage("Own pet request");
        });

        const startCall = (eventApi.emit as any).mock.calls.find((call: any[]) => call[0] === "pet-chat-start");
        const ownClientRequestId = startCall[1].client_request_id;

        await act(async () => {
            listeners["chat-turn-start"]?.({
                payload: { turn_id: "foreign-turn", client_request_id: "foreign-request" },
            });
            listeners["chat-turn-finish"]?.({
                payload: {
                    turn_id: "foreign-turn",
                    client_request_id: "foreign-request",
                    status: "completed",
                },
            });
        });

        expect(hookState?.isStreaming).toBe(true);

        await act(async () => {
            listeners["chat-turn-start"]?.({
                payload: { turn_id: "own-turn", client_request_id: ownClientRequestId },
            });
            listeners["chat-turn-finish"]?.({
                payload: {
                    turn_id: "own-turn",
                    status: "completed",
                },
            });
        });

        expect(hookState?.isStreaming).toBe(false);
    });

    it("ignores chat errors from other turns while the own turn is active", async () => {
        await act(async () => {
            await hookState?.sendMessage("Own pet request with error");
        });

        const startCall = (eventApi.emit as any).mock.calls.find((call: any[]) => call[0] === "pet-chat-start");
        const ownClientRequestId = startCall[1].client_request_id;
        await act(async () => {
            listeners["chat-turn-start"]?.({
                payload: { turn_id: "own-turn", client_request_id: ownClientRequestId },
            });
            listeners["chat-error"]?.({
                payload: { trace_id: "foreign-turn", message: "foreign error" },
            });
        });

        expect(hookState?.isStreaming).toBe(true);

        await act(async () => {
            listeners["chat-error"]?.({
                payload: {
                    trace_id: "own-turn",
                    message: "own error",
                },
            });
        });

        expect(hookState?.isStreaming).toBe(false);
    });

    it("ignores empty or whitespace-only messages", async () => {
        expect(hookState).not.toBeNull();

        await act(async () => {
            await hookState?.sendMessage("   ");
        });

        expect(eventApi.emit).not.toHaveBeenCalled();
        expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
    });

    it("aborts sendMessage if is_chat_busy returns true", async () => {
        expect(hookState).not.toBeNull();
        vi.spyOn(coreApi, "invoke").mockImplementation(async (cmd: string) => {
            if (cmd === "is_chat_busy") return true;
            return undefined;
        });

        await act(async () => {
            await hookState?.sendMessage("Hello when busy");
        });

        expect(eventApi.emit).not.toHaveBeenCalled();
        expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
    });

    it("emits pet-chat-failed and hides bubble if stream_chat fails", async () => {
        expect(hookState).not.toBeNull();
        vi.spyOn(coreApi, "invoke").mockImplementation(async (cmd: string) => {
            if (cmd === "stream_chat") throw new Error("chat_turn_busy: Chat turn is currently busy");
            return undefined;
        });

        await act(async () => {
            await hookState?.sendMessage("Hello that will fail");
        });

        expect(eventApi.emit).toHaveBeenCalledWith("pet-chat-failed", expect.objectContaining({
            client_request_id: expect.stringMatching(/^pet_\d+_[a-z0-9]+$/),
            error: "chat_turn_busy: Chat turn is currently busy",
        }));
        expect(coreApi.invoke).toHaveBeenCalledWith("hide_bubble_window");
        expect(hookState?.isStreaming).toBe(false);
    });

    it("does not call stream_chat and resets streaming when pet-chat-rejected is received during handshake", async () => {
        expect(hookState).not.toBeNull();

        vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
            if (event === "pet-chat-start") {
                listeners["pet-chat-rejected"]?.({
                    payload: { client_request_id: payload?.client_request_id, reason: "busy" },
                });
            }
        });

        await act(async () => {
            await hookState?.sendMessage("Hello pet rejected");
        });

        expect(hookState?.isStreaming).toBe(false);
        expect(coreApi.invoke).toHaveBeenCalledWith("hide_bubble_window");
        expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
    });

    it("cancels active turn on late pet-chat-rejected", async () => {
        expect(hookState).not.toBeNull();

        // Start a turn with default accepted mock
        await act(async () => {
            await hookState?.sendMessage("Hello pet");
        });
        expect(hookState?.isStreaming).toBe(true);

        const emitArgs = (eventApi.emit as any).mock.calls.find((call: any[]) => call[0] === "pet-chat-start");
        const clientRequestId = emitArgs[1].client_request_id;

        // Simulate late rejection from ChatPanel
        await act(async () => {
            listeners["pet-chat-rejected"]?.({ payload: { client_request_id: clientRequestId, reason: "busy" } });
        });

        expect(hookState?.isStreaming).toBe(false);
        expect(coreApi.invoke).toHaveBeenCalledWith("cancel_chat_turn", {
            turnId: clientRequestId,
            reason: "pet_chat_rejected",
        });
        expect(coreApi.invoke).toHaveBeenCalledWith("hide_bubble_window");
    });

    it("emits pet-chat-failed and resets state when handshake times out after 1000ms", async () => {
        vi.useFakeTimers();
        try {
            vi.spyOn(eventApi, "emit").mockImplementation(async () => {});

            let sendPromise: Promise<void> | null = null;
            await act(async () => {
                sendPromise = hookState!.sendMessage("Pet chat timeout message");
            });

            await act(async () => {
                await vi.advanceTimersByTimeAsync(1000);
            });

            await act(async () => {
                await sendPromise!;
            });

            expect(eventApi.emit).toHaveBeenCalledWith("pet-chat-failed", expect.objectContaining({
                client_request_id: expect.stringMatching(/^pet_\d+_[a-z0-9]+$/),
                error: "handshake_timeout",
            }));
            expect(hookState?.isStreaming).toBe(false);
            expect(coreApi.invoke).toHaveBeenCalledWith("hide_bubble_window");
            expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
        } finally {
            vi.useRealTimers();
        }
    });

    it("ignores late pet-chat-accepted arriving after handshake timeout", async () => {
        vi.useFakeTimers();
        try {
            let capturedReqId: string | undefined;
            vi.spyOn(eventApi, "emit").mockImplementation(async (event: string, payload: any) => {
                if (event === "pet-chat-start") {
                    capturedReqId = payload?.client_request_id;
                }
            });

            let sendPromise: Promise<void>;
            act(() => {
                sendPromise = hookState!.sendMessage("Pet late acceptance");
            });

            await act(async () => {
                await vi.advanceTimersByTimeAsync(1000);
            });
            await act(async () => {
                await sendPromise!;
            });

            expect(hookState?.isStreaming).toBe(false);

            await act(async () => {
                listeners["pet-chat-accepted"]?.({
                    payload: { client_request_id: capturedReqId, conversation_id: "conv-mock" },
                });
            });

            expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
            expect(hookState?.isStreaming).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });
});
