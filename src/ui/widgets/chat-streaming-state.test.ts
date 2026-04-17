import { describe, expect, it } from "vitest";
import { getStreamingRevealText, shouldRenderTypingIndicator, shouldRevealLiveTurnToolTrace } from "./chat-streaming-state";

describe("shouldRenderTypingIndicator", () => {
    it("shows the typing indicator while thinking before the assistant bubble exists", () => {
        expect(shouldRenderTypingIndicator({
            isThinking: true,
            activeMessageIndex: null,
            messages: [
                { role: "user" },
            ],
        })).toBe(true);
    });

    it("hides the typing indicator once the current assistant bubble already exists", () => {
        expect(shouldRenderTypingIndicator({
            isThinking: true,
            activeMessageIndex: 1,
            messages: [
                { role: "user" },
                { role: "kokoro" },
            ],
        })).toBe(false);
    });
});

describe("shouldRevealLiveTurnToolTrace", () => {
    it("keeps normal tool traces hidden before the assistant bubble exists", () => {
        expect(shouldRevealLiveTurnToolTrace({
            activeMessageIndex: null,
            approvalStatus: undefined,
            messages: [
                { role: "user" },
            ],
        })).toBe(false);
    });

    it("reveals tool traces once the current assistant bubble already exists", () => {
        expect(shouldRevealLiveTurnToolTrace({
            activeMessageIndex: 1,
            approvalStatus: undefined,
            messages: [
                { role: "user" },
                { role: "kokoro" },
            ],
        })).toBe(true);
    });

    it("still reveals pending approval requests immediately", () => {
        expect(shouldRevealLiveTurnToolTrace({
            activeMessageIndex: null,
            approvalStatus: "requested",
            messages: [
                { role: "user" },
            ],
        })).toBe(true);
    });
});

describe("getStreamingRevealText", () => {
    it("keeps typing visible while only whitespace has streamed in", () => {
        expect(getStreamingRevealText({
            accumulatedText: "\n  ",
            delta: "\n  ",
            hasVisibleTextStarted: false,
        })).toBe(null);
    });

    it("reveals the full buffered text when the first visible character arrives", () => {
        expect(getStreamingRevealText({
            accumulatedText: "\n  hello",
            delta: "hello",
            hasVisibleTextStarted: false,
        })).toBe("\n  hello");
    });

    it("reveals only the latest delta after visible text has started", () => {
        expect(getStreamingRevealText({
            accumulatedText: "\n  hello world",
            delta: " world",
            hasVisibleTextStarted: true,
        })).toBe(" world");
    });
});
