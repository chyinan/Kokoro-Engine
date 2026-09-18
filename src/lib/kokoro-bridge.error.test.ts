import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
    parseKokoroError,
    parseFailureEvent,
    parseLegacyChatError,
    getEngineInfo,
    getKokoroErrorMessage,
    onChatError,
} from "./kokoro-bridge";

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

    it("keeps object-form Tauri errors readable instead of [object Object]", () => {
        const raw = {
            code: "Internal",
            message: "download request failed: connection reset",
        };

        expect(parseKokoroError(raw)).toMatchObject(raw);
        expect(getKokoroErrorMessage(raw)).toBe("download request failed: connection reset");
    });

    it("extracts message from externally tagged Rust errors", () => {
        const raw = {
            Internal: "failed to download model file",
        };

        expect(parseKokoroError(raw)).toBe("failed to download model file");
        expect(getKokoroErrorMessage(raw)).toBe("failed to download model file");
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

describe("getKokoroErrorMessage robustness and structured errors", () => {
    it("extracts candidate message keys according to priority", () => {
        expect(getKokoroErrorMessage({ code: "IO_ERROR", reason: "磁盘空间不足" })).toBe("磁盘空间不足");
        expect(getKokoroErrorMessage({ code: "AUTH_ERROR", details: "无效的身份令牌" })).toBe("无效的身份令牌");
        expect(getKokoroErrorMessage({ msg: "服务暂不可用" })).toBe("服务暂不可用");
    });

    it("extracts nested record errors recursively", () => {
        expect(getKokoroErrorMessage({ error: { message: "网络连接拒绝" } })).toBe("网络连接拒绝");
        expect(getKokoroErrorMessage({ details: { reason: "模型校验和不匹配" } })).toBe("模型校验和不匹配");
    });

    it("handles circular references without throwing or producing [object Object]", () => {
        const circular: Record<string, unknown> = { code: "CIRCULAR_REF" };
        circular.self = circular;

        const result = getKokoroErrorMessage(circular);
        expect(result).toBe("CIRCULAR_REF");
        expect(result).not.toBe("[object Object]");
    });

    it("handles null, undefined, and empty objects safely", () => {
        expect(getKokoroErrorMessage(null)).toBe("Unknown error");
        expect(getKokoroErrorMessage(undefined)).toBe("Unknown error");
        expect(getKokoroErrorMessage({})).toBe("Unknown error");
        expect(getKokoroErrorMessage("")).toBe("Unknown error");
        expect(getKokoroErrorMessage("   ")).toBe("Unknown error");
    });

    it("uses custom toString when available and meaningful", () => {
        const customObj = {
            toString() {
                return "Custom error message";
            },
        };
        expect(getKokoroErrorMessage(customObj)).toBe("Custom error message");
    });

    it("never returns [object Object] for arbitrary objects", () => {
        const weirdObj = Object.create(null);
        expect(getKokoroErrorMessage(weirdObj)).toBe("Unknown error");
    });
});

