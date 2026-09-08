// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import * as bridge from "../../../lib/kokoro-bridge";
import {
    clearRegisteredExternalTurns,
    registerExternalTurn,
} from "./chat-turn-lifecycle";

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

describe("ChatPanel - External turns handling (onboarding and mod requests)", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;
    let turnStartCb: ((event: any) => void) | null = null;
    let cancelChatTurnMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        clearRegisteredExternalTurns();
        cancelChatTurnMock = vi.fn().mockResolvedValue(undefined);
        vi.spyOn(bridge, "cancelChatTurn").mockImplementation(cancelChatTurnMock as any);
        vi.spyOn(bridge, "listConversations").mockImplementation(vi.fn(async () => []));
        vi.spyOn(bridge, "listCharacters").mockImplementation(vi.fn(async () => [{
            id: "char-1",
            name: "Kokoro",
            avatar: null,
            persona: "",
            user_nickname: "",
            source_format: "kokoro",
            created_at: 0,
            updated_at: 0,
        }]));
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(vi.fn(async () => ({ installed: true } as any)));
        vi.spyOn(bridge, "setVisionTextInputFocused").mockImplementation(vi.fn(async () => undefined));
        vi.spyOn(bridge, "synthesize").mockImplementation(vi.fn(async () => undefined));

        vi.spyOn(bridge, "onChatTurnStart").mockImplementation((cb: any) => {
            turnStartCb = cb;
            return Promise.resolve(() => { turnStartCb = null; });
        });
        vi.spyOn(bridge, "onChatTurnDelta").mockImplementation(() => Promise.resolve(() => {}));
        vi.spyOn(bridge, "onChatTurnFinish").mockImplementation(() => Promise.resolve(() => {}));
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
        clearRegisteredExternalTurns();
    });

    it("does NOT cancel turn-start when interactionDisabled is true (onboarding mode)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel, { interactionDisabled: true }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-onboarding-test-1",
                client_request_id: "onboarding_12345",
                conversation_id: "conv-1",
                user_message_id: 1,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // ChatPanel should ignore the turn and NOT call cancelChatTurn
        expect(cancelChatTurnMock).not.toHaveBeenCalled();
    });

    it("does NOT cancel turn-start for mod request with mod_ prefix even when ChatPanel is active", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel, { interactionDisabled: false }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-mod-test-1",
                client_request_id: "mod_1725700000_abc123",
                conversation_id: "conv-1",
                user_message_id: 2,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(cancelChatTurnMock).not.toHaveBeenCalled();
    });

    it("does NOT cancel turn-start for onboarding request with onboarding_ prefix when ChatPanel is active", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel, { interactionDisabled: false }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-onboarding-test-2",
                client_request_id: "onboarding_4f1b51e0-7df0-47fa-8025-5e6080512f45",
                conversation_id: null,
                user_message_id: 3,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(cancelChatTurnMock).not.toHaveBeenCalled();
    });

    it("does NOT cancel turn-start for dynamically registered external turn", async () => {
        registerExternalTurn("custom_external_client_id_42");

        await act(async () => {
            root.render(createElement(ChatPanel, { interactionDisabled: false }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-custom-ext-1",
                client_request_id: "custom_external_client_id_42",
                conversation_id: "conv-1",
                user_message_id: 4,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(cancelChatTurnMock).not.toHaveBeenCalled();
    });

    it("cancels un-authorized untracked turn when pendingRequest is null (preserving convergence safety)", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel, { interactionDisabled: false }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        await act(async () => {
            turnStartCb?.({
                turn_id: "turn-rogue-1",
                client_request_id: "untracked_rogue_turn",
                conversation_id: "conv-1",
                user_message_id: 5,
            });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(cancelChatTurnMock).toHaveBeenCalledWith("turn-rogue-1", "stale_turn_no_pending_request");
    });
});
