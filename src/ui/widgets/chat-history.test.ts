import { describe, expect, it } from "vitest";
import type { ConversationMessage } from "../../lib/kokoro-bridge";
import { buildChatMessagesFromConversation } from "./chat-history";

function createMessage(overrides: Partial<ConversationMessage>): ConversationMessage {
    return {
        role: "assistant",
        content: "",
        created_at: "2026-04-05T00:00:00Z",
        ...overrides,
    };
}

describe("buildChatMessagesFromConversation", () => {
    it("从 tool_result metadata 恢复完整工具身份字段", () => {
        const messages: Array<ConversationMessage> = [
            createMessage({
                role: "assistant",
                content: "让我检查一下。",
                metadata: JSON.stringify({
                    turn_id: "turn-1",
                }),
            }),
            createMessage({
                role: "tool",
                content: "读取成功",
                metadata: JSON.stringify({
                    type: "tool_result",
                    turn_id: "turn-1",
                    tool_call_id: "call-1",
                    tool_id: "mcp__filesystem__read_file",
                    tool_name: "read_file",
                    source: "mcp",
                    server_name: "filesystem",
                    needs_feedback: true,
                }),
            }),
        ];

        const chatMessages = buildChatMessagesFromConversation(messages);

        expect(chatMessages).toHaveLength(1);
        expect(chatMessages[0]?.tools).toEqual([
            expect.objectContaining({
                tool: "read_file",
                toolId: "mcp__filesystem__read_file",
                source: "mcp",
                serverName: "filesystem",
                needsFeedback: true,
            }),
        ]);
    });

    it("旧历史缺少新字段时回退到 tool_name 或 tool", () => {
        const messages: Array<ConversationMessage> = [
            createMessage({
                role: "assistant",
                content: "我来调用工具。",
                metadata: JSON.stringify({
                    turn_id: "turn-legacy",
                }),
            }),
            createMessage({
                role: "tool",
                content: "旧工具执行完成",
                metadata: JSON.stringify({
                    type: "tool_result",
                    turn_id: "turn-legacy",
                    tool: "legacy_lookup",
                }),
            }),
        ];

        const chatMessages = buildChatMessagesFromConversation(messages);

        expect(chatMessages).toHaveLength(1);
        expect(chatMessages[0]?.tools).toEqual([
            expect.objectContaining({
                tool: "legacy_lookup",
                text: "旧工具执行完成",
                toolId: undefined,
                source: undefined,
                serverName: undefined,
                needsFeedback: undefined,
            }),
        ]);
    });
});
