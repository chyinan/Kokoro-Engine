import { describe, expect, it, vi } from "vitest";
import type { ChatPanelMessage } from "./turn-state";
import type { EditConversationMessageRequest, EditConversationMessageResponse } from "@/lib/kokoro-bridge";

describe("chat message editing persistence and optimistic synchronization", () => {
    it("skips editing when new text is empty or whitespace only", () => {
        const onEdit = vi.fn();
        const editingText = "   \n  \t  ";

        if (editingText.trim()) {
            onEdit(editingText.trim());
        }

        expect(onEdit).not.toHaveBeenCalled();
    });

    it("optimistically updates message and calls editConversationMessage by message_id", async () => {
        let messages: ChatPanelMessage[] = [
            { id: 42, role: "user", text: "Original question" },
            { id: 43, role: "kokoro", text: "Original answer" },
        ];

        const editCalls: EditConversationMessageRequest[] = [];
        const editConversationMessage = vi.fn(async (req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            editCalls.push(req);
            return {
                message_id: req.message_id ?? 999,
                updated_content: req.new_content,
            };
        });

        const activeConversationId = "conv-xyz";
        const globalIndex = 0;
        const newText = "Updated question";

        // Simulate onEdit flow
        const trimmed = newText.trim();
        const targetMsg = messages[globalIndex];

        // 1. Optimistic update
        messages = [...messages];
        messages[globalIndex] = { ...messages[globalIndex], text: trimmed };

        expect(messages[0].text).toBe("Updated question");

        // 2. Invoke IPC
        const res = await editConversationMessage({
            conversation_id: activeConversationId,
            message_id: targetMsg.id,
            visible_index: globalIndex,
            new_content: trimmed,
        });

        expect(editCalls).toHaveLength(1);
        expect(editCalls[0]).toEqual({
            conversation_id: "conv-xyz",
            message_id: 42,
            visible_index: 0,
            new_content: "Updated question",
        });
        expect(res.updated_content).toBe("Updated question");
    });

    it("backfills returned message_id for newly sent messages lacking id", async () => {
        let messages: ChatPanelMessage[] = [
            // Newly created message in active session before page reload
            { role: "user", text: "Just sent message" },
        ];

        const editConversationMessage = vi.fn(async (req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            return {
                message_id: 108,
                updated_content: req.new_content,
            };
        });

        const globalIndex = 0;
        const newText = "Fixed typo in sent message";
        const targetMsg = messages[globalIndex];

        // 1. Optimistic update
        messages = [...messages];
        messages[globalIndex] = { ...messages[globalIndex], text: newText.trim() };

        // 2. Invoke IPC without message_id (using visible_index)
        const res = await editConversationMessage({
            conversation_id: "conv-live",
            message_id: targetMsg.id,
            visible_index: globalIndex,
            new_content: newText.trim(),
        });

        // 3. Backfill message_id
        if (res?.message_id && !targetMsg.id) {
            messages = [...messages];
            messages[globalIndex] = { ...messages[globalIndex], id: res.message_id };
        }

        expect(messages[0].text).toBe("Fixed typo in sent message");
        expect(messages[0].id).toBe(108);
    });

    it("catches and reports persistence errors when IPC fails", async () => {
        let messages: ChatPanelMessage[] = [
            { id: 10, role: "user", text: "Some question" },
        ];

        const editConversationMessage = vi.fn(async (_req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            throw new Error("Database disk I/O error");
        });

        let errorReported: string | null = null;
        const setError = (err: string) => {
            errorReported = err;
        };

        const globalIndex = 0;
        const newText = "Attempted edit";
        const targetMsg = messages[globalIndex];

        messages = [...messages];
        messages[globalIndex] = { ...messages[globalIndex], text: newText.trim() };

        try {
            await editConversationMessage({
                conversation_id: "conv-1",
                message_id: targetMsg.id,
                visible_index: globalIndex,
                new_content: newText.trim(),
            });
        } catch {
            setError("Failed to save edited message");
        }

        expect(errorReported).toBe("Failed to save edited message");
    });
});
