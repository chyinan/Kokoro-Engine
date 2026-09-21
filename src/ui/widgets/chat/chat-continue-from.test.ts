import { describe, expect, it } from "vitest";
import { getContinueFromRegenerationIndex } from "./chat-continue-from";
import type { ChatPanelMessage } from "./turn-state";

describe("continue from chat message", () => {
    const messages: ChatPanelMessage[] = [
        { role: "user", text: "今天几号" },
        { role: "kokoro", text: "今天是 2026 年 9 月 21 日。" },
        { role: "user", text: "那今天星期几" },
    ];

    it("regenerates the selected user turn so its assistant reply is restored", () => {
        expect(getContinueFromRegenerationIndex(messages, 0)).toBe(1);
        expect(getContinueFromRegenerationIndex(messages, 2)).toBe(3);
    });

    it("keeps assistant-message continuation as a truncate-only action", () => {
        expect(getContinueFromRegenerationIndex(messages, 1)).toBeNull();
    });

    it("ignores an invalid message index", () => {
        expect(getContinueFromRegenerationIndex(messages, -1)).toBeNull();
        expect(getContinueFromRegenerationIndex(messages, messages.length)).toBeNull();
    });
});
