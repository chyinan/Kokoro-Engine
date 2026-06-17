import { describe, expect, it } from "vitest";
import {
    ensureTurnMessage,
    stripStreamingMarkup,
    updateTurnMessage,
    type ChatPanelMessage,
    type PendingTurnState,
} from "./turn-state";

function turn(overrides: Partial<PendingTurnState> = {}): PendingTurnState {
    return {
        turnId: "turn-1",
        messageIndex: null,
        rawText: "",
        visibleTextStarted: false,
        translationPending: false,
        tools: [],
        ...overrides,
    };
}

describe("chat turn state", () => {
    it("strips streamed control markup", () => {
        expect(stripStreamingMarkup("hello[TOOL_CALL:get_time|{}]world")).toBe("helloworld");
        expect(stripStreamingMarkup("hello[TRANSLATE:你好]")).toBe("hello");
    });

    it("creates one assistant message for a turn", () => {
        const state = turn();
        const messages = ensureTurnMessage([], state);
        expect(messages).toEqual([{ role: "kokoro", text: "", turnId: "turn-1" }]);
        expect(state.messageIndex).toBe(0);
    });

    it("updates the active assistant message", () => {
        const messages: ChatPanelMessage[] = [{ role: "kokoro", text: "", turnId: "turn-1" }];
        const state = turn({ messageIndex: 0 });
        const next = updateTurnMessage(messages, state, (message) => ({ ...message, text: "hello" }));
        expect(next[0]?.text).toBe("hello");
    });
});
