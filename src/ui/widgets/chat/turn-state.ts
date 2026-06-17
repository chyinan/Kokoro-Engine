import type { ToolTraceItem } from "@/lib/kokoro-bridge";
import {
    hasActiveKokoroBubble,
    hasVisibleAssistantContent,
    shouldRevealLiveTurnToolTrace,
} from "../chat-streaming-state";

export interface ChatPanelMessage {
    role: "user" | "kokoro" | "tool" | "context";
    text: string;
    images?: string[];
    translation?: string;
    translationPending?: boolean;
    isError?: boolean;
    tools?: ToolTraceItem[];
    capturedAt?: string;
    source?: string;
    turnId?: string;
}

export interface PendingTurnState {
    turnId: string;
    messageIndex: number | null;
    rawText: string;
    visibleTextStarted: boolean;
    translation?: string;
    translationPending: boolean;
    tools: ToolTraceItem[];
    pendingContext?: ChatPanelMessage;
}

export const stripStreamingMarkup = (text: string) =>
    text
        .replace(/\[ACTION:\w+\]\s*/g, "")
        .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
        .replace(/\[TRANSLATE:[^\]]*\]\s*/g, "")
        .replace(/\[\w+\|[^\]]*=[^\]]*\]\s*/g, "");

export const stripStoredMarkup = (text: string) =>
    stripStreamingMarkup(text)
        .replace(/\[EMOTION:[^\]]*\]/g, "")
        .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
        .replace(/\[TRANSLATE:[\s\S]*?\]/gi, "");

export const ensureTurnMessage = (messages: ChatPanelMessage[], turn: PendingTurnState) => {
    if (hasActiveKokoroBubble(messages, turn.messageIndex)) {
        return [...messages];
    }

    const next = [...messages];
    if (turn.pendingContext && !next.some(message => message.role === "context" && message.turnId === turn.turnId)) {
        next.push({
            ...turn.pendingContext,
            turnId: turn.turnId,
        });
    }
    next.push({
        role: "kokoro" as const,
        text: "",
        turnId: turn.turnId,
        ...(turn.tools.length > 0 ? { tools: [...turn.tools] } : {}),
    });
    turn.messageIndex = next.length - 1;
    return next;
};

export const updateTurnMessage = (
    messages: ChatPanelMessage[],
    turn: PendingTurnState,
    updater: (current: ChatPanelMessage) => ChatPanelMessage
) => {
    if (!hasActiveKokoroBubble(messages, turn.messageIndex)) {
        return messages;
    }

    const next = [...messages];
    next[turn.messageIndex!] = updater(next[turn.messageIndex!]);
    return next;
};

export function mergeToolTraceItems(existing: ReadonlyArray<ToolTraceItem>, incoming: ToolTraceItem): Array<ToolTraceItem> {
    if (incoming.approvalRequestId) {
        const targetIndex = existing.findIndex(tool => tool.approvalRequestId === incoming.approvalRequestId);
        if (targetIndex >= 0) {
            const next = [...existing];
            next[targetIndex] = incoming;
            return next;
        }
    }
    return [...existing, incoming];
}

export function buildToolTraceItem(event: {
    tool: string;
    tool_name?: string;
    tool_id?: string;
    source?: ToolTraceItem["source"];
    server_name?: string;
    needs_feedback?: boolean;
    permission_level?: ToolTraceItem["permissionLevel"];
    risk_tags?: ToolTraceItem["riskTags"];
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): ToolTraceItem {
    const baseTool = {
        tool: event.tool,
        toolName: event.tool_name ?? event.tool,
        toolId: event.tool_id,
        source: event.source,
        serverName: event.server_name,
        needsFeedback: event.needs_feedback,
        permissionLevel: event.permission_level,
        riskTags: event.risk_tags,
        approvalRequestId: event.approval_request_id,
        approvalStatus: event.approval_status,
    } satisfies Omit<ToolTraceItem, "text">;

    return event.result
        ? {
            ...baseTool,
            text: event.result.message,
            isError: false,
        }
        : {
            ...baseTool,
            text: event.error || "",
            isError: true,
            denyKind: event.deny_kind,
        };
}

export function getApprovalErrorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

export function getApprovalRequestId(tool: ToolTraceItem): string | null {
    return typeof tool.approvalRequestId === "string" && tool.approvalRequestId.length > 0
        ? tool.approvalRequestId
        : null;
}

function updateMessageTools(messages: Array<ChatPanelMessage>, globalIndex: number, updater: (tools: Array<ToolTraceItem>) => Array<ToolTraceItem>): Array<ChatPanelMessage> {
    if (globalIndex < 0 || globalIndex >= messages.length) {
        return messages;
    }
    const current = messages[globalIndex];
    const nextTools = updater(current.tools ? [...current.tools] : []);
    const next = [...messages];
    next[globalIndex] = {
        ...current,
        tools: nextTools.length > 0 ? nextTools : undefined,
    };
    return next;
}

export function removePendingApprovalHint(text: string): string {
    return text.replace(/\n等待用户审批后继续。$/, "");
}

export function createRejectedToolTrace(tool: ToolTraceItem): ToolTraceItem {
    return {
        ...tool,
        text: removePendingApprovalHint(tool.text),
        isError: true,
        approvalStatus: "rejected",
    };
}

export function createApprovedToolTrace(tool: ToolTraceItem): ToolTraceItem {
    return {
        ...tool,
        text: removePendingApprovalHint(tool.text),
        isError: false,
        approvalStatus: "approved",
    };
}

function getResolvedToolText(tool: ToolTraceItem, fallback: string): string {
    return fallback || removePendingApprovalHint(tool.text);
}

function isApprovalRequested(event: { approval_status?: ToolTraceItem["approvalStatus"] }): boolean {
    return event.approval_status === "requested";
}

function isApprovalResolved(event: { approval_status?: ToolTraceItem["approvalStatus"] }): boolean {
    return event.approval_status === "approved" || event.approval_status === "rejected";
}

function shouldKeepToolEntryVisible(_tool: ToolTraceItem): boolean {
    return true;
}

export function filterVisibleTools(tools: Array<ToolTraceItem>): Array<ToolTraceItem> {
    return tools.filter(shouldKeepToolEntryVisible);
}

export function normalizeToolList(tools: Array<ToolTraceItem>): Array<ToolTraceItem> {
    return filterVisibleTools(tools);
}

export function hasRenderableTurnContent(turn: PendingTurnState, text: string): boolean {
    return hasVisibleAssistantContent(text) || normalizeToolList(turn.tools).length > 0;
}

export function removeTurnContext(messages: Array<ChatPanelMessage>, turn: PendingTurnState): Array<ChatPanelMessage> {
    if (!turn.pendingContext) {
        return messages;
    }
    return messages.filter(message => !(message.role === "context" && message.turnId === turn.turnId));
}

export function removeTurnMessages(messages: Array<ChatPanelMessage>, turn: PendingTurnState): Array<ChatPanelMessage> {
    const withoutAssistant = hasActiveKokoroBubble(messages, turn.messageIndex)
        ? [...messages.slice(0, turn.messageIndex!), ...messages.slice(turn.messageIndex! + 1)]
        : messages;
    return removeTurnContext(withoutAssistant, turn);
}

export function mergeToolIntoTurn(turn: PendingTurnState, incoming: ToolTraceItem): void {
    turn.tools = normalizeToolList(mergeToolTraceItems(turn.tools, incoming));
}

export function updateTurnToolsInMessages(prev: Array<ChatPanelMessage>, turn: PendingTurnState, incoming: ToolTraceItem): Array<ChatPanelMessage> {
    if (!shouldRevealLiveTurnToolTrace({
        messages: prev,
        activeMessageIndex: turn.messageIndex,
        approvalStatus: incoming.approvalStatus,
    })) {
        return prev;
    }

    const ensured = ensureTurnMessage(prev, turn);
    return updateTurnMessage(ensured, turn, (current) => ({
        ...current,
        tools: normalizeToolList(mergeToolTraceItems(current.tools || [], incoming)),
    }));
}

function isToolApprovalPending(tool: ToolTraceItem): boolean {
    return tool.denyKind === "pending_approval" && tool.approvalStatus === "requested";
}

function findPendingToolIndex(message: ChatPanelMessage, approvalRequestId: string): number {
    return (message.tools || []).findIndex(tool => tool.approvalRequestId === approvalRequestId);
}

function replaceToolAtIndex(tools: Array<ToolTraceItem>, index: number, replacement: ToolTraceItem): Array<ToolTraceItem> {
    if (index < 0 || index >= tools.length) {
        return tools;
    }
    const next = [...tools];
    next[index] = replacement;
    return next;
}

function updatePendingToolStatus(messages: Array<ChatPanelMessage>, globalIndex: number, approvalRequestId: string, replacement: ToolTraceItem): Array<ChatPanelMessage> {
    return updateMessageTools(messages, globalIndex, (tools) => {
        const targetIndex = tools.findIndex(tool => tool.approvalRequestId === approvalRequestId);
        return targetIndex >= 0 ? replaceToolAtIndex(tools, targetIndex, replacement) : tools;
    });
}

function findToolMessageIndexByApprovalRequestId(messages: ReadonlyArray<ChatPanelMessage>, approvalRequestId: string): number {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
        const message = messages[index];
        if ((message.tools || []).some(tool => tool.approvalRequestId === approvalRequestId)) {
            return index;
        }
    }
    return -1;
}

function isKokoroMessage(message: ChatPanelMessage | undefined): boolean {
    return message?.role === "kokoro";
}

function shouldAppendEmptyKokoroBubble(messages: ReadonlyArray<ChatPanelMessage>): boolean {
    const last = messages[messages.length - 1];
    return !isKokoroMessage(last);
}

function appendPendingApprovalBubble(messages: Array<ChatPanelMessage>, tool: ToolTraceItem): Array<ChatPanelMessage> {
    if (shouldAppendEmptyKokoroBubble(messages)) {
        return [...messages, { role: "kokoro", text: "", tools: [tool] }];
    }
    const next = [...messages];
    const last = next[next.length - 1];
    next[next.length - 1] = {
        ...last,
        tools: normalizeToolList(mergeToolTraceItems(last.tools || [], tool)),
    };
    return next;
}

function setPendingApprovalOnLatestMessage(messages: Array<ChatPanelMessage>, tool: ToolTraceItem): Array<ChatPanelMessage> {
    const approvalRequestId = getApprovalRequestId(tool);
    if (approvalRequestId) {
        const existingIndex = findToolMessageIndexByApprovalRequestId(messages, approvalRequestId);
        if (existingIndex >= 0) {
            return updateMessageTools(messages, existingIndex, (tools) => normalizeToolList(mergeToolTraceItems(tools, tool)));
        }
    }
    return appendPendingApprovalBubble(messages, tool);
}

function isToolResolvedStatus(status: ToolTraceItem["approvalStatus"] | undefined): boolean {
    return status === "approved" || status === "rejected";
}

function getResolvedToolReplacement(event: { result?: { message: string }; error?: string; approval_status?: ToolTraceItem["approvalStatus"] }, current: ToolTraceItem): ToolTraceItem {
    if (event.approval_status === "approved") {
        if (event.error) {
            return {
                ...current,
                text: getResolvedToolText(current, event.error),
                isError: true,
                denyKind: "execution_error",
                approvalStatus: "approved",
            };
        }
        return {
            ...createApprovedToolTrace(current),
            text: getResolvedToolText(current, event.result?.message || current.text),
        };
    }
    return {
        ...createRejectedToolTrace(current),
        text: getResolvedToolText(current, event.error || current.text),
    };
}

function updateLatestPendingApproval(messages: Array<ChatPanelMessage>, event: {
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
    result?: { message: string };
    error?: string;
}): Array<ChatPanelMessage> {
    const approvalRequestId = typeof event.approval_request_id === "string" ? event.approval_request_id : null;
    if (!approvalRequestId) {
        return messages;
    }

    const targetMessageIndex = findToolMessageIndexByApprovalRequestId(messages, approvalRequestId);
    if (targetMessageIndex < 0) {
        return messages;
    }

    const targetMessage = messages[targetMessageIndex];
    const toolIndex = findPendingToolIndex(targetMessage, approvalRequestId);
    if (toolIndex < 0) {
        return messages;
    }

    const currentTool = targetMessage.tools?.[toolIndex];
    if (!currentTool) {
        return messages;
    }

    const replacement = getResolvedToolReplacement(event, currentTool);
    return updatePendingToolStatus(messages, targetMessageIndex, approvalRequestId, replacement);
}

function resolveApprovalEvent(messages: Array<ChatPanelMessage>, toolEntry: ToolTraceItem, event: {
    approval_status?: ToolTraceItem["approvalStatus"];
    approval_request_id?: string;
    result?: { message: string };
    error?: string;
}): Array<ChatPanelMessage> {
    if (isApprovalRequested(event)) {
        return setPendingApprovalOnLatestMessage(messages, toolEntry);
    }
    if (isApprovalResolved(event)) {
        return updateLatestPendingApproval(messages, event);
    }
    return messages;
}

function isLiveTurn(turn: PendingTurnState | null, turnId: string): turn is PendingTurnState {
    return Boolean(turn && turn.turnId === turnId);
}

function shouldHandleAsApprovalEvent(event: { approval_status?: ToolTraceItem["approvalStatus"] }): boolean {
    return event.approval_status === "requested" || event.approval_status === "approved" || event.approval_status === "rejected";
}

function toolRequiresApprovalAction(tool: ToolTraceItem): boolean {
    return isToolApprovalPending(tool);
}

export function canSubmitApproval(tool: ToolTraceItem): boolean {
    return toolRequiresApprovalAction(tool) && getApprovalRequestId(tool) !== null;
}

function clearApprovalWaitingSuffix(tool: ToolTraceItem): ToolTraceItem {
    return {
        ...tool,
        text: removePendingApprovalHint(tool.text),
    };
}

function setToolPendingResolutionState(messages: Array<ChatPanelMessage>, globalIndex: number, tool: ToolTraceItem, approvalStatus: "approved" | "rejected"): Array<ChatPanelMessage> {
    const approvalRequestId = getApprovalRequestId(tool);
    if (!approvalRequestId) {
        return messages;
    }
    const replacement = approvalStatus === "approved"
        ? createApprovedToolTrace(clearApprovalWaitingSuffix(tool))
        : createRejectedToolTrace(clearApprovalWaitingSuffix(tool));
    return updatePendingToolStatus(messages, globalIndex, approvalRequestId, replacement);
}

function getToolEntryFromEvent(event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): ToolTraceItem {
    return buildToolTraceItem(event);
}

function updateMessagesForToolEvent(messages: Array<ChatPanelMessage>, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatPanelMessage> {
    const toolEntry = getToolEntryFromEvent(event);
    if (shouldHandleAsApprovalEvent(event)) {
        return resolveApprovalEvent(messages, toolEntry, event);
    }
    return messages;
}

function updateMessagesForApprovalFallback(messages: Array<ChatPanelMessage>, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatPanelMessage> {
    return updateMessagesForToolEvent(messages, event);
}

function getResolvedApprovalStatus(tool: ToolTraceItem): ToolTraceItem["approvalStatus"] {
    return tool.approvalStatus;
}

function isToolAlreadyResolved(tool: ToolTraceItem): boolean {
    return isToolResolvedStatus(getResolvedApprovalStatus(tool));
}

export function updateApprovalToolLocally(messages: Array<ChatPanelMessage>, globalIndex: number, tool: ToolTraceItem, approvalStatus: "approved" | "rejected"): Array<ChatPanelMessage> {
    if (isToolAlreadyResolved(tool)) {
        return messages;
    }
    return setToolPendingResolutionState(messages, globalIndex, tool, approvalStatus);
}

function normalizeApprovalToolEntry(tool: ToolTraceItem): ToolTraceItem {
    return tool;
}

function normalizeApprovalEventToolEntry(event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): ToolTraceItem {
    return normalizeApprovalToolEntry(buildToolTraceItem(event));
}

function mergeApprovalEventIntoCurrentTurn(turn: PendingTurnState | null, eventTurnId: string, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): ToolTraceItem | null {
    if (!isLiveTurn(turn, eventTurnId)) {
        return null;
    }
    const toolEntry = normalizeApprovalEventToolEntry(event);
    mergeToolIntoTurn(turn, toolEntry);
    return toolEntry;
}

function updateMessagesAfterApprovalMerge(prev: Array<ChatPanelMessage>, turn: PendingTurnState | null, eventTurnId: string, toolEntry: ToolTraceItem | null, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatPanelMessage> {
    if (toolEntry && isLiveTurn(turn, eventTurnId)) {
        return updateTurnToolsInMessages(prev, turn, toolEntry);
    }
    return updateMessagesForApprovalFallback(prev, event);
}

function updateUiForToolEvent(prev: Array<ChatPanelMessage>, turn: PendingTurnState | null, eventTurnId: string, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatPanelMessage> {
    const toolEntry = mergeApprovalEventIntoCurrentTurn(turn, eventTurnId, event);
    return updateMessagesAfterApprovalMerge(prev, turn, eventTurnId, toolEntry, event);
}

export function getToolEventStateUpdate(event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}, turn: PendingTurnState | null, eventTurnId: string) {
    return (prev: Array<ChatPanelMessage>) => updateUiForToolEvent(prev, turn, eventTurnId, event);
}
