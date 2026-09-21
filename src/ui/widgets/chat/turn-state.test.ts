import { describe, expect, it } from "vitest";
import {
    ensureTurnMessage,
    getApprovalErrorMessage,
    stripPlainTextFormatting,
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

    it("strips escaped and regular bold markers from date and weekday replies", () => {
        expect(stripPlainTextFormatting("今天是 \\*\\*2026年9月21日，星期一\\*\\*。"))
            .toBe("今天是 2026年9月21日，星期一。");
        expect(stripPlainTextFormatting("今天是 **2026年9月21日**，**星期一**。"))
            .toBe("今天是 2026年9月21日，星期一。");
    });

    it("strips multiple bold segments without removing ordinary stars", () => {
        expect(stripPlainTextFormatting("\\*\\*日期\\*\\*：\\*\\*2026-09-21\\*\\*。"))
            .toBe("日期：2026-09-21。");
        expect(stripPlainTextFormatting("2**3** and unfinished **bold"))
            .toBe("2**3** and unfinished **bold");
        expect(stripPlainTextFormatting("**bold**suffix"))
            .toBe("boldsuffix");
        expect(stripPlainTextFormatting("今天是 \\*星期一\\*。"))
            .toBe("今天是 星期一。");
        expect(stripPlainTextFormatting("今天是 *星期一*。"))
            .toBe("今天是 星期一。");
        expect(stripPlainTextFormatting("2 * 3 = 6\n* 条目"))
            .toBe("2 * 3 = 6\n* 条目");
        expect(stripPlainTextFormatting("literal \\* star"))
            .toBe("literal \\* star");
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

    it("keeps structured approval errors readable", () => {
        expect(getApprovalErrorMessage({
            code: "TOOL_APPROVAL_EXPIRED",
            message: "tool approval has expired",
        })).toBe("tool approval has expired");
    });
});
