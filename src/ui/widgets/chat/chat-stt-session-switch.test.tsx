// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import * as bridge from "../../../lib/kokoro-bridge";
import { loadSavedCharacterDraft } from "./chat-draft-layout";

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

let capturedOnFinalTranscription: ((text: string) => void) | null = null;
let capturedOnWakeWordDetected: ((text?: string) => void) | null = null;
let voiceInputState = "idle";

// Mock hooks
vi.mock("../../hooks", () => ({
    VoiceState: { Idle: "idle", Listening: "listening", Processing: "processing", Speaking: "speaking", Error: "error" },
    useVoiceInput: (onFinal: (text: string) => void) => {
        capturedOnFinalTranscription = onFinal;
        return {
            state: voiceInputState,
            volume: 0,
            partialText: "",
            start: vi.fn(),
            stop: vi.fn(),
        };
    },
    useWakeWord: ({ onWakeWordDetected }: any = {}) => {
        capturedOnWakeWordDetected = onWakeWordDetected;
        return { state: "idle", isListening: false, start: vi.fn(), stop: vi.fn() };
    },
    useTypingReveal: ({ onReveal }: any) => ({
        pushDelta: (delta: string) => onReveal?.(delta),
        flush: vi.fn(),
        reset: vi.fn(),
    }),
}));

// Mock @tauri-apps/api/event
vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
    emit: vi.fn(async () => {}),
}));

// Mock services
vi.mock("../../../core/services", () => ({
    audioPlayer: { isPlaying: false },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("ChatPanel STT session switch draft preservation", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;
    let streamChatMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        vi.clearAllMocks();
        localStorage.clear();
        sessionStorage.clear();
        localStorage.setItem("kokoro_active_character_id", "char-1");
        localStorage.setItem("kokoro_stt_enabled", "true");
        localStorage.setItem("kokoro_stt_auto_send", "true");

        capturedOnFinalTranscription = null;
        capturedOnWakeWordDetected = null;
        voiceInputState = "idle";
        streamChatMock = vi.fn(async () => ({ status: "completed" }));

        vi.spyOn(bridge, "streamChat").mockImplementation(streamChatMock as any);
        vi.spyOn(bridge, "loadConversation").mockImplementation(async (id: string) => ({
            id,
            character_id: "char-1",
            title: `Conversation ${id}`,
            topic: "",
            pinned_state: "{}",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            messages: [],
        }) as any);
        vi.spyOn(bridge, "listConversations").mockImplementation(async () => [
            {
                id: "conv-1",
                character_id: "char-1",
                title: "Conversation 1",
                topic: "",
                pinned_state: "{}",
                created_at: "2026-01-01T00:00:00Z",
                updated_at: "2026-01-01T00:00:00Z",
                messages: [],
            },
            {
                id: "conv-2",
                character_id: "char-1",
                title: "Conversation 2",
                topic: "",
                pinned_state: "{}",
                created_at: "2026-01-01T00:00:00Z",
                updated_at: "2026-01-01T00:00:00Z",
                messages: [],
            },
        ] as any);
        vi.spyOn(bridge, "listCharacters").mockImplementation(async () => [
            { id: "char-1", name: "Character 1", user_nickname: "User", persona: "", source_format: "manual", created_at: 0, updated_at: 0 },
            { id: "char-2", name: "Character 2", user_nickname: "User", persona: "", source_format: "manual", created_at: 0, updated_at: 0 },
        ] as any);
        vi.spyOn(bridge, "setVisionTextInputFocused").mockImplementation(async () => undefined);
        vi.spyOn(bridge, "synthesize").mockImplementation(async () => undefined);
        vi.spyOn(bridge, "onChatTurnStart").mockImplementation(() => Promise.resolve(() => {}));
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
        vi.spyOn(bridge, "cancelChatTurn").mockImplementation(async () => undefined);

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

    it("preserves STT transcription in input and character draft when switching conversation for the same character", async () => {
        let memoryStatusResolve: ((value: any) => void) | null = null;
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(() => {
            return new Promise((resolve) => {
                memoryStatusResolve = resolve;
            });
        });

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(capturedOnFinalTranscription).toBeTypeOf("function");

        // User speaks and STT produces final transcription
        act(() => {
            capturedOnFinalTranscription!("Hello transcribed text for conv-1");
        });

        // While ensureMemoryModelReady is waiting, user switches conversation for char-1 (from conv-1 to conv-2)
        await act(async () => {
            window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
                detail: {
                    runtime: { character_id: "char-1", character_name: "Character 1" },
                    target_conversation_id: "conv-2",
                },
            }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Now memory check finishes
        await act(async () => {
            memoryStatusResolve?.({ installed: true });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Stream chat MUST NOT have been called because session changed
        expect(streamChatMock).not.toHaveBeenCalled();

        // But STT text MUST be preserved in input box and character draft
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.value).toBe("Hello transcribed text for conv-1");

        const savedDraft = loadSavedCharacterDraft("char-1");
        expect(savedDraft).toBe("Hello transcribed text for conv-1");
    });

    it("preserves STT transcription in original character draft when switching to a different character", async () => {
        let memoryStatusResolve: ((value: any) => void) | null = null;
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(() => {
            return new Promise((resolve) => {
                memoryStatusResolve = resolve;
            });
        });

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(capturedOnFinalTranscription).toBeTypeOf("function");

        // User speaks on char-1
        act(() => {
            capturedOnFinalTranscription!("Speech intended for char-1");
        });

        // User switches to char-2 while async memory check is in progress
        await act(async () => {
            window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
                detail: {
                    runtime: { character_id: "char-2", character_name: "Character 2" },
                    target_conversation_id: "conv-char2",
                },
            }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Memory check resolves
        await act(async () => {
            memoryStatusResolve?.({ installed: true });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Must not call streamChat
        expect(streamChatMock).not.toHaveBeenCalled();

        // char-1's draft storage MUST contain the transcribed text
        expect(loadSavedCharacterDraft("char-1")).toBe("Speech intended for char-1");

        // char-2's input must NOT be polluted with char-1's text
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.value).toBe("");
    });

    it("isolates non-auto-send transcription to original character draft when switching character during speech", async () => {
        localStorage.setItem("kokoro_stt_auto_send", "false");

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Mic toggle starts listening on char-1
        const micBtn = container.querySelector('button[title="chat.input.mic.title.idle"]') as HTMLButtonElement;
        expect(micBtn).not.toBeNull();
        await act(async () => {
            micBtn.click();
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // User switches to char-2 while recording
        await act(async () => {
            window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
                detail: {
                    runtime: { character_id: "char-2", character_name: "Character 2" },
                    target_conversation_id: "conv-char2",
                },
            }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Transcription finishes for char-1's speech
        await act(async () => {
            capturedOnFinalTranscription!("Manual review speech for char-1");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Must be saved to char-1's draft, NOT char-2's input
        expect(loadSavedCharacterDraft("char-1")).toBe("Manual review speech for char-1");
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.value).toBe("");
    });

    it("restores base draft correctly on empty transcription", async () => {
        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        await act(async () => {
            textarea.value = "Pre-existing draft";
            textarea.dispatchEvent(new Event("input", { bubbles: true }));
        });

        // Mic toggle starts listening and captures base draft
        const micBtn = container.querySelector('button[title="chat.voiceInput"]') as HTMLButtonElement;
        if (micBtn) {
            act(() => {
                micBtn.click();
            });
        }

        // Empty transcription arrives
        await act(async () => {
            capturedOnFinalTranscription!("   ");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(textarea.value).toBe("Pre-existing draft");
    });

    it("does not auto-send to new conversation when switching conversation for same character during mic recording", async () => {
        let memoryStatusResolve: ((value: any) => void) | null = null;
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(() => {
            return new Promise((resolve) => {
                memoryStatusResolve = resolve;
            });
        });

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 1. User is on conv-1. Click mic to start recording
        const micBtn = container.querySelector('button[title="chat.input.mic.title.idle"]') as HTMLButtonElement;
        expect(micBtn).not.toBeNull();
        await act(async () => {
            micBtn.click();
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 2. WHILE recording is active, user switches to conv-2 for the same character
        await act(async () => {
            window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
                detail: {
                    runtime: { character_id: "char-1", character_name: "Character 1" },
                    target_conversation_id: "conv-2",
                },
            }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 3. User finishes speaking; final transcription arrives
        await act(async () => {
            capturedOnFinalTranscription!("Speech recorded while on conv-1");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 4. Memory check resolves
        await act(async () => {
            memoryStatusResolve?.({ installed: true });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // MUST NOT call streamChat because conversation was switched during recording!
        expect(streamChatMock).not.toHaveBeenCalled();

        // But STT text MUST be preserved in input box and character draft
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.value).toBe("Speech recorded while on conv-1");
        expect(loadSavedCharacterDraft("char-1")).toBe("Speech recorded while on conv-1");
    });

    it("does not auto-send when switching conversation for same character during wake-word recording", async () => {
        let memoryStatusResolve: ((value: any) => void) | null = null;
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockImplementation(() => {
            return new Promise((resolve) => {
                memoryStatusResolve = resolve;
            });
        });

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(capturedOnWakeWordDetected).toBeTypeOf("function");

        // 1. Wake word detected while on conv-1, starts recording
        await act(async () => {
            capturedOnWakeWordDetected!();
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 2. While recording, user switches to conv-2 for the same character
        await act(async () => {
            window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
                detail: {
                    runtime: { character_id: "char-1", character_name: "Character 1" },
                    target_conversation_id: "conv-2",
                },
            }));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 3. Speech completes and final transcription arrives
        await act(async () => {
            capturedOnFinalTranscription!("Wake-word speech from conv-1");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // 4. Memory check resolves
        await act(async () => {
            memoryStatusResolve?.({ installed: true });
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // MUST NOT call streamChat because session changed!
        expect(streamChatMock).not.toHaveBeenCalled();

        // Preserved in input and draft
        const textarea = container.querySelector('textarea[data-onboarding-id="chat-input"]') as HTMLTextAreaElement;
        expect(textarea.value).toBe("Wake-word speech from conv-1");
        expect(loadSavedCharacterDraft("char-1")).toBe("Wake-word speech from conv-1");
    });

    it("auto-sends correctly when conversation does not switch during mic recording", async () => {
        vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockResolvedValue({ installed: true } as any);

        await act(async () => {
            root.render(createElement(ChatPanel));
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Click mic to record
        const micBtn = container.querySelector('button[title="chat.input.mic.title.idle"]') as HTMLButtonElement;
        await act(async () => {
            micBtn.click();
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        // Final transcription arrives without any conversation switch
        await act(async () => {
            capturedOnFinalTranscription!("Send this message");
            for (let i = 0; i < 5; i++) await Promise.resolve();
        });

        expect(streamChatMock).toHaveBeenCalledTimes(1);
        expect(streamChatMock).toHaveBeenCalledWith(
            expect.objectContaining({
                message: "Send this message",
                conversation_id: "conv-1",
            })
        );
    });
});
