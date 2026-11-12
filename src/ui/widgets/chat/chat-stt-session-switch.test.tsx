// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import ChatPanel from "../ChatPanel";
import { chatViewFixture as chat } from "../../../lib/chat/view-test-fixture";
import { recoverChatDraftSubmission, type ChatDraftSubmissionStatus } from "../../../lib/kokoro-bridge";
import { deferred, turn } from "../../../lib/chat/__tests__/harness";
import { setLatestCameraFrame } from "../../../lib/camera-frame-cache";
vi.mock("../../../lib/chat/application", async () => ({ chatApplication: (await import("../../../lib/chat/view-test-fixture")).chatViewFixture }));
vi.mock("../../../lib/kokoro-bridge", async original => ({
    ...(await original<typeof import("../../../lib/kokoro-bridge")>()),
    setVisionTextInputFocused: vi.fn(async () => {}),
    recoverChatDraftSubmission: vi.fn(),
}));
import { loadSavedCharacterDraft, loadSavedCharacterDraftImages, saveCharacterDraft, saveCharacterDraftImages } from "./chat-draft-layout";

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

describe("ChatPanel draft preservation", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;
    beforeEach(async () => {
        vi.clearAllMocks(); chat.reset(); localStorage.clear(); sessionStorage.clear();
        vi.mocked(recoverChatDraftSubmission).mockReset().mockResolvedValue("unsent");
        localStorage.setItem("kokoro_stt_enabled", "true");
        localStorage.setItem("kokoro_stt_auto_send", "true");
        capturedOnFinalTranscription = null; capturedOnWakeWordDetected = null; voiceInputState = "idle";
        container = document.createElement("div"); document.body.append(container); root = createRoot(container);
        await act(async () => root.render(createElement(ChatPanel)));
    });
    afterEach(() => { setLatestCameraFrame(null); act(() => root.unmount()); vi.useRealTimers(); container.remove(); });
    function record() {
        const mic = container.querySelector<HTMLButtonElement>('button[aria-label="chat.input.mic.title.idle"]');
        expect(mic).not.toBeNull(); act(() => mic!.click());
    }
    async function finish(text: string) { await act(async () => capturedOnFinalTranscription?.(text)); }
    const draftText = "  draft with an image  ";
    const draftImage = "data:image/png;base64,iVBORw0KGgo=";
    async function loadImageDraft() {
        saveCharacterDraft("draft-character", draftText);
        saveCharacterDraftImages("draft-character", [draftImage]);
        localStorage.setItem("kokoro_vision_enabled", "true");
        await act(async () => {
            chat.set({ character_id: "draft-character" });
            window.dispatchEvent(new Event("kokoro-vision-settings-changed"));
        });
    }
    async function send() {
        const button = container.querySelector<HTMLButtonElement>('button[aria-label="Send message"]');
        expect(button).not.toBeNull();
        await act(async () => button!.click());
    }
    it("keeps the unsent text and image when another window wins admission", async () => {
        await loadImageDraft();
        chat.send.mockRejectedValueOnce({ code: "Chat", message: "chat_turn_busy: A chat turn is already in progress" });
        await send();
        expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({ message: draftText.trim(), images: [draftImage] }));
        expect(container.querySelector("textarea")?.value).toBe(draftText);
        expect(container.querySelector('img[alt="pending"]')?.getAttribute("src")).toBe(draftImage);
        expect(loadSavedCharacterDraft("draft-character")).toBe(draftText);
        expect(loadSavedCharacterDraftImages("draft-character")).toEqual([draftImage]);
    });
    it("sends the automatic camera frame separately from the selected image", async () => {
        await loadImageDraft();
        const cameraFrame = "data:image/jpeg;base64,Y2FtZXJh";
        setLatestCameraFrame(cameraFrame);
        await send();
        expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({ images: [draftImage], camera_frame: cameraFrame }));
    });
    it("drops a cached camera frame when vision is disabled before the settings notification", async () => {
        await loadImageDraft();
        setLatestCameraFrame("data:image/jpeg;base64,Y2FtZXJh");
        localStorage.setItem("kokoro_vision_enabled", "false");

        await send();

        expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({
            message: draftText.trim(), images: undefined, camera_frame: undefined,
        }));
    });
    it("clears a submitted draft at persistence, before the generation finishes", async () => {
        await loadImageDraft();
        const response = deferred<Awaited<ReturnType<typeof chat.send>>>();
        chat.send.mockImplementationOnce(() => response.promise);
        await send();
        const requestId = chat.send.mock.lastCall?.[0].client_request_id;
        expect(requestId).toBeTruthy();
        const accepted = { ...turn(requestId!, 2, "accepted"), character_id: "draft-character", conversation_id: "conv-1", user_message_id: null, images: [draftImage] };
        await act(async () => chat.set({ revision: 2, busy: true, turns: [accepted] }));
        expect(container.querySelector("textarea")?.value).toBe(draftText);
        await act(async () => chat.set({ revision: 3, turns: [{ ...accepted, revision: 3, user_message_id: 10 }] }));
        expect(container.querySelector("textarea")?.value).toBe("");
        expect(loadSavedCharacterDraftImages("draft-character")).toEqual([]);
        await act(async () => response.resolve({ conversation_id: "conv-1", status: "completed", text: "reply" }));
    });
    async function closeWhileSending() {
        await loadImageDraft();
        chat.send.mockReturnValueOnce(new Promise(() => {}));
        await send();
        const requestId = chat.send.mock.lastCall?.[0].client_request_id;
        expect(requestId).toBeTruthy();
        act(() => root.unmount());
        root = createRoot(container);
        return requestId!;
    }
    it("does not restore a sent draft when a new window starts with only the saved user row", async () => {
        const requestId = await closeWhileSending();
        sessionStorage.clear();
        chat.set({ turns: [], messages: [{ id: 10, role: "user", content: draftText.trim(), created_at: "now" }] });
        const recovery = deferred<ChatDraftSubmissionStatus>();
        vi.mocked(recoverChatDraftSubmission).mockReturnValueOnce(recovery.promise);

        await act(async () => root.render(createElement(ChatPanel)));
        expect(recoverChatDraftSubmission).toHaveBeenCalledWith("draft-character", requestId);
        expect(container.querySelector("textarea")?.value).toBe("");
        await act(async () => recovery.resolve("persisted"));
        expect(container.querySelector("textarea")?.value).toBe("");
        expect(loadSavedCharacterDraft("draft-character")).toBe("");
        expect(chat.send).toHaveBeenCalledOnce();
    });
    it("clears the sent attachment when the reopened window sees the request bind", async () => {
        const requestId = await closeWhileSending();
        vi.mocked(recoverChatDraftSubmission).mockResolvedValueOnce("pending");
        await act(async () => root.render(createElement(ChatPanel)));
        expect(container.querySelector("textarea")?.value).toBe("");
        expect(container.querySelector('img[alt="pending"]')).toBeNull();

        await act(async () => chat.set({ turns: [{ ...turn(requestId, 3), character_id: "draft-character", user_message_id: 10 }] }));
        expect(loadSavedCharacterDraft("draft-character")).toBe("");
        expect(loadSavedCharacterDraftImages("draft-character")).toEqual([]);
    });
    it("keeps a newer draft and attachment when recovering a submitted draft", async () => {
        await closeWhileSending();
        const newImage = "data:image/png;base64,bmV3";
        saveCharacterDraft("draft-character", "new unsent text");
        saveCharacterDraftImages("draft-character", [draftImage, newImage]);
        vi.mocked(recoverChatDraftSubmission).mockResolvedValueOnce("persisted");

        await act(async () => root.render(createElement(ChatPanel)));
        expect(container.querySelector("textarea")?.value).toBe("new unsent text");
        expect(loadSavedCharacterDraftImages("draft-character")).toEqual([newImage]);
    });
    it("restores a submitted draft if its request ends before saving the user message", async () => {
        const requestId = await closeWhileSending();
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        vi.mocked(recoverChatDraftSubmission).mockResolvedValueOnce("pending");
        await act(async () => root.render(createElement(ChatPanel)));

        await act(async () => chat.set({ turns: [{ ...turn(requestId, 3, "error"), character_id: "draft-character", user_message_id: null }] }));
        await act(async () => vi.advanceTimersByTimeAsync(750));
        expect(container.querySelector("textarea")?.value).toBe(draftText);
        expect(container.querySelector('img[alt="pending"]')?.getAttribute("src")).toBe(draftImage);
    });
    it("clears an unsent marker before allowing the recovered draft to be submitted again", async () => {
        const requestId = await closeWhileSending();
        const recovery = deferred<ChatDraftSubmissionStatus>();
        vi.mocked(recoverChatDraftSubmission).mockReturnValueOnce(recovery.promise);
        await act(async () => root.render(createElement(ChatPanel)));
        expect(container.querySelector("textarea")?.disabled).toBe(true);
        expect(chat.send).toHaveBeenCalledOnce();

        await act(async () => recovery.resolve("unsent"));
        expect(container.querySelector("textarea")?.value).toBe(draftText);
        expect(container.querySelector('img[alt="pending"]')?.getAttribute("src")).toBe(draftImage);
        expect(localStorage.getItem("kokoro_chat_submission_draft-character")).toBeNull();
        expect(sessionStorage.getItem("kokoro_chat_submission_draft-character")).toBeNull();
        await act(async () => root.unmount());
        root = createRoot(container);
        await act(async () => root.render(createElement(ChatPanel)));
        expect(recoverChatDraftSubmission).toHaveBeenCalledExactlyOnceWith("draft-character", requestId);
        expect(container.querySelector("textarea")?.disabled).toBe(false);

        await send();
        expect(chat.send).toHaveBeenCalledTimes(2);
        expect(chat.send.mock.lastCall?.[0].client_request_id).not.toBe(requestId);
    });
    it("keeps the composer blocked through a failed lookup and recovers persistence without a turn event", async () => {
        await closeWhileSending();
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        vi.mocked(recoverChatDraftSubmission)
            .mockRejectedValueOnce(new Error("database temporarily unavailable"))
            .mockResolvedValueOnce("pending")
            .mockResolvedValueOnce("persisted");
        await act(async () => root.render(createElement(ChatPanel)));
        expect(container.querySelector("textarea")?.disabled).toBe(true);
        await act(async () => vi.advanceTimersByTimeAsync(750));
        expect(container.querySelector("textarea")?.disabled).toBe(true);
        await act(async () => vi.advanceTimersByTimeAsync(750));
        expect(container.querySelector("textarea")?.disabled).toBe(false);
        expect(loadSavedCharacterDraft("draft-character")).toBe("");
        expect(chat.send).toHaveBeenCalledOnce();
    });
    it("auto-sends through the application when the recording session stays current", async () => {
        record(); await finish("spoken words");
        expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({ message: "spoken words", character_id: "char-1", conversation_id: "conv-1" }));
    });
    it("keeps an auto-sent transcription in the input if admission fails", async () => {
        chat.send.mockRejectedValueOnce(new Error("chat_turn_busy"));
        record(); await finish("spoken words");
        expect(container.querySelector("textarea")?.value).toBe("spoken words");
        expect(loadSavedCharacterDraft("char-1")).toBe("spoken words");
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
