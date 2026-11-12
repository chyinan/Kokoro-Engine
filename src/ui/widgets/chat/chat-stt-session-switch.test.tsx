// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import { chatViewFixture as chat } from "../../../lib/chat/view-test-fixture";
vi.mock("../../../lib/chat/application", async () => ({ chatApplication: (await import("../../../lib/chat/view-test-fixture")).chatViewFixture }));
vi.mock("../../../lib/kokoro-bridge", async original => ({ ...(await original<typeof import("../../../lib/kokoro-bridge")>()), setVisionTextInputFocused: vi.fn(async () => {}) }));
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
    beforeEach(async () => {
        vi.clearAllMocks(); chat.reset(); localStorage.clear(); sessionStorage.clear();
        localStorage.setItem("kokoro_stt_enabled", "true");
        localStorage.setItem("kokoro_stt_auto_send", "true");
        capturedOnFinalTranscription = null; capturedOnWakeWordDetected = null; voiceInputState = "idle";
        container = document.createElement("div"); document.body.append(container); root = createRoot(container);
        await act(async () => root.render(createElement(ChatPanel)));
    });
    afterEach(() => { act(() => root.unmount()); container.remove(); });
    function record() {
        const mic = container.querySelector<HTMLButtonElement>('button[aria-label="chat.input.mic.title.idle"]');
        expect(mic).not.toBeNull(); act(() => mic!.click());
    }
    async function finish(text: string) { await act(async () => capturedOnFinalTranscription?.(text)); }
    it("auto-sends through the application when the recording session stays current", async () => {
        record(); await finish("spoken words");
        expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({ message: "spoken words", character_id: "char-1", conversation_id: "conv-1" }));
    });
    it.each(["conv-2", null])("preserves transcription and its saved draft after same-character switch to %s", async conversation_id => {
        record(); await act(async () => chat.set({ conversation_id, generation: 2 })); await finish("belongs to the old session");
        expect(chat.send).not.toHaveBeenCalled();
        expect(container.querySelector("textarea")?.value).toBe("belongs to the old session");
        expect(loadSavedCharacterDraft("char-1")).toBe("belongs to the old session");
    });
    it("preserves the old character draft and does not populate the new character", async () => {
        record(); await act(async () => chat.set({ character_id: "char-2", conversation_id: "conv-2", generation: 2 })); await finish("char one speech");
        expect(chat.send).not.toHaveBeenCalled();
        expect(loadSavedCharacterDraft("char-1")).toBe("char one speech");
        expect(container.querySelector("textarea")?.value).toBe("");
    });
    it("protects A to B to A using generation even when IDs match again", async () => {
        record(); await act(async () => chat.set({ generation: 3 })); await finish("old generation");
        expect(chat.send).not.toHaveBeenCalled(); expect(loadSavedCharacterDraft("char-1")).toBe("old generation");
    });
    it("keeps transcription as a draft while a history mutation is busy", async () => {
        record(); await act(async () => chat.set({ busy: true })); await finish("during deletion");
        expect(chat.send).not.toHaveBeenCalled(); expect(loadSavedCharacterDraft("char-1")).toBe("during deletion");
    });
    it("keeps manual-review speech as a saved draft", async () => {
        localStorage.setItem("kokoro_stt_auto_send", "false");
        await act(async () => window.dispatchEvent(new Event("kokoro-stt-settings-changed")));
        record(); await finish("review me"); expect(chat.send).not.toHaveBeenCalled();
        expect(loadSavedCharacterDraft("char-1")).toBe("review me");
    });
    it("wake-word recording captures the original session", async () => {
        await act(async () => capturedOnWakeWordDetected?.());
        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2 }));
        await finish("wake speech"); expect(chat.send).not.toHaveBeenCalled();
        expect(loadSavedCharacterDraft("char-1")).toBe("wake speech");
    });
    it("empty transcription restores the draft without submitting", async () => {
        record(); await finish("   "); expect(chat.send).not.toHaveBeenCalled();
    });
});
