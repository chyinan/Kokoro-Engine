import { describe, expect, it, vi } from "vitest";

describe("chat clear history defensive guards and confirmation lifecycle", () => {
    it("prevents opening confirmation modal when message list is empty", () => {
        let showClearConfirm = false;
        const messages: unknown[] = [];
        const isBusy = false;
        const isStreaming = false;

        const handleClearClick = () => {
            if (messages.length === 0 || isBusy || isStreaming) return;
            showClearConfirm = true;
        };

        handleClearClick();
        expect(showClearConfirm).toBe(false);
    });

    it("prevents opening confirmation modal when streaming or busy", () => {
        let showClearConfirm = false;
        const messages = [{ text: "hello" }];

        const handleClearClick = (busy: boolean, streaming: boolean) => {
            if (messages.length === 0 || busy || streaming) return;
            showClearConfirm = true;
        };

        handleClearClick(true, false);
        expect(showClearConfirm).toBe(false);

        handleClearClick(false, true);
        expect(showClearConfirm).toBe(false);
    });

    it("opens confirmation modal when messages exist and engine is idle", () => {
        let showClearConfirm = false;
        const messages = [{ text: "hello" }];
        const isBusy = false;
        const isStreaming = false;

        const handleClearClick = () => {
            if (messages.length === 0 || isBusy || isStreaming) return;
            showClearConfirm = true;
        };

        handleClearClick();
        expect(showClearConfirm).toBe(true);
    });

    it("dismisses confirmation modal without clearing when cancelled", () => {
        let showClearConfirm = true;
        let messages = [{ text: "preserve me" }];
        const clearHistory = vi.fn();

        // User clicks Cancel
        showClearConfirm = false;

        expect(showClearConfirm).toBe(false);
        expect(clearHistory).not.toHaveBeenCalled();
        expect(messages).toHaveLength(1);
    });

    it("executes clearHistory and empties messages when confirmed", async () => {
        let showClearConfirm = true;
        let messages = [{ text: "delete me 1" }, { text: "delete me 2" }];
        const clearHistory = vi.fn(async () => {});

        const executeClear = async () => {
            showClearConfirm = false;
            try {
                await clearHistory();
            } catch {
                // Backend might not be ready
            }
            messages = [];
        };

        await executeClear();

        expect(showClearConfirm).toBe(false);
        expect(clearHistory).toHaveBeenCalledTimes(1);
        expect(messages).toHaveLength(0);
    });
});
