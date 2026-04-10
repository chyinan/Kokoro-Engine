import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(),
    listen: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { parseKokoroError, getEngineInfo } from "./kokoro-bridge";

const mockedInvoke = vi.mocked(invoke);

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

describe("bridge invoke error normalization", () => {
    it("throws structured KokoroErrorObject when invoke returns json error", async () => {
        mockedInvoke.mockRejectedValueOnce(
            JSON.stringify({
                code: "CHAT_STREAM_ERROR",
                message: "provider timeout",
                stage: "llm_stream",
                retryable: true,
                trace_id: "turn-xyz",
            })
        );

        await expect(getEngineInfo()).rejects.toMatchObject({
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            stage: "llm_stream",
            retryable: true,
            trace_id: "turn-xyz",
        });
    });

    it("keeps plain string error unchanged when invoke returns non-json error", async () => {
        mockedInvoke.mockRejectedValueOnce("network unreachable");

        await expect(getEngineInfo()).rejects.toBe("network unreachable");
    });
});
