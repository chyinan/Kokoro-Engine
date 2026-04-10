import { describe, expect, it } from "vitest";
import { parseKokoroError } from "./kokoro-bridge";

describe("parseKokoroError structured payload", () => {
    it("keeps stage/retryable/trace_id for observability", () => {
        const raw = JSON.stringify({
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            stage: "llm_stream",
            retryable: true,
            trace_id: "turn-xyz",
        });

        const parsed = parseKokoroError(raw);
        expect(typeof parsed).toBe("object");
        if (typeof parsed === "string") throw new Error("expected object");

        expect(parsed.code).toBe("CHAT_STREAM_ERROR");
        expect(parsed.stage).toBe("llm_stream");
        expect(parsed.retryable).toBe(true);
        expect(parsed.trace_id).toBe("turn-xyz");
    });
});
