// @vitest-environment jsdom

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
            request: {
                message: "Hello from pet",
                client_request_id: emitArgs[1].client_request_id,
            },
        });
    });

    it("prevents sending another message while streaming", async () => {
        expect(hookState).not.toBeNull();

        // Simulate turn started
        await act(async () => {
            listeners["chat-turn-start"]?.({ payload: { turn_id: "turn-1" } });
        });

        expect(hookState?.isStreaming).toBe(true);

        // Attempting to send while streaming should be ignored
        await act(async () => {
            await hookState?.sendMessage("Duplicate pet message");
        });

        expect(eventApi.emit).not.toHaveBeenCalled();
    });

    it("ignores empty or whitespace-only messages", async () => {
        expect(hookState).not.toBeNull();

        await act(async () => {
            await hookState?.sendMessage("   ");
        });

        expect(eventApi.emit).not.toHaveBeenCalled();
        expect(coreApi.invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
    });
});
