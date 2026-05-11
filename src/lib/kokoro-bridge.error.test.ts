import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { parseKokoroError, parseFailureEvent, parseLegacyChatError, getEngineInfo, onChatError } from "./kokoro-bridge";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
    vi.clearAllMocks();
});

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

    it("keeps structured object payloads instead of stringifying them", () => {
        const parsed = parseKokoroError({
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            stage: "llm_stream",
            retryable: true,
            trace_id: "turn-xyz",
        });

        expect(typeof parsed).toBe("object");
        if (typeof parsed === "string") throw new Error("expected object");

        expect(parsed.message).toBe("provider timeout");
    });

    it("uses message fields from generic object errors", () => {
        expect(parseKokoroError({ message: "bad api key" })).toBe("bad api key");
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

    it("extracts message from structured chat-error json strings", () => {
        const parsed = parseLegacyChatError(JSON.stringify({
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            stage: "llm_stream",
            retryable: true,
            trace_id: "turn-123",
        }));

        expect(parsed).toBe("provider timeout");
    });

    it("extracts message from structured chat-error objects", () => {
        const parsed = parseLegacyChatError({
            code: "CHAT_STREAM_ERROR",
            message: "provider timeout",
            stage: "llm_stream",
            retryable: true,
            trace_id: "turn-123",
        });

        expect(parsed).toBe("provider timeout");
    });
});

describe("chat-error listener normalization", () => {
    it("passes readable messages to callbacks for object payloads", async () => {
        mockedListen.mockImplementationOnce(async (_event, handler) => {
            handler({
                event: "chat-error",
                id: 1,
                payload: {
                    code: "CHAT_STREAM_ERROR",
                    message: "provider timeout",
                    stage: "llm_stream",
                    retryable: true,
                    trace_id: "turn-123",
                },
            });
            return () => undefined;
        });

        const callback = vi.fn();
        await onChatError(callback);

        expect(callback).toHaveBeenCalledWith("provider timeout");
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
