import type { ConversationMessage, ToolTraceItem } from "../../lib/kokoro-bridge";

export interface ChatHistoryMessage {
    role: "user" | "kokoro";
    text: string;
    images?: string[];
    translation?: string;
    tools?: ToolTraceItem[];
}

function parseDenyKind(meta: Record<string, unknown> | null, errorText: string, rawContent: string): ToolTraceItem["denyKind"] {
    const denyKindFromMetadata = meta?.deny_kind;
    if (
        denyKindFromMetadata === "pending_approval"
        || denyKindFromMetadata === "fail_closed"
        || denyKindFromMetadata === "policy_denied"
        || denyKindFromMetadata === "hook_denied"
        || denyKindFromMetadata === "execution_error"
    ) {
        return denyKindFromMetadata;
    }

    if (errorText.startsWith("Denied pending approval:")) return "pending_approval";
    if (errorText.startsWith("Denied by fail-closed policy:")) return "fail_closed";
    if (errorText.startsWith("Denied by policy:")) return "policy_denied";
    if (errorText.startsWith("Denied by hook:")) return "hook_denied";
    if (rawContent.startsWith("Error:")) return "execution_error";
    return undefined;
}

export function buildChatMessagesFromConversation(msgs: ConversationMessage[]): ChatHistoryMessage[] {
    const chatMsgs: ChatHistoryMessage[] = [];
    const turnToAssistantIndex = new Map<string, number>();
    const pendingToolsByTurn = new Map<string, ToolTraceItem[]>();
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
            const toolName = typeof meta?.tool_name === "string"
                ? meta.tool_name
                : typeof meta?.tool === "string"
                    ? meta.tool
                    : "tool";
            const errorText = m.content.startsWith("Error:") ? m.content.replace(/^Error:\s*/, "") : m.content;
            const denyKind = parseDenyKind(meta, errorText, m.content);
            const toolEntry: ToolTraceItem = {
                tool: toolName,
                toolName,
                toolId: typeof meta?.tool_id === "string" ? meta.tool_id : undefined,
                text: errorText,
                isError: m.content.startsWith("Error:"),
                source: meta?.source === "builtin" || meta?.source === "mcp" ? meta.source : undefined,
                serverName: typeof meta?.server_name === "string" ? meta.server_name : undefined,
                needsFeedback: typeof meta?.needs_feedback === "boolean" ? meta.needs_feedback : undefined,
                permissionLevel: meta?.permission_level === "safe" || meta?.permission_level === "elevated" ? meta.permission_level : undefined,
                riskTags: Array.isArray(meta?.risk_tags)
                    ? meta.risk_tags.filter(
                        (tag): tag is NonNullable<ToolTraceItem["riskTags"]>[number] =>
                            tag === "read" || tag === "write" || tag === "external" || tag === "sensitive"
                    )
                    : undefined,
                denyKind,
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
