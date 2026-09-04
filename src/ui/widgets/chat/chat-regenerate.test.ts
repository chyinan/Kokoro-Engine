import { describe, expect, it } from "vitest";
import type { ChatRequest } from "../../../lib/kokoro-bridge";

describe("chat regeneration contract and logic", () => {
    it("allows ChatRequest to specify regenerate flag", () => {
        const req: ChatRequest = {
            message: "Hello again",
            character_id: "char-1",
            regenerate: true,
        };
        expect(req.regenerate).toBe(true);
    });

    it("calculates correct delete count and user message index for regeneration", () => {
        interface Msg {
            role: "user" | "kokoro";
            text: string;
            images?: string[];
        }
        const msgs: Msg[] = [
            { role: "user", text: "Question 1" },
            { role: "kokoro", text: "Answer 1" },
            { role: "user", text: "Question 2", images: ["img1.png"] },
            { role: "kokoro", text: "Answer 2" },
        ];

        // Target: regenerate Answer 2 (index 3)
        const targetIndex = 3;
        const lastUserIdx = msgs.slice(0, targetIndex).reverse().findIndex(m => m.role === "user");
        expect(lastUserIdx).not.toBe(-1);
        const userMsgIndex = targetIndex - 1 - lastUserIdx;
        expect(userMsgIndex).toBe(2);
        expect(msgs[userMsgIndex].text).toBe("Question 2");
        expect(msgs[userMsgIndex].images).toEqual(["img1.png"]);

        const messagesToDelete = msgs.length - targetIndex;
        expect(messagesToDelete).toBe(1);

        // Target: regenerate Answer 1 (index 1) - branching back to previous turn
        const targetIndex1 = 1;
        const lastUserIdx1 = msgs.slice(0, targetIndex1).reverse().findIndex(m => m.role === "user");
        const userMsgIndex1 = targetIndex1 - 1 - lastUserIdx1;
        expect(userMsgIndex1).toBe(0);
        expect(msgs[userMsgIndex1].text).toBe("Question 1");

        const messagesToDelete1 = msgs.length - targetIndex1;
        expect(messagesToDelete1).toBe(3);
    });
});
