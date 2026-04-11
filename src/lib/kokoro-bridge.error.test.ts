import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(),
    listen: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { parseKokoroError, parseFailureEvent, parseLegacyChatError, getEngineInfo } from "./kokoro-bridge";

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

describe("failure event parsing", () => {
    it("parses structured failure event object", () => {
        const payload = {
            event_id: "evt-1",
            timestamp: "2026-04-11T10:00:00Z",
            domain: "chat",
            stage: "llm_stream",
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            retryable: true,
            trace_id: "turn-123",
            conversation_id: "conv-1",
            turn_id: "turn-123",
            character_id: "char-1",
            context: { deny_kind: "policy_denied" },
        };

        const parsed = parseFailureEvent(payload);
        expect(parsed?.code).toBe("CHAT_STREAM_ERROR");
        expect(parsed?.stage).toBe("llm_stream");
        expect(parsed?.retryable).toBe(true);
        expect(parsed?.trace_id).toBe("turn-123");
    });

    it("keeps legacy string payload via parseLegacyChatError", () => {
        const parsed = parseLegacyChatError("network unreachable");
        expect(parsed).toBe("network unreachable");
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
