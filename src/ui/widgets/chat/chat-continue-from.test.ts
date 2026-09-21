import { describe, expect, it } from "vitest";
import { getContinueFromCutoffIndex } from "./chat-continue-from";
import type { ChatPanelMessage } from "./turn-state";

describe("continue from chat message", () => {
    const messages: ChatPanelMessage[] = [
        { role: "user", text: "first question" },
        { role: "kokoro", text: "first answer" },
        { role: "user", text: "second question" },
        { role: "kokoro", text: "second answer" },
    ];

    it("keeps the selected user message and its existing assistant reply", () => {
        expect(getContinueFromCutoffIndex(messages, 0)).toBe(2);
        expect(getContinueFromCutoffIndex(messages, 2)).toBe(4);
    });

    it("keeps only the selected assistant message when starting from an assistant", () => {
        expect(getContinueFromCutoffIndex(messages, 1)).toBe(2);
    });

    it("keeps context rows between a user message and its existing answer", () => {
        const withContext: ChatPanelMessage[] = [
            { role: "user", text: "look at this image" },
            { role: "context", text: "vision context" },
            { role: "kokoro", text: "I can see it" },
            { role: "user", text: "continue" },
        ];

        expect(getContinueFromCutoffIndex(withContext, 0)).toBe(3);
    });

    it("does not preserve a later answer when the selected user has no answer", () => {
        const unanswered: ChatPanelMessage[] = [
            { role: "user", text: "unanswered question" },
            { role: "user", text: "follow-up question" },
            { role: "kokoro", text: "follow-up answer" },
        ];

        expect(getContinueFromCutoffIndex(unanswered, 0)).toBe(1);
    });

    it("ignores an invalid message index", () => {
        expect(getContinueFromCutoffIndex(messages, -1)).toBeNull();
        expect(getContinueFromCutoffIndex(messages, messages.length)).toBeNull();
    });
});
