// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@tauri-apps/api/core";
import { useVoiceInput } from "./useVoiceInput";

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
});
