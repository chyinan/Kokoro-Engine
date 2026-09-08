// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import * as bridge from "../../../lib/kokoro-bridge";

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

// Mock @tauri-apps/api/event
vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
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
    textarea.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("ChatPanel - dropped chat-turn-finish handling", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;

    // Event callback captures
    let turnStartCb: ((event: any) => void) | null = null;
    let turnDeltaCb: ((event: any) => void) | null = null;
    let turnFinishCb: ((event: any) => void) | null = null;
    let turnAcknowledgedCb: ((event: any) => void) | null = null;

    let streamChatResolver: ((res: any) => void) | null = null;
    let streamChatRejecter: ((err: any) => void) | null = null;
    let streamChatMock: ReturnType<typeof vi.fn>;
    let loadConversationMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        vi.clearAllMocks();
        turnStartCb = null;
        turnDeltaCb = null;
        turnFinishCb = null;
        turnAcknowledgedCb = null;
        streamChatResolver = null;
        streamChatRejecter = null;

        streamChatMock = vi.fn(() => new Promise((resolve, reject) => {
            streamChatResolver = resolve;
            streamChatRejecter = reject;
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
        vi.spyOn(bridge, "clearHistory").mockImplementation(vi.fn(async () => undefined));

        vi.spyOn(bridge, "onChatTurnAcknowledged").mockImplementation((cb: any) => {
            turnAcknowledgedCb = cb;
            return Promise.resolve(() => { turnAcknowledgedCb = null; });
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
        vi.spyOn(bridge, "onChatTurnTool").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onTelegramChatSync").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onVisionObservation").mockImplementation(() => Promise.resolve(() => {}));

        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
    });

    afterEach(() => {
        act(() => {
            root.unmount();
        });
        container.remove();
        vi.restoreAllMocks();
    });

    it("clears busy state and finalizes turn when chat-turn-finish event is dropped", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Find textarea and form
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea).not.toBeNull();
        expect(textarea.disabled).toBe(false);

        const form = container.querySelector("form") as HTMLFormElement;
        expect(form).not.toBeNull();

        // 1. User types message and submits
        await act(async () => {
            setTextareaValue(textarea, "Hello Kokoro");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Verify streamChat was called
        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const requestPayload = streamChatMock.mock.calls[0][0];
        const clientRequestId = requestPayload.client_request_id;
        expect(clientRequestId).toBeDefined();

        // UI is now busy
        expect(textarea.disabled).toBe(true);

        // 2. onChatTurnStart arrives
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-test-1",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                user_message_id: 101,
            });
        });

        // 3. onChatTurnDelta arrives with assistant text
        await act(async () => {
            turnDeltaCb?.({
                turn_id: "turn-test-1",
                delta: "Hello, nice to meet you!",
            });
        });

        // Verify assistant bubble has text
        expect(container.textContent).toContain("Hello, nice to meet you!");

        // 4. NOTE: chat-turn-finish event is NOT fired! (Simulating event loss / listener missing)
        // Instead, streamChatPromise directly resolves!
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 101,
                assistant_message_id: 102,
                client_request_id: clientRequestId,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 5. Verify UI is NO LONGER BUSY!
        expect(textarea.disabled).toBe(false);
        const sendBtn = container.querySelector('button[aria-label="Send message"]') as HTMLButtonElement;
        expect(sendBtn).not.toBeNull();

        // Assistant message still intact
        expect(container.textContent).toContain("Hello, nice to meet you!");

        // 6. Verify user can immediately send a second message without getting blocked
        await act(async () => {
            setTextareaValue(textarea, "How is the weather?");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Second streamChat must have been triggered!
        expect(streamChatMock).toHaveBeenCalledTimes(2);
        expect(streamChatMock.mock.calls[1][0].message).toBe("How is the weather?");
    });

    it("preserves visible messages and reports an error when clearing history fails", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        await act(async () => {
            setTextareaValue(textarea, "Keep this message");
        });
        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);

        const clientRequestId = streamChatMock.mock.calls[0][0].client_request_id;
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-clear-test",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                user_message_id: 101,
            });
        });
        await act(async () => {
            turnDeltaCb?.({
                turn_id: "turn-clear-test",
                delta: "Keep this reply",
            });
        });
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 101,
                assistant_message_id: 102,
                client_request_id: clientRequestId,
                status: "completed",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(container.textContent).toContain("Keep this reply");
        vi.mocked(bridge.clearHistory).mockRejectedValueOnce(new Error("database unavailable"));

        await act(async () => {
            (container.querySelector('button[aria-label="chat.actions.clear"]') as HTMLButtonElement).click();
        });
        const confirmButton = Array.from(container.querySelectorAll("button"))
            .find(button => button.textContent === "chat.actions.confirm_clear_button") as HTMLButtonElement;
        expect(confirmButton).not.toBeUndefined();

        await act(async () => {
            confirmButton.click();
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(bridge.clearHistory).toHaveBeenCalledTimes(1);
        expect(container.textContent).toContain("Keep this reply");
        expect(container.textContent).toContain("database unavailable");
        expect(textarea.disabled).toBe(false);
    });

    it("triggers resync to recover assistant message and clears busy state when all streaming events are lost", async () => {
        loadConversationMock.mockResolvedValueOnce({
            id: "conv-1",
            character_id: "char-1",
            title: "Test Conversation",
            topic: "",
            pinned_state: "{}",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            messages: [
                { id: 201, role: "user", content: "Lost events test", created_at: "2026-01-01T00:00:00Z" },
                { id: 202, role: "assistant", content: "Persisted answer from database", created_at: "2026-01-01T00:00:01Z" },
            ],
        });

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        // 1. Submit message
        await act(async () => {
            setTextareaValue(textarea, "Lost events test");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const clientRequestId = streamChatMock.mock.calls[0][0].client_request_id;
        expect(textarea.disabled).toBe(true);

        // 2. Completely drop turnStart, delta, and finish (no event callbacks invoked)
        // streamChat returns with authoritative message IDs
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 201,
                assistant_message_id: 202,
                client_request_id: clientRequestId,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 3. Verify resync was invoked and UI unblocked
        expect(loadConversationMock).toHaveBeenCalledWith("conv-1");
        expect(textarea.disabled).toBe(false);
        expect(container.textContent).toContain("Persisted answer from database");
    });

    it("safely ignores late chat-turn-finish arriving after fallback teardown has completed", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        await act(async () => {
            setTextareaValue(textarea, "Late finish test");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const clientRequestId = streamChatMock.mock.calls[0][0].client_request_id;

        // Delta arrives
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-late-1",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                user_message_id: 301,
            });
            turnDeltaCb?.({
                turn_id: "turn-late-1",
                delta: "Response before late finish",
            });
        });

        // streamChat resolves first
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 301,
                assistant_message_id: 302,
                client_request_id: clientRequestId,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(textarea.disabled).toBe(false);

        // Now late finish event arrives!
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn-late-1",
                status: "completed",
                conversation_id: "conv-1",
                assistant_message_id: 302,
                client_request_id: clientRequestId,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Verify still unblocked, no error thrown
        expect(textarea.disabled).toBe(false);
        expect(container.textContent).toContain("Response before late finish");
    });

    it("removes partial assistant bubble, suppresses TTS, and unblocks UI when cancelled response arrives and chat-turn-finish is dropped", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        // 1. Submit user message
        await act(async () => {
            setTextareaValue(textarea, "Cancel test message");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const clientRequestId = streamChatMock.mock.calls[0][0].client_request_id;
        expect(textarea.disabled).toBe(true);

        // 2. Start turn and emit partial delta
        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-cancel-1",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                user_message_id: 401,
            });
            turnDeltaCb?.({
                turn_id: "turn-cancel-1",
                delta: "Partial stream content before user stopped",
            });
        });

        expect(container.textContent).toContain("Partial stream content before user stopped");

        // 3. Backend cancels turn: returns status: 'cancelled' and assistant_message_id: null.
        // chat-turn-finish is LOST (never fired).
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 401,
                assistant_message_id: null,
                client_request_id: clientRequestId,
                status: "cancelled",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 4. Verify UI is unblocked
        expect(textarea.disabled).toBe(false);

        // 5. Verify partial assistant content is REMOVED from the message list
        expect(container.textContent).not.toContain("Partial stream content before user stopped");

        // 6. User message is still preserved
        expect(container.textContent).toContain("Cancel test message");

        // 7. Verify TTS was NEVER called for the cancelled turn
        expect(bridge.synthesize).not.toHaveBeenCalled();

        // 8. User can immediately send another message without deadlock
        await act(async () => {
            setTextareaValue(textarea, "Next message after cancel");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(2);
        expect(streamChatMock.mock.calls[1][0].message).toBe("Next message after cancel");
    });

    it("handles stream failure when chat-failure is lost and streamChat rejects (Plan A)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        await act(async () => {
            setTextareaValue(textarea, "Failure test message");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const requestPayload = streamChatMock.mock.calls[0][0];
        const clientRequestId = requestPayload.client_request_id;

        // UI is busy
        expect(textarea.disabled).toBe(true);

        // Turn finish event fires with status = "error"
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn-fail-1",
                status: "error",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                assistant_message_id: null,
            });
        });

        // NOTE: chat-failure is LOST (never emitted), but streamChat Promise rejects with Kokoro error!
        await act(async () => {
            streamChatRejecter?.("LLM provider connection timed out after 30s");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Fast-forward timeout for error bubble if needed (setTimeout 500ms in onCatchError)
        await act(async () => {
            await new Promise((r) => setTimeout(r, 600));
        });

        // UI is unblocked
        expect(textarea.disabled).toBe(false);

        // User can send another message without being blocked
        await act(async () => {
            setTextareaValue(textarea, "Retry message after failure");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(2);
        expect(streamChatMock.mock.calls[1][0].message).toBe("Retry message after failure");
    });

    it("handles defensive stream failure when streamChat resolves with status: error (Plan C)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        await act(async () => {
            setTextareaValue(textarea, "Defensive failure test message");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const requestPayload = streamChatMock.mock.calls[0][0];
        const clientRequestId = requestPayload.client_request_id;

        // UI is busy
        expect(textarea.disabled).toBe(true);

        // Turn finish event fires with status = "error"
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn-fail-2",
                status: "error",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                assistant_message_id: null,
            });
        });

        // streamChat resolves with status = "error"
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 501,
                assistant_message_id: null,
                client_request_id: clientRequestId,
                status: "error",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            await new Promise((r) => setTimeout(r, 600));
        });

        // UI is unblocked
        expect(textarea.disabled).toBe(false);

        // User can send another message without being blocked
        await act(async () => {
            setTextareaValue(textarea, "Retry message after defensive error");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(2);
        expect(streamChatMock.mock.calls[1][0].message).toBe("Retry message after defensive error");
    });

    it("clears busy state and triggers resync when chat-turn-acknowledged arrives, chat-turn-start is lost, and chat-turn-finish arrives", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        const form = container.querySelector("form") as HTMLFormElement;

        await act(async () => {
            setTextareaValue(textarea, "ACK received but start lost message");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        const requestPayload = streamChatMock.mock.calls[0][0];
        const clientRequestId = requestPayload.client_request_id;

        // UI is busy
        expect(textarea.disabled).toBe(true);

        // 1. Backend acknowledges turn
        await act(async () => {
            turnAcknowledgedCb?.({
                turn_id: "turn-ack-lost-start-1",
                client_request_id: clientRequestId,
            });
        });

        // 2. chat-turn-start is LOST (never sent/received)

        // 3. chat-turn-finish arrives
        await act(async () => {
            turnFinishCb?.({
                turn_id: "turn-ack-lost-start-1",
                status: "completed",
                client_request_id: clientRequestId,
                conversation_id: "conv-1",
                assistant_message_id: 301,
            });
        });

        // 4. streamChat also resolves
        await act(async () => {
            streamChatResolver?.({
                conversation_id: "conv-1",
                user_message_id: 201,
                assistant_message_id: 301,
                client_request_id: clientRequestId,
                status: "completed",
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            await new Promise((r) => setTimeout(r, 100));
        });

        // UI is completely unblocked
        expect(textarea.disabled).toBe(false);

        // Because chat-turn-start was lost, needsResync triggered loadConversation
        expect(loadConversationMock).toHaveBeenCalledWith("conv-1");

        // User can send another message immediately
        await act(async () => {
            setTextareaValue(textarea, "Follow up after lost start");
        });

        await act(async () => {
            form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(2);
        expect(streamChatMock.mock.calls[1][0].message).toBe("Follow up after lost start");
    });
});
