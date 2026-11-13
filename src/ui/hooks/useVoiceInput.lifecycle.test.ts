// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@tauri-apps/api/core";
import { useVoiceInput } from "./useVoiceInput";
import { deferred } from "../../lib/chat/__tests__/harness";

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => undefined),
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("useVoiceInput recovery", () => {
    let container: HTMLDivElement;
    let root: Root;
    let hookState: ReturnType<typeof useVoiceInput> | null;
    let getUserMedia: ReturnType<typeof vi.fn>;

    function TestHarness(): null {
        hookState = useVoiceInput(vi.fn());
        return null;
    }

    beforeEach(() => {
        vi.clearAllMocks();
        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
        hookState = null;
        getUserMedia = vi.fn().mockRejectedValue(new Error("permission denied"));
        Object.defineProperty(navigator, "mediaDevices", {
            configurable: true,
            value: { getUserMedia },
        });
        vi.mocked(invoke).mockResolvedValue(undefined);
    });

    afterEach(() => {
        vi.unstubAllGlobals();
        act(() => root.unmount());
        document.body.replaceChildren();
    });

    it("returns to idle after microphone startup fails so the user can retry", async () => {
        await act(async () => {
            root.render(createElement(TestHarness));
        });

        await act(async () => {
            await hookState?.start();
        });

        expect(hookState?.state).toBe("idle");
        expect(getUserMedia).toHaveBeenCalledOnce();
    });

    it("keeps interrupt capture idle when the backend denies admission and accepts a later idle interrupt", async () => {
        vi.stubGlobal("__TAURI_INTERNALS__", {});
        await act(async () => root.render(createElement(TestHarness)));
        const onStarted = vi.fn();
        const options = { autoStopOnSilence: true, voiceInterrupt: { isAllowed: () => true, onStarted } };
        vi.mocked(invoke).mockResolvedValueOnce(false).mockResolvedValueOnce(true);

        await act(async () => { await hookState?.start(options); });
        expect(invoke).toHaveBeenCalledExactlyOnceWith("start_native_mic", { autoStopOnSilence: true, onlyWhenChatIdle: true });
        expect(hookState?.state).toBe("idle");
        expect(onStarted).not.toHaveBeenCalled();
        expect(getUserMedia).not.toHaveBeenCalled();

        await act(async () => { await hookState?.start(options); });
        expect(hookState?.state).toBe("listening");
        expect(onStarted).toHaveBeenCalledOnce();
        vi.mocked(invoke).mockImplementation(async (command) => command === "complete_audio_stream"
            ? { text: "", segments: [] } : undefined);
        await act(async () => { await hookState?.stop(); });
    });

    it("stops a delayed native start if auto-send was disabled while it was pending", async () => {
        vi.stubGlobal("__TAURI_INTERNALS__", {});
        await act(async () => root.render(createElement(TestHarness)));
        const nativeStart = deferred<boolean>();
        vi.mocked(invoke).mockReturnValueOnce(nativeStart.promise);
        const onStarted = vi.fn();
        let autoSend = true;
        let starting: Promise<void> | undefined;
        await act(async () => {
            starting = hookState?.start({ voiceInterrupt: { isAllowed: () => autoSend, onStarted } });
        });
        expect(invoke).toHaveBeenCalledWith("start_native_mic", expect.objectContaining({ onlyWhenChatIdle: true }));

        autoSend = false;
        await act(async () => { nativeStart.resolve(true); await starting; });
        expect(invoke).toHaveBeenLastCalledWith("stop_native_mic");
        expect(hookState?.state).toBe("idle");
        expect(onStarted).not.toHaveBeenCalled();
        expect(getUserMedia).not.toHaveBeenCalled();
    });
});
