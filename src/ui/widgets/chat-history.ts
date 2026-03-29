import type { ConversationMessage } from "../../lib/kokoro-bridge";

export interface ChatHistoryMessage {
    role: "user" | "kokoro";
    text: string;
    images?: string[];
    translation?: string;
    tools?: { text: string; isError?: boolean }[];
}

export function buildChatMessagesFromConversation(msgs: ConversationMessage[]): ChatHistoryMessage[] {
    const chatMsgs: ChatHistoryMessage[] = [];
    const turnToAssistantIndex = new Map<string, number>();
    const pendingToolsByTurn = new Map<string, { text: string; isError?: boolean }[]>();
    const pendingTurnOrder: string[] = [];

    for (const m of msgs) {
        let meta: Record<string, unknown> | null = null;
        if (m.metadata) {
            try {
                meta = JSON.parse(m.metadata) as Record<string, unknown>;
            } catch {
                meta = null;
            }
        }

        const technicalType = typeof meta?.type === "string" ? meta.type : undefined;
        const turnId = typeof meta?.turn_id === "string" ? meta.turn_id : undefined;

        if (m.role === "tool" || technicalType === "tool_result") {
            const toolName = typeof meta?.tool_name === "string" ? meta.tool_name : "tool";
            const toolEntry = {
                text: `${toolName}: ${m.content}`,
                isError: m.content.startsWith("Error:"),
            };
            const targetIndex = turnId ? turnToAssistantIndex.get(turnId) : undefined;

            if (targetIndex !== undefined) {
                const target = chatMsgs[targetIndex];
                chatMsgs[targetIndex] = {
                    ...target,
                    tools: [...(target.tools || []), toolEntry],
                };
            } else if (turnId) {
                if (!pendingToolsByTurn.has(turnId)) {
                    pendingTurnOrder.push(turnId);
                }
                pendingToolsByTurn.set(turnId, [
                    ...(pendingToolsByTurn.get(turnId) || []),
                    toolEntry,
                ]);
            }
            continue;
        }

        if (m.role !== "user") {
            if (technicalType === "assistant_tool_calls") {
                continue;
            }

            let translation: string | undefined;
            if (typeof meta?.translation === "string") {
                translation = meta.translation;
            }
            if (!translation) {
                const translateMatch = m.content.match(/\[TRANSLATE:\s*([\s\S]*?)\]/i);
                if (translateMatch) translation = translateMatch[1].trim();
            }

            const text = m.content
                .replace(/\[ACTION:\w+\]\s*/g, "")
                .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
                .replace(/\[EMOTION:[^\]]*\]/g, "")
                .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
                .replace(/\[TRANSLATE:[\s\S]*?\]/gi, "")
                .replace(/\[\w+\|[^\]]*=[^\]]*\]\s*/g, "")
                .trim();
            const pendingTools = turnId ? pendingToolsByTurn.get(turnId) : undefined;

            chatMsgs.push({
                role: "kokoro",
                text,
                translation,
                tools: pendingTools && pendingTools.length > 0 ? pendingTools : undefined,
            });

            if (turnId) {
                turnToAssistantIndex.set(turnId, chatMsgs.length - 1);
                pendingToolsByTurn.delete(turnId);
            }
            continue;
        }

        chatMsgs.push({ role: "user", text: m.content });
    }

    for (const turnId of pendingTurnOrder) {
        const tools = pendingToolsByTurn.get(turnId);
        if (!tools || tools.length === 0) continue;
        chatMsgs.push({
            role: "kokoro",
            text: "",
            tools,
        });
    }

    return chatMsgs;
}
