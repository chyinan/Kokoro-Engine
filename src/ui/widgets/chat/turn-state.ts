import type { ToolTraceItem } from "@/lib/kokoro-bridge";
import {
    hasActiveKokoroBubble,
    hasVisibleAssistantContent,
    shouldRevealLiveTurnToolTrace,
} from "../chat-streaming-state";

export interface ChatPanelMessage {
    id?: number;
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
    clientRequestId?: string;
}

export interface PendingTurnState {
    turnId: string;
    generation?: number;
    conversationId?: string | null;
    clientRequestId?: string | null;
    messageIndex: number | null;
    rawText: string;
    streamingVisibleText?: string;
    visibleTextStarted: boolean;
    translation?: string;
    translationPending: boolean;
    tools: ToolTraceItem[];
    pendingContext?: ChatPanelMessage;
    needsResync?: boolean;
}

function stripPairedEmphasisMarkers(
    text: string,
    marker: string,
    removeUnmatched: boolean,
): string {
    let result = text;
    let searchFrom = 0;

    while (searchFrom < result.length) {
        const open = result.indexOf(marker, searchFrom);
        if (open < 0) break;

        const contentStart = open + marker.length;
        if (marker === "*" && (
            result[open - 1] === "*"
            || result[open - 1] === "\\"
            || result[contentStart] === "*"
        )) {
            searchFrom = contentStart;
            continue;
        }
        const close = result.indexOf(marker, contentStart);
        if (close < 0) {
            if (!removeUnmatched) break;
            result = `${result.slice(0, open)}${result.slice(open + marker.length)}`;
            searchFrom = open;
            continue;
        }

        const content = result.slice(contentStart, close);
        if (!content.trim()) {
            searchFrom = contentStart;
            continue;
        }

        if (
            looksLikeRegexLiteral(result, open, close, marker)
            || looksLikeGlobPattern(result, open, close, marker)
        ) {
            searchFrom = contentStart;
            continue;
        }

        const previous = open > 0 ? result[open - 1] : undefined;
        const firstContent = content[0];
        const hasContentBoundaryWhitespace = /^\s|\s$/.test(content);
        const looksLikeWordOperator = Boolean(
            previous && firstContent && /[A-Za-z0-9]/.test(previous) && /[A-Za-z0-9]/.test(firstContent),
        );

        if (hasContentBoundaryWhitespace || looksLikeWordOperator) {
            searchFrom = contentStart;
            continue;
        }

        result = `${result.slice(0, close)}${result.slice(close + marker.length)}`;
        result = `${result.slice(0, open)}${result.slice(open + marker.length)}`;
        searchFrom = open;
    }

    return result;
}

function looksLikeRegexLiteral(text: string, open: number, close: number, marker: string): boolean {
    const openingSlash = text.lastIndexOf("/", open - 1);
    if (openingSlash < 0 || (openingSlash > 0 && text[openingSlash - 1] === "\\")) return false;

    const beforeSlash = openingSlash > 0 ? text[openingSlash - 1] : undefined;
    if (beforeSlash && !/[\s([{=:;,!?]/.test(beforeSlash)) return false;
    if (text[close + marker.length] !== "/") return false;

    return !text.slice(openingSlash + 1, close + marker.length).includes("\n");
}

function looksLikeGlobPattern(text: string, open: number, close: number, marker: string): boolean {
    const content = text.slice(open + marker.length, close);
    const afterClose = text[close + marker.length];
    return (
        content.startsWith(".")
        || content.endsWith(".")
        || /[/\\[\]{}]/.test(content)
        || text[open - 1] === "/"
        || text[open - 1] === "\\"
        || afterClose === "/"
        || afterClose === "\\"
        || (marker === "*" && (text[open - 1] === "." || afterClose === "."))
    );
}

function stripOutsideCodeSpans(text: string, transform: (segment: string) => string): string {
    let result = "";
    let segmentStart = 0;
    let index = 0;

    while (index < text.length) {
        if (text[index] !== "`" || (index > 0 && text[index - 1] === "\\")) {
            index += 1;
            continue;
        }

        let openingEnd = index + 1;
        while (openingEnd < text.length && text[openingEnd] === "`") {
            openingEnd += 1;
        }
        const delimiterLength = openingEnd - index;
        let cursor = openingEnd;
        let closingStart = -1;

        while (cursor < text.length) {
            if (text[cursor] !== "`" || (cursor > 0 && text[cursor - 1] === "\\")) {
                cursor += 1;
                continue;
            }

            let closingEnd = cursor + 1;
            while (closingEnd < text.length && text[closingEnd] === "`") {
                closingEnd += 1;
            }
            if (closingEnd - cursor === delimiterLength) {
                closingStart = cursor;
                break;
            }
            cursor = closingEnd;
        }

        if (closingStart < 0) {
            // An unmatched backtick is safer when left untouched: stars after it
            // may be part of an unfinished code span.
            return `${result}${transform(text.slice(segmentStart, index))}${text.slice(index)}`;
        }

        result += transform(text.slice(segmentStart, index));
        result += text.slice(index, closingStart + delimiterLength);
        index = closingStart + delimiterLength;
        segmentStart = index;
    }

    return result + transform(text.slice(segmentStart));
}

function stripPlainTextFormattingSegment(text: string, removeUnmatched: boolean): string {
    let result = text;
    for (const marker of ["\\*\\*", "**", "\\*", "*"]) {
        result = stripPairedEmphasisMarkers(result, marker, removeUnmatched);
    }
    return result;
}

export const stripPlainTextFormatting = (
    text: string,
    options: { removeUnmatched?: boolean } = {},
) => stripOutsideCodeSpans(
    text,
    segment => stripPlainTextFormattingSegment(segment, options.removeUnmatched ?? false),
);

function findUnmatchedMarker(text: string, marker: string): number | null {
    const positions: number[] = [];
    let searchFrom = 0;
    while (searchFrom < text.length) {
        const open = text.indexOf(marker, searchFrom);
        if (open < 0) break;

        const contentStart = open + marker.length;
        if (marker === "*" && (
            text[open - 1] === "*"
            || text[open - 1] === "\\"
            || text[contentStart] === "*"
        )) {
            searchFrom = contentStart;
            continue;
        }
        positions.push(open);
        searchFrom = contentStart;
    }
    return positions.length % 2 === 1 ? positions[positions.length - 1] ?? null : null;
}

function isPotentialStreamingEmphasisStart(text: string, open: number, marker: string): boolean {
    const previous = text[open - 1];
    const next = text[open + marker.length];
    if (marker === "*" && (
        (previous === undefined || /\s/.test(previous))
        && (next === undefined || /\s/.test(next))
    )) {
        return false;
    }

    // Keep multiplication and exponent-like identifiers visible while their
    // surrounding text is still streaming.
    if (previous && next && /[A-Za-z0-9]/.test(previous) && /[A-Za-z0-9]/.test(next)) {
        return false;
    }
    return true;
}

function findUnmatchedEmphasisStart(text: string): number | null {
    let offset = 0;
    let pending: number | null = null;
    stripOutsideCodeSpans(text, segment => {
        if (pending === null) {
            for (const marker of ["\\*\\*", "**", "\\*", "*"]) {
                const local = findUnmatchedMarker(segment, marker);
                if (local !== null && isPotentialStreamingEmphasisStart(segment, local, marker)) {
                    pending = offset + local;
                    break;
                }
            }
        }
        offset += segment.length;
        return segment;
    });
    return pending;
}

export const stripStreamingControlMarkup = (text: string) => text
    .replace(/\[ACTION:\w+\]\s*/g, "")
    .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
    .replace(/\[TRANSLATE:[^\]]*\]\s*/g, "")
    .replace(/\[\w+\|[^\]]*=[^\]]*\]\s*/g, "");

export const stripStreamingMarkup = (
    text: string,
    options: { removeUnmatched?: boolean } = {},
) => stripPlainTextFormatting(stripStreamingControlMarkup(text), options);

/**
 * Return only the stable prefix of a streaming response. An unmatched
 * emphasis delimiter is buffered instead of being deleted, so legal stars
 * such as multiplication operators remain visible and can be restored when
 * the complete response arrives.
 */
export const getStreamingVisibleText = (text: string): string => {
    const cleaned = stripStreamingMarkup(text);
    const pendingStart = findUnmatchedEmphasisStart(cleaned);
    return pendingStart === null ? cleaned : cleaned.slice(0, pendingStart);
};

export const stripStoredMarkup = (text: string) => stripPlainTextFormatting(
    stripStreamingMarkup(text)
        .replace(/\[EMOTION:[^\]]*\]/g, "")
        .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
        .replace(/\[TRANSLATE:[\s\S]*?\]/gi, ""),
);

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
        clientRequestId: turn.clientRequestId ?? undefined,
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
    if (error instanceof Error) {
        return error.message;
    }
    if (typeof error === "object" && error !== null) {
        const message = (error as { readonly message?: unknown }).message;
        if (typeof message === "string" && message.length > 0) {
            return message;
        }
        try {
            const serialized = JSON.stringify(error);
            if (typeof serialized === "string") return serialized;
        } catch {
            // Fall through to the generic string conversion for unusual host objects.
        }
    }
    return String(error);
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
