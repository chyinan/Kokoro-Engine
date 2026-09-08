// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import * as bridge from "../../../lib/kokoro-bridge";
import * as eventApi from "@tauri-apps/api/event";
import {
    DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS,
    DEFAULT_BACKEND_PREPARATION_WATCHDOG_TIMEOUT_MS,
} from "./chat-turn-lifecycle";

// Event listener capture for @tauri-apps/api/event
let listeners: Record<string, (event: any) => void> = {};

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (event: string, cb: any) => {
        listeners[event] = cb;
        return () => {};
    }),
    emit: vi.fn(async () => {}),
}));

// Mock framer-motion
vi.mock("framer-motion", () => ({
    motion: {
        div: forwardRef(({ children, className, onClick, ...props }: any, ref: any) => {
            const domProps: any = {};
            for (const [key, value] of Object.entries(props)) {
                if (key.startsWith("data-") || key === "aria-hidden") {
                    domProps[key] = value;
                }
            }
            return createElement("div", { ref, className, onClick, ...domProps }, children);
        }),
        button: forwardRef(({ children, className, onClick, disabled, type, "aria-label": ariaLabel, title }: any, ref: any) => {
            return createElement("button", { ref, className, onClick, disabled, type, "aria-label": ariaLabel, title }, children);
        }),
    },
    AnimatePresence: ({ children }: any) => children,
}));

// Mock react-i18next
vi.mock("react-i18next", () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

// Mock hooks
vi.mock("../../hooks", () => ({
    VoiceState: { Idle: "idle", Listening: "listening", Processing: "processing", Speaking: "speaking", Error: "error" },
    useVoiceInput: () => ({ state: "idle", volume: 0, partialText: "", start: vi.fn(), stop: vi.fn() }),
    useWakeWord: () => ({ state: "idle", isListening: false, start: vi.fn(), stop: vi.fn() }),
    useTypingReveal: ({ onReveal }: any) => ({
        pushDelta: (delta: string) => onReveal?.(delta),
        flush: vi.fn(),
        reset: vi.fn(),
    }),
}));

// Mock services
vi.mock("../../../core/services", () => ({
    audioPlayer: { isPlaying: false },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
    const nativeSetter = Object.getOwnPropertyDescriptor(
        window.HTMLTextAreaElement.prototype,
        "value",
    )?.set;
    nativeSetter?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("ChatPanel Pet & Proactive Turn Concurrency", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;

    let turnStartCb: ((event: any) => void) | null = null;
    let turnAckCb: ((event: any) => void) | null = null;
    let turnDeltaCb: ((event: any) => void) | null = null;
    let turnFinishCb: ((event: any) => void) | null = null;
    let turnToolCb: ((event: any) => void) | null = null;

    let streamChatResolvers: Array<(res: any) => void> = [];
    let streamChatMock: ReturnType<typeof vi.fn>;
    let loadConversationMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        vi.clearAllMocks();
        listeners = {};
        turnStartCb = null;
        turnAckCb = null;
        turnDeltaCb = null;
        turnFinishCb = null;
        turnToolCb = null;
        streamChatResolvers = [];

        streamChatMock = vi.fn(() => new Promise((resolve) => {
            streamChatResolvers.push(resolve);
        }));

        loadConversationMock = vi.fn(async () => ({
            id: "conv-1",
            character_id: "char-1",
            title: "Test Conversation",
            topic: "",
            pinned_state: "{}",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            messages: [],
        }));

        vi.spyOn(bridge, "streamChat").mockImplementation(streamChatMock as any);
        vi.spyOn(bridge, "loadConversation").mockImplementation(loadConversationMock as any);
        vi.spyOn(bridge, "listConversations").mockImplementation(vi.fn(async () => []));
        vi.spyOn(bridge, "listCharacters").mockImplementation(vi.fn(async () => [{
            id: "default",
            name: "Default",
            user_nickname: "User",
            persona: "",
            source_format: "manual",
            created_at: 0,
            updated_at: 0,
        }]));
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(vi.fn(async () => ({ installed: true } as any)));
        vi.spyOn(bridge, "setVisionTextInputFocused").mockImplementation(vi.fn(async () => undefined));
        vi.spyOn(bridge, "synthesize").mockImplementation(vi.fn(async () => undefined));
        vi.spyOn(bridge, "cancelChatTurn").mockImplementation(vi.fn(async () => undefined));

        vi.spyOn(bridge, "onChatTurnAcknowledged").mockImplementation((cb: any) => {
            turnAckCb = cb;
            return Promise.resolve(() => { turnAckCb = null; });
        });
        vi.spyOn(bridge, "onChatTurnStart").mockImplementation((cb: any) => {
            turnStartCb = cb;
            return Promise.resolve(() => { turnStartCb = null; });
        });
        vi.spyOn(bridge, "onChatTurnDelta").mockImplementation((cb: any) => {
            turnDeltaCb = cb;
            return Promise.resolve(() => { turnDeltaCb = null; });
        });
        vi.spyOn(bridge, "onChatTurnFinish").mockImplementation((cb: any) => {
            turnFinishCb = cb;
            return Promise.resolve(() => { turnFinishCb = null; });
        });
        vi.spyOn(bridge, "onChatTurnTextComplete").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatError").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatWarning").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatFailure").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatTurnTranslation").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatTurnTool").mockImplementation((cb: any) => {
            turnToolCb = cb;
            return Promise.resolve(() => { turnToolCb = null; });
        });
        vi.spyOn(bridge, "onTelegramChatSync").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onVisionObservation").mockImplementation(() => Promise.resolve(() => {}));

        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
    });

    afterEach(() => {
        act(() => {
            root?.unmount();
        });
        container?.remove();
        vi.restoreAllMocks();
    });

    it("rejects pet-chat-start when ChatPanel is already busy", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        // 1. User sends message -> enters busy state
        await act(async () => {
            setTextareaValue(textarea, "Initial message");
        });
        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const originalReqId = streamChatMock.mock.calls[0][0].client_request_id;
        expect(originalReqId).toBeDefined();

        // 2. While busy, pet-chat-start event fires
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Concurrent pet message", client_request_id: "pet_conflict" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Pet message must NOT be added to messages
        const messageTexts = Array.from(container.querySelectorAll('[data-role="user"]')).map(el => el.textContent);
        expect(messageTexts.some(t => t?.includes("Concurrent pet message"))).toBe(false);
    });

    it("rejects proactive-trigger when ChatPanel is already busy", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        // 1. User sends message -> enters busy state
        await act(async () => {
            setTextareaValue(textarea, "User message");
        });
        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);

        // 2. Proactive trigger fires while busy
        await act(async () => {
            listeners["proactive-trigger"]?.({
                payload: { instruction: "Spontaneous proactive message" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // streamChat should NOT have been called for proactive trigger
        expect(streamChatMock).toHaveBeenCalledTimes(1);
    });

    it("does not allow a late-resolving old request to terminate an active pet turn", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        // 1. First request starts
        await act(async () => {
            setTextareaValue(textarea, "Request 1");
        });
        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });
        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const req1Id = streamChatMock.mock.calls[0][0].client_request_id;
        const req1Resolver = streamChatResolvers[0];

        // 1b. Request 1 turn starts and finishes normally via events
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-req1",
                client_request_id: req1Id,
                conversation_id: "conv-1",
                user_message_id: 101,
            });
            turnFinishCb?.({
                turn_id: "turn-req1",
                status: "completed",
                conversation_id: "conv-1",
                client_request_id: req1Id,
                assistant_message_id: 102,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Note: req1 streamChat Promise is deliberately kept unresolved at this point to simulate late promise resolution!

        // 2. Pet chat starts with its own client_request_id
        const petReqId = "pet_test_12345";
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Hello from Pet", client_request_id: petReqId }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Pet turn start event arrives
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-pet",
                client_request_id: petReqId,
                conversation_id: "conv-1",
                user_message_id: 103,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Pet streaming delta arrives
        await act(async () => {
            turnDeltaCb?.({
                turn_id: "turn-pet",
                delta: "Pet reply chunk",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 3. NOW, the old Request 1's streamChat Promise finally resolves!
        await act(async () => {
            req1Resolver({
                conversation_id: "conv-1",
                user_message_id: 101,
                assistant_message_id: 102,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 4. VERIFY: The pet turn is NOT killed!
        // Textarea should remain disabled because Pet turn is still streaming/busy!
        expect(textarea.disabled).toBe(true);

        // Pet streaming continues with another delta
        await act(async () => {
            turnDeltaCb?.({
                turn_id: "turn-pet",
                delta: " more pet content",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Pet turn finishes
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn-pet",
                status: "completed",
                conversation_id: "conv-1",
                client_request_id: petReqId,
                assistant_message_id: 104,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Now textarea should be re-enabled after pet turn legitimately finishes
        expect(textarea.disabled).toBe(false);
    });

    it("proactive-trigger sets clientRequestId and passes it to streamChat", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Trigger proactive while idle
        await act(async () => {
            listeners["proactive-trigger"]?.({
                payload: { instruction: "Tell a short story" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const callArgs = streamChatMock.mock.calls[0][0];
        expect(callArgs.message).toBe("Tell a short story");
        expect(callArgs.hidden).toBe(true);
        expect(callArgs.client_request_id).toMatch(/^proactive_\d+_[a-z0-9]+$/);
    });

    it("rejects proactive-trigger when audio player is playing", async () => {
        const { audioPlayer } = await import("../../../core/services");
        (audioPlayer as any).isPlaying = true;

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            listeners["proactive-trigger"]?.({
                payload: { instruction: "Should not speak while audio is playing" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).not.toHaveBeenCalled();
        (audioPlayer as any).isPlaying = false;
    });

    it("interaction-trigger tracks clientRequestId and sets busy state", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(false);

        await act(async () => {
            listeners["interaction-trigger"]?.({
                payload: {
                    gesture: "tap",
                    hitArea: "head",
                    client_request_id: "interaction_test_123",
                }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // After interaction trigger, ChatPanel is in streaming/busy state
        expect(textarea.disabled).toBe(true);
        expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger-accepted", expect.objectContaining({
            client_request_id: "interaction_test_123",
        }));
    });

    it("emits pet-chat-rejected when ChatPanel is busy", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Trigger a first turn to make ChatPanel busy
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "First message", client_request_id: "pet_first_1" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(eventApi.emit).toHaveBeenCalledWith("pet-chat-accepted", expect.objectContaining({
            client_request_id: "pet_first_1",
        }));

        // Now ChatPanel is busy. Send a second pet-chat-start
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Second message", client_request_id: "pet_second_2" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(eventApi.emit).toHaveBeenCalledWith("pet-chat-rejected", {
            client_request_id: "pet_second_2",
            reason: "busy",
        });
    });

    it("rolls back optimistic user message when pet-chat-failed is received", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Pet chat starts optimistically
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Optimistic message", client_request_id: "pet_opt_123" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Message should be visible
        expect(container.textContent).toContain("Optimistic message");

        // Backend stream_chat failed, emitting pet-chat-failed
        await act(async () => {
            listeners["pet-chat-failed"]?.({
                payload: { client_request_id: "pet_opt_123", error: "chat_turn_busy" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Optimistic message should be rolled back and removed
        expect(container.textContent).not.toContain("Optimistic message");
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(false);
    });

    it("emits interaction-trigger-rejected when ChatPanel is busy", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Make ChatPanel busy
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Turn 1", client_request_id: "pet_busy_1" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Trigger interaction while busy
        await act(async () => {
            listeners["interaction-trigger"]?.({
                payload: {
                    gesture: "tap",
                    hitArea: "head",
                    client_request_id: "interaction_busy_2",
                }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger-rejected", {
            client_request_id: "interaction_busy_2",
            reason: "busy",
        });
    });

    it("clears busy state when interaction-trigger-failed is received", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            listeners["interaction-trigger"]?.({
                payload: {
                    gesture: "tap",
                    hitArea: "head",
                    client_request_id: "interaction_to_fail",
                }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(true);

        await act(async () => {
            listeners["interaction-trigger-failed"]?.({
                payload: { client_request_id: "interaction_to_fail", error: "failed" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(textarea.disabled).toBe(false);
    });

    it("rolls back optimistic message on handshake_timeout without showing error toast", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Timeout message", client_request_id: "pet_timeout_123" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(container.textContent).toContain("Timeout message");

        await act(async () => {
            listeners["pet-chat-failed"]?.({
                payload: { client_request_id: "pet_timeout_123", error: "handshake_timeout" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(container.textContent).not.toContain("Timeout message");
        expect(container.textContent).not.toContain("handshake_timeout");
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(false);
    });

    it("resets busy state via external pending watchdog if sender fails to reverse-notify", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Abandoned message", client_request_id: "pet_abandoned_456" }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Abandoned message");
            let textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(true);

            // Fast-forward 2500ms: should NOT expire prematurely (as old 2.5s bug did)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(2500);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });
            expect(container.textContent).toContain("Abandoned message");
            textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(true);

            // Fast-forward to the full watchdog timeout
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS - 2500);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Watchdog should have cleaned up the abandoned turn
            expect(container.textContent).not.toContain("Abandoned message");
            textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(
                "pet_abandoned_456",
                "external_pending_turn_watchdog_timeout",
            );
        } finally {
            vi.useRealTimers();
        }
    });

    it("does not kill legitimate turn when backend preparation takes >2.5s (e.g. 3500ms)", async () => {
        vi.useFakeTimers();
        try {
            const cancelChatTurnSpy = vi.spyOn(bridge, "cancelChatTurn").mockResolvedValue(undefined as any);

            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_slow_backend_789";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Slow backend message", client_request_id: clientRequestId }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Fast-forward 3500ms (backend memory search / hooks in progress)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(3500);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Pending request is still held and message is still in DOM
            expect(container.textContent).toContain("Slow backend message");

            // Backend completes preparation and emits chat-turn-start
            await act(async () => {
                turnStartCb?.({
                    turn_id: "turn_slow_1",
                    client_request_id: clientRequestId,
                    conversation_id: "conv_1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // It must NOT be cancelled as a stale turn!
            expect(cancelChatTurnSpy).not.toHaveBeenCalledWith("turn_slow_1", expect.stringContaining("stale_turn"));

            // Fast forward past the original 15s watchdog duration:
            // Since the watchdog was cleared on valid turn start, it should NOT fire!
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Slow backend message");
        } finally {
            vi.useRealTimers();
        }
    });

    it("unrelated pet-chat-failed does not clear watchdog of another pending request", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_active_req_111";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Active pending message", client_request_id: clientRequestId }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // An unrelated pet-chat-failed arrives for a different request
            await act(async () => {
                listeners["pet-chat-failed"]?.({
                    payload: { client_request_id: "unrelated_other_req", error: "some_error" }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // The pending message should still be there
            expect(container.textContent).toContain("Active pending message");

            // When full timeout expires, the watchdog for clientRequestId should still properly fire!
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Watchdog properly cleaned up the abandoned turn
            expect(container.textContent).not.toContain("Active pending message");
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("unrelated chat-turn-start does not clear watchdog of another pending request", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_active_req_222";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Active pending message 2", client_request_id: clientRequestId }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // An unrelated chat-turn-start arrives (e.g. from an external authorized turn like mod or onboarding)
            await act(async () => {
                turnStartCb?.({
                    turn_id: "turn_mod_1",
                    client_request_id: "mod_unrelated_turn",
                    conversation_id: "conv_1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // The pending message should still be there
            expect(container.textContent).toContain("Active pending message 2");

            // When full timeout expires, the watchdog for clientRequestId should still properly fire!
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Watchdog properly cleaned up the abandoned turn
            expect(container.textContent).not.toContain("Active pending message 2");
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("unrelated interaction-trigger-failed does not clear watchdog of another pending request", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "interaction_active_req_333";
            await act(async () => {
                listeners["interaction-trigger"]?.({
                    payload: {
                        gesture: "pat",
                        hitArea: "body",
                        client_request_id: clientRequestId,
                    }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // An unrelated interaction-trigger-failed arrives
            await act(async () => {
                listeners["interaction-trigger-failed"]?.({
                    payload: { client_request_id: "unrelated_other_interaction", error: "busy" }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Input is still disabled (holding busy lock for pending interaction)
            let textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(true);

            // Fast forward to full watchdog timeout
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Watchdog properly cleaned up and released lock
            textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(
                clientRequestId,
                "external_pending_interaction_watchdog_timeout",
            );
        } finally {
            vi.useRealTimers();
        }
    });

    it("interaction-trigger does not kill legitimate turn when backend preparation takes >2.5s (e.g. 3500ms)", async () => {
        vi.useFakeTimers();
        try {
            const cancelChatTurnSpy = vi.spyOn(bridge, "cancelChatTurn").mockResolvedValue(undefined as any);

            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "interaction_slow_backend_444";
            await act(async () => {
                listeners["interaction-trigger"]?.({
                    payload: {
                        gesture: "poke",
                        hitArea: "head",
                        client_request_id: clientRequestId,
                    }
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Fast-forward 3500ms
            await act(async () => {
                await vi.advanceTimersByTimeAsync(3500);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Backend completes preparation and emits chat-turn-start
            await act(async () => {
                turnStartCb?.({
                    turn_id: "turn_interaction_slow_1",
                    client_request_id: clientRequestId,
                    conversation_id: "conv_1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // It must NOT be cancelled as a stale turn!
            expect(cancelChatTurnSpy).not.toHaveBeenCalledWith("turn_interaction_slow_1", expect.stringContaining("stale_turn"));

            // Fast forward past original 15s watchdog duration: should NOT fire
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            let textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(true); // Still in active turn
        } finally {
            vi.useRealTimers();
        }
    });

    it("recovers interactability and allows subsequent chat after stopping a turn stuck in tool/MCP execution", async () => {
        const cancelChatTurnSpy = vi.spyOn(bridge, "cancelChatTurn").mockResolvedValue(undefined as any);

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 1. Send user message
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        await act(async () => {
            const inputSetter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")?.set;
            inputSetter?.call(textarea, "Call MCP tool");
            textarea.dispatchEvent(new Event("input", { bubbles: true }));
        });

        const form = container.querySelector("form");
        await act(async () => {
            form?.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 2. Turn starts with the clientRequestId that was passed to streamChat
        const lastCall = streamChatMock.mock.calls[streamChatMock.mock.calls.length - 1][0];
        const clientRequestId = lastCall.client_request_id;

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn_tool_hang_1",
                client_request_id: clientRequestId,
                conversation_id: "conv_1",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 3. Tool execution starts and gets stuck
        await act(async () => {
            turnToolCb?.({
                turn_id: "turn_tool_hang_1",
                tool: {
                    id: "call_mcp_1",
                    name: "mcp_query",
                    status: "calling",
                }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Textarea is disabled during tool execution
        expect(textarea.disabled).toBe(true);

        // 4. User clicks Stop button
        const stopBtn = container.querySelector('button[aria-label="chat.actions.stop"]');
        expect(stopBtn).not.toBeNull();
        await act(async () => {
            stopBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(cancelChatTurnSpy).toHaveBeenCalledWith("turn_tool_hang_1", "stopped_from_chat_panel");

        // 5. Backend returns cancelled finish event
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn_tool_hang_1",
                status: "cancelled",
                conversation_id: "conv_1",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 6. UI is unlocked and user can send a new message
        expect(textarea.disabled).toBe(false);

        // Send next message
        await act(async () => {
            const inputSetter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")?.set;
            inputSetter?.call(textarea, "Second message after stop");
            textarea.dispatchEvent(new Event("input", { bubbles: true }));
        });

        await act(async () => {
            form?.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(container.textContent).toContain("Second message after stop");
    });

    it("rejects pet-chat-start if pet-chat-failed arrived earlier out-of-order (tombstone)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Failed arrives BEFORE start (out-of-order)
        await act(async () => {
            listeners["pet-chat-failed"]?.({
                payload: { client_request_id: "pet_out_of_order", error: "handshake_timeout" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Now start arrives
        await act(async () => {
            listeners["pet-chat-start"]?.({
                payload: { message: "Out of order message", client_request_id: "pet_out_of_order" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Should be rejected with cancelled, not accepted
        expect(eventApi.emit).toHaveBeenCalledWith("pet-chat-rejected", {
            client_request_id: "pet_out_of_order",
            reason: "cancelled",
        });
        expect(container.textContent).not.toContain("Out of order message");
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(false);
    });

    it("rejects interaction-trigger if interaction-trigger-failed arrived earlier out-of-order (tombstone)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Failed arrives BEFORE trigger
        await act(async () => {
            listeners["interaction-trigger-failed"]?.({
                payload: { client_request_id: "interaction_out_of_order", error: "handshake_timeout" }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Trigger arrives
        await act(async () => {
            listeners["interaction-trigger"]?.({
                payload: {
                    gesture: "tap",
                    hitArea: "head",
                    client_request_id: "interaction_out_of_order",
                }
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(eventApi.emit).toHaveBeenCalledWith("interaction-trigger-rejected", {
            client_request_id: "interaction_out_of_order",
            reason: "cancelled",
        });
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.disabled).toBe(false);
    });

    it("cancels backend turn on watchdog timeout when chat-turn-start is lost, allowing subsequent chat submission", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_abandoned_turn_999";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Abandoned message before send", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            let textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(true);

            // Advance time to full watchdog timeout
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Watchdog cancelled backend turn
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(
                clientRequestId,
                "external_pending_turn_watchdog_timeout",
            );
            expect(container.textContent).not.toContain("Abandoned message before send");
            textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);

            // Subsequent user message can be sent normally
            await act(async () => {
                setTextareaValue(textarea, "Hello after watchdog recovery");
            });

            const form = container.querySelector("form");
            await act(async () => {
                form?.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(streamChatMock).toHaveBeenCalledWith(expect.objectContaining({
                message: "Hello after watchdog recovery",
            }));
            expect(container.textContent).toContain("Hello after watchdog recovery");
        } finally {
            vi.useRealTimers();
        }
    });

    it("chat-turn-acknowledged transitions watchdog to preparation budget, allowing slow backend initialization (>15s) without being abandoned", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_slow_init_turn_101";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Heavy model loading message", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Heavy model loading message");

            // Backend acquires turn lock and emits chat-turn-acknowledged
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_101",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Fast-forward past the default 15s watchdog duration (e.g. 25 seconds)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(25_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Crucial assertion: message is STILL present, NOT killed by watchdog!
            expect(container.textContent).toContain("Heavy model loading message");
            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );

            // Now backend finishes slow preparation (e.g. at 25s) and emits chat-turn-start
            await act(async () => {
                turnStartCb?.({
                    turn_id: "backend_turn_101",
                    client_request_id: clientRequestId,
                    conversation_id: "conv-1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Turn starts normally, deltas stream
            await act(async () => {
                turnDeltaCb?.({
                    turn_id: "backend_turn_101",
                    delta: "Hello from slow model!",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Hello from slow model!");

            // Complete turn
            await act(async () => {
                turnFinishCb?.({
                    turn_id: "backend_turn_101",
                    status: "completed",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("unrelated chat-turn-acknowledged does not clear watchdog of another pending request", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_victim_req_202";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Victim pending message", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // An unrelated chat-turn-acknowledged arrives
            await act(async () => {
                turnAckCb?.({
                    turn_id: "other_turn_999",
                    client_request_id: "completely_different_req",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Victim pending message");

            // Advance past 15s watchdog
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_EXTERNAL_PENDING_WATCHDOG_TIMEOUT_MS);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // The abandoned victim request WAS cleared because the ack did not match!
            expect(container.textContent).not.toContain("Victim pending message");
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(
                clientRequestId,
                "external_pending_turn_watchdog_timeout",
            );
        } finally {
            vi.useRealTimers();
        }
    });

    it("interaction-trigger watchdog is cleared by chat-turn-acknowledged for slow interaction turn", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "interaction_slow_touch_303";
            await act(async () => {
                listeners["interaction-trigger"]?.({
                    payload: {
                        interaction_type: "touch_head",
                        client_request_id: clientRequestId,
                    },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Backend acknowledges
            await act(async () => {
                turnAckCb?.({
                    turn_id: "touch_turn_303",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Fast-forward 20 seconds
            await act(async () => {
                await vi.advanceTimersByTimeAsync(20_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Should NOT have triggered interaction watchdog
            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );

            // Backend starts turn
            await act(async () => {
                turnStartCb?.({
                    turn_id: "touch_turn_303",
                    client_request_id: clientRequestId,
                    conversation_id: "conv-1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Finish turn
            await act(async () => {
                turnFinishCb?.({
                    turn_id: "touch_turn_303",
                    status: "completed",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("stop generation during slow turn after ack cleanly cancels and recovers UI", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_slow_cancelled_404";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Cancelled while loading", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Backend acknowledged
            await act(async () => {
                turnAckCb?.({
                    turn_id: "turn_cancel_404",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Advance 10s (still loading)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(10_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Stop button clicked while still waiting for turn start
            const stopButton = container.querySelector('button[aria-label="chat.actions.stop"]') as HTMLButtonElement;
            expect(stopButton).toBeTruthy();
            await act(async () => {
                stopButton.click();
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(clientRequestId, "stopped_from_chat_panel");

            // Advance past cancellation watchdog (5s) to ensure UI recovers
            await act(async () => {
                await vi.advanceTimersByTimeAsync(5000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("chat-turn-acknowledged transitions to preparation watchdog, which cancels turn if preparation exceeds budget (60s)", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_stuck_prep_505";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Stuck in prompt preparation", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Stuck in prompt preparation");

            // Backend acknowledges turn registration
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_505",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Advance 25s (beyond external pending 15s budget, but within 60s preparation budget)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(25_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Turn is still waiting for preparation, not cancelled yet
            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );

            // Now advance to 60s total from ACK (remaining preparation budget)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(DEFAULT_BACKEND_PREPARATION_WATCHDOG_TIMEOUT_MS - 25_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Preparation watchdog MUST have fired!
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith(
                clientRequestId,
                "external_turn_preparation_watchdog_timeout",
            );

            // Message is cleaned up and UI input recovered
            expect(container.textContent).not.toContain("Stuck in prompt preparation");
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("late chat-turn-acknowledged after chat-turn-start does not re-arm a watchdog", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_late_ack_606";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Fast turn message", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // chat-turn-start arrives before ack
            await act(async () => {
                turnStartCb?.({
                    turn_id: "backend_turn_606",
                    client_request_id: clientRequestId,
                    conversation_id: "conv-1",
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Now late ack arrives
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_606",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Advance 65s (past the 60s preparation budget)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(65_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // cancelChatTurn must NOT have been called for watchdog
            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );

            // Complete turn normally
            await act(async () => {
                turnFinishCb?.({
                    turn_id: "backend_turn_606",
                    status: "completed",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("chat-turn-acknowledged followed by lost chat-turn-start safely recovers when chat-turn-finish arrives", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_lost_start_701";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Lost start message", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Lost start message");

            // Backend acknowledges turn registration
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_lost_start_701",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // chat-turn-start is LOST (never sent/received)

            // Backend finishes and emits chat-turn-finish
            await act(async () => {
                turnFinishCb?.({
                    turn_id: "backend_turn_lost_start_701",
                    status: "completed",
                    client_request_id: clientRequestId,
                    conversation_id: "conv-1",
                    assistant_message_id: 555,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // UI must have recovered (not stuck in busy)
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);

            // Because start was lost, needsResync was true and loadConversation must have been invoked to sync DB
            expect(loadConversationMock).toHaveBeenCalledWith("conv-1");

            // Fast-forward 65s (past the 60s preparation budget)
            await act(async () => {
                await vi.advanceTimersByTimeAsync(65_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Preparation watchdog must have been cleared on finish and must NOT fire
            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );
        } finally {
            vi.useRealTimers();
        }
    });

    it("both acknowledged and start lost safely recovers when chat-turn-finish arrives with client_request_id", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_both_lost_702";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Both ack and start lost", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(container.textContent).toContain("Both ack and start lost");

            // Neither ack nor start arrives.
            // Backend completes and emits chat-turn-finish with client_request_id
            await act(async () => {
                turnFinishCb?.({
                    turn_id: "backend_turn_both_lost_702",
                    status: "completed",
                    client_request_id: clientRequestId,
                    conversation_id: "conv-1",
                    assistant_message_id: 777,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // UI recovers via pendingRequest fallback validation
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);

            // Resync triggered
            expect(loadConversationMock).toHaveBeenCalledWith("conv-1");

            // Watchdog must be cleared
            await act(async () => {
                await vi.advanceTimersByTimeAsync(65_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                clientRequestId,
                expect.stringContaining("watchdog"),
            );
        } finally {
            vi.useRealTimers();
        }
    });

    it("chat-turn-acknowledged arriving after cancel immediately cancels backend turn_id", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            const clientRequestId = "pet_pre_cancel_703";
            await act(async () => {
                listeners["pet-chat-start"]?.({
                    payload: { message: "Cancel immediately", client_request_id: clientRequestId },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // User cancels immediately
            const stopButton = container.querySelector('button[aria-label="chat.actions.stop"]') as HTMLButtonElement;
            expect(stopButton).toBeTruthy();
            await act(async () => {
                stopButton.click();
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Delayed ACK arrives with turn_id
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_cancelled_703",
                    client_request_id: clientRequestId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // cancelChatTurn must be called with backend turn_id
            expect(bridge.cancelChatTurn).toHaveBeenCalledWith("backend_turn_cancelled_703", "cancelled_by_user");
        } finally {
            vi.useRealTimers();
        }
    });

    it("chat-turn-acknowledged followed by lost start and finish safely recovers when streamChat resolves", async () => {
        vi.useFakeTimers();
        try {
            await act(async () => {
                root.render(createElement(ChatPanel));
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Trigger proactive action which calls streamChat
            await act(async () => {
                listeners["proactive-trigger"]?.({
                    payload: { instruction: "Stream recovery instruction" },
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(streamChatMock).toHaveBeenCalled();
            const calledReqId = streamChatMock.mock.calls[0][0].client_request_id;
            expect(calledReqId).toBeTruthy();

            // ACK arrives
            await act(async () => {
                turnAckCb?.({
                    turn_id: "backend_turn_stream_704",
                    client_request_id: calledReqId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // Both start and finish are lost

            // streamChat resolves
            await act(async () => {
                const resolver = streamChatResolvers[0];
                resolver?.({
                    conversation_id: "conv-1",
                    assistant_message_id: 999,
                    status: "completed",
                    client_request_id: calledReqId,
                });
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            // UI recovers
            const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
            expect(textarea.disabled).toBe(false);

            // DB resync triggered because assistant bubble was missing
            expect(loadConversationMock).toHaveBeenCalledWith("conv-1");

            // Advance 65s -> watchdog does not fire
            await act(async () => {
                await vi.advanceTimersByTimeAsync(65_000);
                for (let i = 0; i < 5; i++) await Promise.resolve();
            });

            expect(bridge.cancelChatTurn).not.toHaveBeenCalledWith(
                calledReqId,
                expect.stringContaining("watchdog"),
            );
        } finally {
            vi.useRealTimers();
        }
    });
});

