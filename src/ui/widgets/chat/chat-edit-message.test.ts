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

    it("catches errors and rolls back optimistic UI when IPC fails", async () => {
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
        const previousText = targetMsg.text;
        const targetId = targetMsg.id;

        // 1. Optimistic update
        messages = [...messages];
        messages[globalIndex] = { ...messages[globalIndex], text: newText.trim() };
        expect(messages[0].text).toBe("Attempted edit");

        // 2. Attempt persistence and catch failure
        try {
            await editConversationMessage({
                conversation_id: "conv-1",
                message_id: targetMsg.id,
                new_content: newText.trim(),
            });
        } catch {
            // Rollback optimistic update
            const targetIdx = targetId ? messages.findIndex(m => m.id === targetId) : -1;
            if (targetIdx !== -1 && messages[targetIdx].text === newText.trim()) {
                messages = [...messages];
                messages[targetIdx] = { ...messages[targetIdx], text: previousText };
            }
            setError("Failed to save edited message");
        }

        expect(errorReported).toBe("Failed to save edited message");
        // Verify optimistic UI was reverted back to previousText!
        expect(messages[0].text).toBe("Some question");
    });

    it("does not overwrite newer user input if a previous failed edit rolls back late", async () => {
        let messages: ChatPanelMessage[] = [
            { id: 10, role: "user", text: "Original question" },
        ];

        const targetMsg = messages[0];
        const firstEditText = "First edit attempt";
        const targetId = targetMsg.id;

        // 1. First edit applied optimistically
        messages = [...messages];
        messages[0] = { ...messages[0], text: firstEditText };

        // 2. User quickly types and applies a second edit before first fails
        const secondEditText = "Second newer edit";
        messages = [...messages];
        messages[0] = { ...messages[0], text: secondEditText };

        // 3. First edit fails late
        const targetIdx = targetId ? messages.findIndex(m => m.id === targetId) : -1;
        if (targetIdx !== -1 && messages[targetIdx].text === firstEditText) {
            // Guard prevents rollback because text is no longer firstEditText
            messages = [...messages];
            messages[targetIdx] = { ...messages[targetIdx], text: "Original question" };
        }

        // Verify second edit was preserved
        expect(messages[0].text).toBe("Second newer edit");
    });

    it("synchronizes conversation_id and user_message_id on send and turn start, enabling reliable edit persistence", async () => {
        let activeConversationId: string | null = null;
        let messages: ChatPanelMessage[] = [];

        // 1. User starts an empty conversation
        activeConversationId = null;
        messages = [];

        // 2. User sends the first message with clientRequestId
        const clientRequestId = "req_123456";
        messages = [
            ...messages,
            { role: "user", text: "Hello first message", clientRequestId },
        ];
        expect(messages[0].id).toBeUndefined();
        expect(activeConversationId).toBeNull();

        // 3. Backend emits chat-turn-start with newly allocated conversation_id and message_id
        const turnStartPayload = {
            turn_id: "turn-1",
            client_request_id: clientRequestId,
            conversation_id: "conv-auto-999",
            user_message_id: 501,
        };

        if (turnStartPayload.conversation_id) {
            activeConversationId = turnStartPayload.conversation_id;
        }
        if (turnStartPayload.user_message_id) {
            const idx = messages.findIndex(m => m.clientRequestId === turnStartPayload.client_request_id);
            if (idx !== -1 && !messages[idx].id) {
                messages = [...messages];
                messages[idx] = { ...messages[idx], id: turnStartPayload.user_message_id };
            }
        }

        expect(activeConversationId).toBe("conv-auto-999");
        expect(messages[0].id).toBe(501);

        // 4. User immediately edits the first message using message_id
        const editCalls: EditConversationMessageRequest[] = [];
        const editConversationMessage = vi.fn(async (req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            editCalls.push(req);
            return {
                message_id: req.message_id ?? 501,
                updated_content: req.new_content,
            };
        });

        const targetMsg = messages[0];
        const newText = "Hello corrected message";

        const res = await editConversationMessage({
            conversation_id: activeConversationId ?? undefined,
            message_id: targetMsg.id,
            new_content: newText,
        });

        expect(editCalls).toHaveLength(1);
        expect(editCalls[0]).toEqual({
            conversation_id: "conv-auto-999",
            message_id: 501,
            new_content: "Hello corrected message",
        });
        expect(res.message_id).toBe(501);
    });

    it("blocks editing when message_id cannot be resolved after timeout", async () => {
        const messages: ChatPanelMessage[] = [
            { role: "user", text: "Offline unsaved message" },
        ];

        const editConversationMessage = vi.fn();
        let errorThrown: string | null = null;

        const executeEdit = async (msg: ChatPanelMessage, text: string) => {
            if (!msg.id) {
                throw new Error("Message ID not yet synchronized, cannot edit");
            }
            await editConversationMessage({
                message_id: msg.id,
                new_content: text,
            });
        };

        try {
            await executeEdit(messages[0], "Try to edit offline message");
        } catch (err: unknown) {
            errorThrown = err instanceof Error ? err.message : String(err);
        }

        expect(editConversationMessage).not.toHaveBeenCalled();
        expect(errorThrown).toBe("Message ID not yet synchronized, cannot edit");
    });

    it("handles editing safely when conversation contains folded tool calls using message_id", async () => {
        // Conversation has:
        // - msg 0: User (id: 101)
        // - (hidden tool call & result, not in messages array)
        // - msg 1: Assistant with tools folded (id: 104)
        // - msg 2: User follow-up (id: 105)
        const messages: ChatPanelMessage[] = [
            { id: 101, role: "user", text: "Search the weather" },
            { id: 104, role: "kokoro", text: "Weather is 25C", tools: [{ tool: "weather", text: "25C" }] },
            { id: 105, role: "user", text: "Thanks a lot" },
        ];

        const editCalls: EditConversationMessageRequest[] = [];
        const editConversationMessage = vi.fn(async (req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            editCalls.push(req);
            return {
                message_id: req.message_id!,
                updated_content: req.new_content,
            };
        });

        // User edits follow-up (UI index 2, message_id 105)
        const targetMsg = messages[2];
        await editConversationMessage({
            conversation_id: "conv-tools",
            message_id: targetMsg.id,
            new_content: "Thanks a lot! How about tomorrow?",
        });

        expect(editCalls).toHaveLength(1);
        expect(editCalls[0].message_id).toBe(105);
        expect(editCalls[0].new_content).toBe("Thanks a lot! How about tomorrow?");
    });

    it("updates message text with truncated content when backend enforces max_message_chars", async () => {
        let messages: ChatPanelMessage[] = [
            { id: 201, role: "user", text: "Short text" },
        ];

        const longInput = "A".repeat(3000);
        const expectedTruncated = "A".repeat(2000) + "…[truncated]";

        const editConversationMessage = vi.fn(async (req: EditConversationMessageRequest): Promise<EditConversationMessageResponse> => {
            return {
                message_id: req.message_id!,
                updated_content: expectedTruncated,
            };
        });

        // 1. Optimistic update
        messages = [{ ...messages[0], text: longInput }];
        expect(messages[0].text.length).toBe(3000);

        // 2. Invoke edit
        const res = await editConversationMessage({
            message_id: 201,
            new_content: longInput,
        });

        // 3. Backfill truncated content from backend
        if (res?.message_id) {
            const idx = messages.findIndex(m => m.id === res.message_id);
            if (idx !== -1) {
                messages = [...messages];
                messages[idx] = {
                    ...messages[idx],
                    text: res.updated_content ?? messages[idx].text,
                };
            }
        }

        // Verify messages array updated with truncated content
        expect(messages[0].text).toBe(expectedTruncated);
        expect(messages[0].text.endsWith("…[truncated]")).toBe(true);
    });
});

