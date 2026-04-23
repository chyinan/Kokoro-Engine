import { useState, useRef, useEffect, useCallback, useDeferredValue, memo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Send, Trash2, AlertCircle, MessageCircle, ChevronLeft, ImagePlus, X, Mic, MicOff, History, Maximize2, Minimize2 } from "lucide-react";
import { streamChat, onChatTurnStart, onChatTurnDelta, onChatTurnFinish, onChatError, onChatWarning, onChatFailure, onChatTurnTranslation, clearHistory, uploadVisionImage, synthesize, onChatTurnTool, listConversations, loadConversation, onTelegramChatSync, deleteLastMessages, approveToolApproval, rejectToolApproval } from "../../lib/kokoro-bridge";
import type { FailureEvent, ToolTraceItem } from "../../lib/kokoro-bridge";
import { getLatestCameraFrame } from "../../lib/camera-frame-cache";
import { listen } from "@tauri-apps/api/event";
import { useVoiceInput, VoiceState, useTypingReveal, useWakeWord } from "../hooks";
import { useTranslation } from "react-i18next";
import ConversationSidebar from "./ConversationSidebar";
import { ChatMessage } from "./ChatMessage";
import { buildChatMessagesFromConversation } from "./chat-history";
import { getStreamingRevealText, hasActiveKokoroBubble, hasVisibleAssistantContent, shouldRenderTypingIndicator, shouldRevealLiveTurnToolTrace } from "./chat-streaming-state";

// ── Types ──────────────────────────────────────────────────
interface ChatMessage {
    role: "user" | "kokoro" | "tool";
    text: string;
    images?: string[];
    translation?: string;
    isError?: boolean;
    tools?: ToolTraceItem[];
}

interface PendingTurnState {
    turnId: string;
    messageIndex: number | null;
    rawText: string;
    visibleTextStarted: boolean;
    translation?: string;
    tools: ToolTraceItem[];
}

export type { ChatMessage as ChatPanelMessage };

const stripStreamingMarkup = (text: string) =>
    text
        .replace(/\[ACTION:\w+\]\s*/g, "")
        .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
        .replace(/\[TRANSLATE:[^\]]*\]\s*/g, "")
        .replace(/\[\w+\|[^\]]*=[^\]]*\]\s*/g, "");

const stripStoredMarkup = (text: string) =>
    stripStreamingMarkup(text)
        .replace(/\[EMOTION:[^\]]*\]/g, "")
        .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
        .replace(/\[TRANSLATE:[\s\S]*?\]/gi, "");

const ensureTurnMessage = (messages: ChatMessage[], turn: PendingTurnState) => {
    if (hasActiveKokoroBubble(messages, turn.messageIndex)) {
        return [...messages];
    }

    const next = [...messages, {
        role: "kokoro" as const,
        text: "",
        tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
    }];
    turn.messageIndex = next.length - 1;
    return next;
};

const updateTurnMessage = (
    messages: ChatMessage[],
    turn: PendingTurnState,
    updater: (current: ChatMessage) => ChatMessage
) => {
    if (!hasActiveKokoroBubble(messages, turn.messageIndex)) {
        return messages;
    }

    const next = [...messages];
    next[turn.messageIndex!] = updater(next[turn.messageIndex!]);
    return next;
};

function mergeToolTraceItems(existing: ReadonlyArray<ToolTraceItem>, incoming: ToolTraceItem): Array<ToolTraceItem> {
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

function buildToolTraceItem(event: {
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

function getApprovalErrorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

function getApprovalRequestId(tool: ToolTraceItem): string | null {
    return typeof tool.approvalRequestId === "string" && tool.approvalRequestId.length > 0
        ? tool.approvalRequestId
        : null;
}

function updateMessageTools(messages: Array<ChatMessage>, globalIndex: number, updater: (tools: Array<ToolTraceItem>) => Array<ToolTraceItem>): Array<ChatMessage> {
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

function removePendingApprovalHint(text: string): string {
    return text.replace(/\n等待用户审批后继续。$/, "");
}

function createRejectedToolTrace(tool: ToolTraceItem): ToolTraceItem {
    return {
        ...tool,
        text: removePendingApprovalHint(tool.text),
        isError: true,
        approvalStatus: "rejected",
    };
}

function createApprovedToolTrace(tool: ToolTraceItem): ToolTraceItem {
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

function filterVisibleTools(tools: Array<ToolTraceItem>): Array<ToolTraceItem> {
    return tools.filter(shouldKeepToolEntryVisible);
}

function normalizeToolList(tools: Array<ToolTraceItem>): Array<ToolTraceItem> {
    return filterVisibleTools(tools);
}

function mergeToolIntoTurn(turn: PendingTurnState, incoming: ToolTraceItem): void {
    turn.tools = normalizeToolList(mergeToolTraceItems(turn.tools, incoming));
}

function updateTurnToolsInMessages(prev: Array<ChatMessage>, turn: PendingTurnState, incoming: ToolTraceItem): Array<ChatMessage> {
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

function findPendingToolIndex(message: ChatMessage, approvalRequestId: string): number {
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

function updatePendingToolStatus(messages: Array<ChatMessage>, globalIndex: number, approvalRequestId: string, replacement: ToolTraceItem): Array<ChatMessage> {
    return updateMessageTools(messages, globalIndex, (tools) => {
        const targetIndex = tools.findIndex(tool => tool.approvalRequestId === approvalRequestId);
        return targetIndex >= 0 ? replaceToolAtIndex(tools, targetIndex, replacement) : tools;
    });
}

function findToolMessageIndexByApprovalRequestId(messages: ReadonlyArray<ChatMessage>, approvalRequestId: string): number {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
        const message = messages[index];
        if ((message.tools || []).some(tool => tool.approvalRequestId === approvalRequestId)) {
            return index;
        }
    }
    return -1;
}

function isKokoroMessage(message: ChatMessage | undefined): boolean {
    return message?.role === "kokoro";
}

function shouldAppendEmptyKokoroBubble(messages: ReadonlyArray<ChatMessage>): boolean {
    const last = messages[messages.length - 1];
    return !isKokoroMessage(last);
}

function appendPendingApprovalBubble(messages: Array<ChatMessage>, tool: ToolTraceItem): Array<ChatMessage> {
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

function setPendingApprovalOnLatestMessage(messages: Array<ChatMessage>, tool: ToolTraceItem): Array<ChatMessage> {
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

function updateLatestPendingApproval(messages: Array<ChatMessage>, event: {
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
    result?: { message: string };
    error?: string;
}): Array<ChatMessage> {
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

function resolveApprovalEvent(messages: Array<ChatMessage>, toolEntry: ToolTraceItem, event: {
    approval_status?: ToolTraceItem["approvalStatus"];
    approval_request_id?: string;
    result?: { message: string };
    error?: string;
}): Array<ChatMessage> {
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

function shouldLogToolEventError(event: { result?: { message: string }; error?: string }): boolean {
    return !event.result && Boolean(event.error);
}

function shouldLogToolEventSuccess(event: { result?: { message: string } }): boolean {
    return Boolean(event.result);
}

function getToolEventErrorMessage(event: { error?: string }): string {
    return event.error || "";
}

function getToolEventSuccessMessage(event: { result?: { message: string } }): string {
    return event.result?.message || "";
}

function shouldHandleAsApprovalEvent(event: { approval_status?: ToolTraceItem["approvalStatus"] }): boolean {
    return event.approval_status === "requested" || event.approval_status === "approved" || event.approval_status === "rejected";
}

function toolRequiresApprovalAction(tool: ToolTraceItem): boolean {
    return isToolApprovalPending(tool);
}

function canSubmitApproval(tool: ToolTraceItem): boolean {
    return toolRequiresApprovalAction(tool) && getApprovalRequestId(tool) !== null;
}

function clearApprovalWaitingSuffix(tool: ToolTraceItem): ToolTraceItem {
    return {
        ...tool,
        text: removePendingApprovalHint(tool.text),
    };
}

function setToolPendingResolutionState(messages: Array<ChatMessage>, globalIndex: number, tool: ToolTraceItem, approvalStatus: "approved" | "rejected"): Array<ChatMessage> {
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

function updateMessagesForToolEvent(messages: Array<ChatMessage>, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatMessage> {
    const toolEntry = getToolEntryFromEvent(event);
    if (shouldHandleAsApprovalEvent(event)) {
        return resolveApprovalEvent(messages, toolEntry, event);
    }
    return messages;
}

function updateMessagesForApprovalFallback(messages: Array<ChatMessage>, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatMessage> {
    return updateMessagesForToolEvent(messages, event);
}

function getResolvedApprovalStatus(tool: ToolTraceItem): ToolTraceItem["approvalStatus"] {
    return tool.approvalStatus;
}

function isToolAlreadyResolved(tool: ToolTraceItem): boolean {
    return isToolResolvedStatus(getResolvedApprovalStatus(tool));
}

function updateApprovalToolLocally(messages: Array<ChatMessage>, globalIndex: number, tool: ToolTraceItem, approvalStatus: "approved" | "rejected"): Array<ChatMessage> {
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

function updateMessagesAfterApprovalMerge(prev: Array<ChatMessage>, turn: PendingTurnState | null, eventTurnId: string, toolEntry: ToolTraceItem | null, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatMessage> {
    if (toolEntry && isLiveTurn(turn, eventTurnId)) {
        return updateTurnToolsInMessages(prev, turn, toolEntry);
    }
    return updateMessagesForApprovalFallback(prev, event);
}

function updateUiForToolEvent(prev: Array<ChatMessage>, turn: PendingTurnState | null, eventTurnId: string, event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}): Array<ChatMessage> {
    const toolEntry = mergeApprovalEventIntoCurrentTurn(turn, eventTurnId, event);
    return updateMessagesAfterApprovalMerge(prev, turn, eventTurnId, toolEntry, event);
}

function logToolEvent(event: { tool: string; result?: { message: string }; error?: string }): void {
    if (shouldLogToolEventSuccess(event)) {
        console.log(`[ToolCall] ${event.tool}: ${getToolEventSuccessMessage(event)}`);
        return;
    }
    if (shouldLogToolEventError(event)) {
        console.error(`[ToolCall] ${event.tool} failed: ${getToolEventErrorMessage(event)}`);
    }
}

function getToolEventStateUpdate(event: {
    tool: string;
    result?: { message: string };
    error?: string;
    deny_kind?: ToolTraceItem["denyKind"];
    approval_request_id?: string;
    approval_status?: ToolTraceItem["approvalStatus"];
}, turn: PendingTurnState | null, eventTurnId: string) {
    return (prev: Array<ChatMessage>) => updateUiForToolEvent(prev, turn, eventTurnId, event);
}


// ── Typing Indicator ───────────────────────────────────────
function TypingIndicator() {
    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="flex items-center gap-1.5 mr-auto px-4 py-3 rounded-lg rounded-tl-none bg-slate-900/50 border border-slate-700/50"
        >
            {[0, 1, 2].map(i => (
                <motion.div
                    key={i}
                    className="w-1.5 h-1.5 rounded-full bg-[var(--color-text-muted)]"
                    animate={{ opacity: [0.3, 1, 0.3], scale: [0.8, 1, 0.8] }}
                    transition={{ duration: 1.2, repeat: Infinity, delay: i * 0.2 }}
                />
            ))}
        </motion.div>
    );
}

// ── Error Toast ────────────────────────────────────────────
function ErrorToast({ message, onDismiss }: { message: string; onDismiss: () => void }) {
    useEffect(() => {
        const timer = setTimeout(onDismiss, 4000);
        return () => clearTimeout(timer);
    }, [onDismiss]);

    return (
        <motion.div
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 20 }}
            className="absolute top-2 right-2 z-[110] flex items-center gap-2 px-4 py-2 rounded-lg bg-red-900/80 border border-red-500/50 text-red-200 text-xs shadow-lg"
        >
            <AlertCircle size={14} strokeWidth={1.5} />
            {message}
        </motion.div>
    );
}

// ── Main Component ─────────────────────────────────────────
// ── MemoizedChatMessage wrapper ───────────────────────────
interface MemoizedChatMessageProps {
    message: ChatMessage;
    globalIndex: number;
    isStreaming: boolean;
    isTranslationExpanded: boolean;
    onToggleTranslation: (index: number) => void;
    onEdit: (index: number, newText: string) => void;
    onRegenerate: (index: number) => Promise<void>;
    onContinueFrom: (index: number) => Promise<void>;
    onApproveTool: (index: number, tool: ToolTraceItem) => Promise<void>;
    onRejectTool: (index: number, tool: ToolTraceItem) => Promise<void>;
}

function createToolActionHandler<TArgs extends Array<unknown>>(
    globalIndex: number,
    handler: (index: number, ...args: TArgs) => void | Promise<void>,
) {
    return (...args: TArgs) => handler(globalIndex, ...args);
}

const MemoizedChatMessage = memo(function MemoizedChatMessage({
    message, globalIndex, isStreaming, isTranslationExpanded,
    onToggleTranslation, onEdit, onRegenerate, onContinueFrom, onApproveTool, onRejectTool,
}: MemoizedChatMessageProps) {
    return (
        <ChatMessage
            message={message}
            index={globalIndex}
            isStreaming={isStreaming}
            isTranslationExpanded={isTranslationExpanded}
            onToggleTranslation={() => onToggleTranslation(globalIndex)}
            onEdit={(text) => onEdit(globalIndex, text)}
            onRegenerate={() => onRegenerate(globalIndex)}
            onContinueFrom={() => onContinueFrom(globalIndex)}
            onApproveTool={createToolActionHandler(globalIndex, onApproveTool)}
            onRejectTool={createToolActionHandler(globalIndex, onRejectTool)}
        />
    );
});

export default function ChatPanel() {
    const { t } = useTranslation();
    const [collapsed, setCollapsed] = useState(false);
    const [messages, setMessages] = useState<ChatMessage[]>([]);
    const deferredMessages = useDeferredValue(messages);
    const [visibleCount, setVisibleCount] = useState(20);
    const [input, setInput] = useState("");
    const [expandedInput, setExpandedInput] = useState(false);
    const expandedTextareaRef = useRef<HTMLTextAreaElement>(null);
    const [isStreaming, setIsStreaming] = useState(false);
    const isStreamingRef = useRef(false);
    const messagesRef = useRef<ChatMessage[]>([]);
    const [isThinking, setIsThinking] = useState(false);

    // Per-message translation expand state (set of message indices)
    const [expandedTranslations, setExpandedTranslations] = useState<Set<number>>(new Set());

    const startStreaming = useCallback(() => {
        isStreamingRef.current = true;
        setIsStreaming(true);
    }, []);
    const stopStreaming = useCallback(() => {
        isStreamingRef.current = false;
        setIsStreaming(false);
    }, []);

    // Raw (unfiltered) full response text — accumulated from all deltas
    const rawResponseRef = useRef("");
    const currentTurnRef = useRef<PendingTurnState | null>(null);

    // Typing reveal: per-character animation
    const { pushDelta, flush: flushReveal, reset: resetReveal } = useTypingReveal({
        active: isStreaming,
        onReveal: (visibleText: string) => {
            setMessages(prev => {
                const activeIndex = currentTurnRef.current?.messageIndex;
                if (activeIndex !== null && activeIndex !== undefined && hasActiveKokoroBubble(prev, activeIndex) && isStreamingRef.current) {
                    const next = [...prev];
                    next[activeIndex] = { ...next[activeIndex], text: visibleText };
                    return next;
                }
                return prev;
            });
        },
    });
    const [error, setError] = useState<string | null>(null);
    const [unreadCount, setUnreadCount] = useState(0);
    const messagesEndRef = useRef<HTMLDivElement>(null);
    const messagesContainerRef = useRef<HTMLDivElement>(null);
    const userScrolledRef = useRef(false);
    const isProgrammaticScrollRef = useRef(false);
    const fileInputRef = useRef<HTMLInputElement>(null);
    // Store last failed request for retry
    const lastFailedRequestRef = useRef<{ message: string; images?: string[]; allowImageGen?: boolean } | null>(null);

    // Vision Mode
    const [visionEnabled, setVisionEnabled] = useState(() => localStorage.getItem("kokoro_vision_enabled") === "true");
    const [cameraEnabled, setCameraEnabled] = useState(() => {
        try { return JSON.parse(localStorage.getItem("kokoro_vision_config") || "{}").camera_enabled === true; } catch { return false; }
    });
    const [pendingImages, setPendingImages] = useState<string[]>([]);
    const [isUploading, setIsUploading] = useState(false);

    // 对话历史侧边栏
    const [sidebarOpen, setSidebarOpen] = useState(false);

    // 自动恢复最近对话
    useEffect(() => {
        const characterId = localStorage.getItem("kokoro_active_character_id") || "default";
        listConversations(characterId).then(convs => {
            if (convs.length > 0) {
                loadConversation(convs[0].id).then(loaded => {
                    setMessages(buildChatMessagesFromConversation(loaded.messages));
                    setExpandedTranslations(new Set()); // Reset translation expand state on conversation load
                }).catch(err => console.error("[ChatPanel] Failed to restore conversation:", err));
            }
        }).catch(() => { /* backend not ready */ });
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // STT (Speech-to-Text) — Advanced VAD Mode
    const [sttEnabled, setSttEnabled] = useState(() => localStorage.getItem("kokoro_stt_enabled") === "true");
    const [sttAutoSend, setSttAutoSend] = useState(() => localStorage.getItem("kokoro_stt_auto_send") === "true");
    const [continuousListening, setContinuousListening] = useState(
        () => localStorage.getItem("kokoro_stt_continuous_listening") === "true"
    );

    useEffect(() => {
        const syncSttSettings = () => {
            setSttEnabled(localStorage.getItem("kokoro_stt_enabled") === "true");
            setSttAutoSend(localStorage.getItem("kokoro_stt_auto_send") === "true");
            setContinuousListening(localStorage.getItem("kokoro_stt_continuous_listening") === "true");
            setWakeWordEnabled(localStorage.getItem("kokoro_wake_word_enabled") === "true");
            setWakeWord(localStorage.getItem("kokoro_wake_word") || "");
        };
        window.addEventListener("kokoro-stt-settings-changed", syncSttSettings);
        window.addEventListener("storage", syncSttSettings);
        window.addEventListener("focus", syncSttSettings);
        return () => {
            window.removeEventListener("kokoro-stt-settings-changed", syncSttSettings);
            window.removeEventListener("storage", syncSttSettings);
            window.removeEventListener("focus", syncSttSettings);
        };
    }, []);

    const handleTranscription = useCallback((text: string) => {
        const trimmed = text.trim();
        if (!trimmed) return;

        if (sttAutoSend) {
            // Auto-send: inject directly into chat
            setInput("");
            setMessages(prev => [...prev, { role: "user", text: trimmed }]);
            startStreaming();
            setIsThinking(true);
            userScrolledRef.current = false;

            const allowImageGen = (() => {
                try {
                    const bgConfig = JSON.parse(localStorage.getItem("kokoro_bg_config") || "{}");
                    return bgConfig.mode === "generated";
                } catch { return false; }
            })();

            streamChat({
                message: trimmed,
                allow_image_gen: allowImageGen,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
            }).catch(err => {
                stopStreaming();
                setIsThinking(false);
                setError(err instanceof Error ? err.message : String(err));
            });
        } else {
            // Fill input box for user review
            setInput(trimmed);
        }
    }, [sttAutoSend, startStreaming, stopStreaming]);

    const { state: voiceState, volume: micVolume, partialText: sttPartialText, start: startVoice, stop: stopVoice } = useVoiceInput(handleTranscription);

    // Refs to avoid stale closures in the voice-interrupt-stt listener
    const startVoiceRef = useRef(startVoice);
    const sttAutoSendRef = useRef(sttAutoSend);
    const sttEnabledRef = useRef(sttEnabled);
    useEffect(() => { startVoiceRef.current = startVoice; }, [startVoice]);
    useEffect(() => { sttAutoSendRef.current = sttAutoSend; }, [sttAutoSend]);
    useEffect(() => { sttEnabledRef.current = sttEnabled; }, [sttEnabled]);

    // Wake word detection — starts main STT when keyword is heard
    const [wakeWordEnabled, setWakeWordEnabled] = useState(() => localStorage.getItem("kokoro_wake_word_enabled") === "true");
    const [wakeWord, setWakeWord] = useState(() => localStorage.getItem("kokoro_wake_word") || "");
    useWakeWord({
        enabled:
            sttEnabled &&
            voiceState === VoiceState.Idle &&
            (continuousListening || (wakeWordEnabled && !!wakeWord)),
        mode: continuousListening ? "speech" : "wake_word",
        wakeWord: continuousListening ? "" : wakeWord,
        onWakeWordDetected: useCallback((text?: string) => {
            if (continuousListening) {
                if (text?.trim()) {
                    handleTranscription(text);
                }
                return;
            }
            startVoice({ autoStopOnSilence: true });
        }, [continuousListening, handleTranscription, startVoice]),
    });

    // Effect: Sync partial STT text to input box for real-time feedback
    useEffect(() => {
        if (voiceState === VoiceState.Listening && sttPartialText) {
            // If auto-send is OFF, we just show the text in the box so user can edit later
            if (!sttAutoSend) {
                setInput(sttPartialText);
            }
            // If auto-send is ON, we usually wait for finalization to send.
            // But we could show a preview? For now, let's keep it simple:
            // Only fill input if NOT auto-sending. 
            // (If auto-sending, the text appears in chat history immediately upon finish).
            // Actually, showing it in input box is good feedback even for auto-send (it enters chat on stop).
            // But valid auto-send logic often clears input.
            // Let's stick to: Always show in input box while speaking.
            // When "Final" fires, if AutoSend -> Clear Input & Send. If Not -> Leave in Input.
            setInput(sttPartialText);
        }
    }, [sttPartialText, voiceState, sttAutoSend]);

    // Sync vision state when localStorage changes (from Settings panel)
    useEffect(() => {
        const checkVision = () => {
            const nextVisionEnabled = localStorage.getItem("kokoro_vision_enabled") === "true";
            setVisionEnabled(nextVisionEnabled);
            if (!nextVisionEnabled) setPendingImages([]);
            try {
                const cfg = JSON.parse(localStorage.getItem("kokoro_vision_config") || "{}");
                setCameraEnabled(cfg.camera_enabled === true);
            } catch { /* ignore */ }
        };
        window.addEventListener("kokoro-vision-settings-changed", checkVision);
        window.addEventListener("storage", checkVision);
        // Also poll on focus since Tauri doesn't fire storage events within same webview
        window.addEventListener("focus", checkVision);
        return () => {
            window.removeEventListener("kokoro-vision-settings-changed", checkVision);
            window.removeEventListener("storage", checkVision);
            window.removeEventListener("focus", checkVision);
        };
    }, []);

    // ── Auto-scroll ────────────────────────────────────────
    const scrollToBottom = useCallback(() => {
        if (!userScrolledRef.current) {
            const container = messagesContainerRef.current;
            if (!container) return;
            isProgrammaticScrollRef.current = true;
            container.scrollTop = container.scrollHeight;
            setTimeout(() => { isProgrammaticScrollRef.current = false; }, 50);
        }
    }, []);

    // Only fire after deferredMessages — DOM is actually updated at this point.
    // Firing on `messages` scrolls to the old DOM height (before new bubble renders).
    useEffect(scrollToBottom, [deferredMessages, scrollToBottom]);

    const handleScroll = useCallback(() => {
        // Ignore scroll events triggered by our own scrollToBottom
        if (isProgrammaticScrollRef.current) return;
        const container = messagesContainerRef.current;
        if (!container) return;
        const atBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 40;
        userScrolledRef.current = !atBottom;
        // Load more messages when scrolled near top
        if (container.scrollTop < 100) {
            setVisibleCount(prev => prev + 20);
        }
    }, []);

    // Track unread messages while collapsed
    useEffect(() => {
        if (collapsed && messages.length > 0) {
            const last = messages[messages.length - 1];
            if (last.role === "kokoro") {
                setUnreadCount(prev => prev + 1);
            }
        }
    // Only fire when a new message arrives, not when collapsed state changes
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [messages.length]);

    // Sync messages ref for use in event callbacks (avoids stale closure)
    useEffect(() => {
        messagesRef.current = messages;
    }, [messages]);

    // ── Chat event listeners ───────────────────────────────
    useEffect(() => {
        let aborted = false;
        const cleanups: (() => void)[] = [];

        const setup = async () => {
            // Listen for pet window sending a message — start streaming in main window too
            const unPetChat = await listen<{ message: string }>("pet-chat-start", (event) => {
                if (aborted) return;
                const text = event.payload.message;
                rawResponseRef.current = "";
                currentTurnRef.current = null;
                resetReveal();
                setMessages(prev => [...prev, { role: "user", text }]);
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;
            });
            if (aborted) { unPetChat(); return; }
            cleanups.push(unPetChat);

            const unTurnStart = await onChatTurnStart(({ turn_id }) => {
                if (aborted) return;
                currentTurnRef.current = {
                    turnId: turn_id,
                    messageIndex: null,
                    rawText: "",
                    visibleTextStarted: false,
                    translation: undefined,
                    tools: [],
                };
                rawResponseRef.current = "";
            });
            if (aborted) { unTurnStart(); return; }
            cleanups.push(unTurnStart);

            const unDelta = await onChatTurnDelta(({ turn_id, delta: rawDelta }) => {
                if (aborted || !isStreamingRef.current) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;

                const delta = stripStreamingMarkup(rawDelta);
                if (!delta) return;

                turn.rawText += delta;
                rawResponseRef.current = turn.rawText;

                const revealText = getStreamingRevealText({
                    accumulatedText: turn.rawText,
                    delta,
                    hasVisibleTextStarted: turn.visibleTextStarted,
                });
                if (!revealText) return;

                setIsThinking(false);
                if (!turn.visibleTextStarted) {
                    turn.visibleTextStarted = true;
                    setMessages(prev => ensureTurnMessage(prev, turn));
                }

                pushDelta(revealText);
            });
            if (aborted) { unDelta(); return; }
            cleanups.push(unDelta);

            const unTranslation = await onChatTurnTranslation(({ turn_id, translation }) => {
                if (aborted) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;
                turn.translation = translation;
            });
            if (aborted) { unTranslation(); return; }
            cleanups.push(unTranslation);

            const unDone = await onChatTurnFinish(({ turn_id, status }) => {
                if (aborted) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;

                flushReveal();
                stopStreaming();
                setIsThinking(false);
                userScrolledRef.current = false;

                const fullText = turn.rawText;
                rawResponseRef.current = fullText;
                const cleanText = stripStoredMarkup(fullText);

                setMessages(prev => {
                    const hasContent = hasVisibleAssistantContent(cleanText) || turn.tools.length > 0;

                    if (hasActiveKokoroBubble(prev, turn.messageIndex)) {
                        if (!hasContent && status === "error") {
                            return [...prev.slice(0, turn.messageIndex!), ...prev.slice(turn.messageIndex! + 1)];
                        }

                        return updateTurnMessage(prev, turn, (current) => ({
                            ...current,
                            text: cleanText,
                            translation: turn.translation,
                            tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
                        }));
                    }

                    if (hasContent) {
                        return [...prev, {
                            role: "kokoro",
                            text: cleanText,
                            translation: turn.translation,
                            tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
                        }];
                    }

                    return prev;
                });

                currentTurnRef.current = null;

                if (localStorage.getItem("kokoro_tts_enabled") === "true" && cleanText.trim()) {
                    console.log("[TTS] Auto-speak triggered, text length:", cleanText.length);
                    synthesize(cleanText.trim(), {
                        provider_id: localStorage.getItem("kokoro_tts_provider") || undefined,
                        voice: localStorage.getItem("kokoro_tts_voice") || undefined,
                        speed: parseFloat(localStorage.getItem("kokoro_tts_speed") || "1.0"),
                        pitch: parseFloat(localStorage.getItem("kokoro_tts_pitch") || "1.0"),
                    }).catch(err => console.error("[TTS] Auto-speak failed:", err));
                }
            });
            if (aborted) { unDone(); return; }
            cleanups.push(unDone);

            const unFailure = await onChatFailure((failure: FailureEvent) => {
                if (aborted) return;
                stopStreaming();
                setIsThinking(false);
                const suffix = failure.stage ? ` (${failure.stage})` : "";
                setError(`${failure.message}${suffix}`);
                currentTurnRef.current = null;
            });
            if (aborted) { unFailure(); return; }
            cleanups.push(unFailure);

            const unError = await onChatError((err: string) => {
                if (aborted) return;
                stopStreaming();
                setIsThinking(false);
                setError(err);
                currentTurnRef.current = null;
            });
            if (aborted) { unError(); return; }
            cleanups.push(unError);

            const unWarning = await onChatWarning((warning: string) => {
                if (aborted) return;
                setError(warning);
            });
            if (aborted) { unWarning(); return; }
            cleanups.push(unWarning);

            const unToolResult = await onChatTurnTool((event) => {
                if (aborted) return;
                logToolEvent(event);
                const turn = currentTurnRef.current;
                setMessages(prev => getToolEventStateUpdate(event, turn, event.turn_id)(prev));
            });
            if (aborted) { unToolResult(); return; }
            cleanups.push(unToolResult);

            // Telegram chat sync — show messages from Telegram bot in desktop UI
            const unTelegramSync = await onTelegramChatSync((data) => {
                if (aborted) return;
                if (data.role === "user") {
                    setMessages(prev => [...prev, { role: "user", text: data.text }]);
                } else {
                    setMessages(prev => [...prev, { role: "kokoro", text: data.text, translation: data.translation }]);
                }
            });
            if (aborted) { unTelegramSync(); return; }
            cleanups.push(unTelegramSync);

            // Interaction reactions (touch/click on Live2D model) handled via auto-generated LLM prompt in interaction-service.ts
            // We no longer listen here to avoid double-handling or showing hardcoded lines.

            // Listen for proactive triggers from backend (heartbeat)
            const unProactive = await listen<any>("proactive-trigger", (event) => {
                if (aborted || isStreamingRef.current) return;
                console.log("[ChatPanel] Proactive trigger:", event.payload);

                const { instruction } = event.payload;

                // Start streaming — compose_prompt() handles full context (system prompt, memory, emotion, history, language)
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;
                resetReveal();
                rawResponseRef.current = "";
                currentTurnRef.current = null;

                streamChat({
                    message: instruction,
                    hidden: true,
                    character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
                }).catch(err => {
                    stopStreaming();
                    setIsThinking(false);
                    setError(err instanceof Error ? err.message : String(err));
                    currentTurnRef.current = null;
                    // Remove the empty placeholder if one was created by delta handler
                    setMessages(prev => {
                        const last = prev[prev.length - 1];
                        if (last && last.role === "kokoro" && !last.text) {
                            return prev.slice(0, -1);
                        }
                        return prev;
                    });
                });
            });
            cleanups.push(() => unProactive());

            // Listen for interaction triggers (touch/click on Live2D model)
            // interaction-service already calls streamChat, we just need to prepare ChatPanel for receiving deltas
            const unInteraction = await listen<any>("interaction-trigger", () => {
                if (aborted || isStreamingRef.current) return;

                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;
                resetReveal();
                rawResponseRef.current = "";
                currentTurnRef.current = null;
            });
            cleanups.push(() => unInteraction());

            // Listen for voice-interrupt-stt: when TTS is interrupted by voice, auto-start STT
            const unVoiceInterruptStt = await listen<any>("voice-interrupt-stt", () => {
                if (aborted || isStreamingRef.current) return;
                if (!sttEnabledRef.current || !sttAutoSendRef.current) return;
                console.log("[ChatPanel] Voice interrupt → starting STT");
                startVoiceRef.current({ autoStopOnSilence: true });
            });
            if (aborted) { unVoiceInterruptStt(); return; }
            cleanups.push(() => unVoiceInterruptStt());
        };

        setup();
        return () => {
            aborted = true;
            cleanups.forEach(fn => fn());
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // ── Send message ───────────────────────────────────────
    const handleSend = async (e?: React.FormEvent) => {
        e?.preventDefault();
        const trimmed = input.trim();
        const messageImages = visionEnabled ? [...pendingImages] : [];
        if ((!trimmed && messageImages.length === 0) || isStreaming) return;

        setMessages(prev => [...prev, { role: "user", text: trimmed, images: messageImages.length > 0 ? messageImages : undefined }]);
        const cameraFrame = visionEnabled ? getLatestCameraFrame() : null;
        const imagesToSend = cameraFrame ? [...messageImages, cameraFrame] : messageImages;
        setInput("");
        setPendingImages([]);
        startStreaming();
        setIsThinking(true);
        userScrolledRef.current = false;
        // Lock out handleScroll until deferredMessages DOM update settles (~200ms)
        isProgrammaticScrollRef.current = true;
        setTimeout(() => { isProgrammaticScrollRef.current = false; }, 200);
        resetReveal();
        rawResponseRef.current = "";
        currentTurnRef.current = null;

        // Check if background mode is "generated"
        let allowImageGen = false;
        try {
            const bgConfig = JSON.parse(localStorage.getItem("kokoro_bg_config") || "{}");
            allowImageGen = bgConfig.mode === "generated";
        } catch { /* ignore */ }

        try {
            await streamChat({
                message: trimmed || "(image attached)",
                allow_image_gen: allowImageGen,
                images: imagesToSend.length > 0 ? imagesToSend : undefined,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
            });
        } catch (err) {
            stopStreaming();
            currentTurnRef.current = null;
            setIsThinking(false);
            setError(err instanceof Error ? err.message : String(err));

            // Save failed request for retry
            lastFailedRequestRef.current = { message: trimmed || "(image attached)", images: imagesToSend.length > 0 ? imagesToSend : undefined, allowImageGen };

            setTimeout(() => {
                setMessages(prev => [...prev, {
                    role: "kokoro",
                    text: t("chat.errors.connection_error"),
                    isError: true,
                }]);
            }, 500);
        }
    };

    // ── Image upload ───────────────────────────────────────
    const handleImageSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
        if (!visionEnabled) return;
        const file = e.target.files?.[0];
        if (!file) return;

        // Validate size (5MB)
        if (file.size > 5 * 1024 * 1024) {
            setError(t("chat.errors.image_too_large"));
            return;
        }

        // Validate type
        if (!file.type.startsWith("image/")) {
            setError(t("chat.errors.only_images"));
            return;
        }

        setIsUploading(true);
        try {
            const buffer = await file.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));
            const url = await uploadVisionImage(bytes, file.name);
            setPendingImages(prev => [...prev, url]);
        } catch (err) {
            setError(err instanceof Error ? err.message : t("chat.errors.upload_failed"));
        } finally {
            setIsUploading(false);
            // Reset file input so same file can be selected again
            if (fileInputRef.current) fileInputRef.current.value = "";
        }
    };

    const removePendingImage = (index: number) => {
        setPendingImages(prev => prev.filter((_, i) => i !== index));
    };

    // ── Clipboard paste image ────────────────────────────────
    const handlePaste = async (e: React.ClipboardEvent) => {
        if (!visionEnabled) return;
        const items = Array.from(e.clipboardData.items);
        const imageItem = items.find(item => item.type.startsWith("image/"));
        if (!imageItem) return;

        e.preventDefault();
        const file = imageItem.getAsFile();
        if (!file) return;

        if (file.size > 5 * 1024 * 1024) {
            setError(t("chat.errors.image_too_large"));
            return;
        }

        setIsUploading(true);
        try {
            const buffer = await file.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));
            const filename = `paste_${Date.now()}.png`;
            const url = await uploadVisionImage(bytes, filename);
            setPendingImages(prev => [...prev, url]);
        } catch (err) {
            setError(err instanceof Error ? err.message : t("chat.errors.upload_failed"));
        } finally {
            setIsUploading(false);
        }
    };

    // ── STT: Advanced VAD Microphone toggle ─────────────────
    const handleMicToggle = useCallback(() => {
        if (voiceState === VoiceState.Idle) {
            startVoice({ autoStopOnSilence: true });
        } else {
            stopVoice();
        }
    }, [voiceState, startVoice, stopVoice]);

    // ── Clear history ──────────────────────────────────────
    const handleClear = async () => {
        try {
            await clearHistory();
        } catch {
            // Backend might not be ready
        }
        setMessages([]);
    };

    // ── Stable message action callbacks ───────────────────
    const onToggleTranslation = useCallback((globalIndex: number) => {
        setExpandedTranslations(prev => {
            const next = new Set(prev);
            if (next.has(globalIndex)) next.delete(globalIndex);
            else next.add(globalIndex);
            return next;
        });
    }, []);

    const onEdit = useCallback((globalIndex: number, newText: string) => {
        setMessages(prev => {
            const updated = [...prev];
            updated[globalIndex] = { ...updated[globalIndex], text: newText };
            return updated;
        });
    }, []);

    const onRegenerate = useCallback(async (globalIndex: number) => {
        const msgs = messagesRef.current;
        const lastUserIndex = msgs.slice(0, globalIndex).reverse().findIndex(m => m.role === "user");
        if (lastUserIndex === -1) return;
        const userMsgIndex = globalIndex - 1 - lastUserIndex;
        const userMsg = msgs[userMsgIndex];

        const messagesToDelete = msgs.length - globalIndex;

        try {
            // 先删除数据库，再更新 UI，避免竞态条件
            await deleteLastMessages(messagesToDelete);
        } catch (e) {
            console.error("[ChatPanel] Failed to delete messages:", e);
        }
        setMessages(prev => prev.slice(0, globalIndex));

        startStreaming();
        setIsThinking(true);
        userScrolledRef.current = false;
        resetReveal();
        rawResponseRef.current = "";
        currentTurnRef.current = null;

        const allowImageGen = (() => {
            try {
                const bgConfig = JSON.parse(localStorage.getItem("kokoro_bg_config") || "{}");
                return bgConfig.mode === "generated";
            } catch { return false; }
        })();

        streamChat({
            message: userMsg.text,
            images: userMsg.images,
            allow_image_gen: allowImageGen,
            character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
        }).catch(err => {
            stopStreaming();
            setIsThinking(false);
            setError(err instanceof Error ? err.message : String(err));
        });
    }, [startStreaming, stopStreaming, resetReveal, setError]);

    const onContinueFrom = useCallback(async (globalIndex: number) => {
        const msgs = messagesRef.current;
        const messagesToDelete = msgs.length - globalIndex - 1;
        if (messagesToDelete > 0) {
            try {
                // 先删除数据库，再更新 UI，避免竞态条件
                await deleteLastMessages(messagesToDelete);
                setMessages(prev => prev.slice(0, globalIndex + 1));
            } catch (e) {
                console.error("[ChatPanel] Failed to delete messages:", e);
            }
        }
    }, []);

    const onApproveTool = useCallback(async (globalIndex: number, tool: ToolTraceItem) => {
        if (!canSubmitApproval(tool)) {
            return;
        }
        const approvalRequestId = getApprovalRequestId(tool);
        if (!approvalRequestId) {
            return;
        }
        try {
            await approveToolApproval(approvalRequestId);
            setMessages(prev => updateApprovalToolLocally(prev, globalIndex, tool, "approved"));
        } catch (error) {
            setError(`审批通过失败: ${getApprovalErrorMessage(error)}`);
        }
    }, []);

    const onRejectTool = useCallback(async (globalIndex: number, tool: ToolTraceItem) => {
        if (!canSubmitApproval(tool)) {
            return;
        }
        const approvalRequestId = getApprovalRequestId(tool);
        if (!approvalRequestId) {
            return;
        }
        try {
            await rejectToolApproval(approvalRequestId, null);
            setMessages(prev => updateApprovalToolLocally(prev, globalIndex, tool, "rejected"));
        } catch (error) {
            setError(`审批拒绝失败: ${getApprovalErrorMessage(error)}`);
        }
    }, []);

    // ── Expand handler ─────────────────────────────────────
    const handleExpand = () => {
        setCollapsed(false);
        setUnreadCount(0);
    };

    // ════════════════════════════════════════════════════════�?
    // Collapsed state �?small floating chat bubble
    // ════════════════════════════════════════════════════════�?
    if (collapsed) {
        return (
            <div className="flex flex-col items-start justify-start h-full pt-4 pl-4">
                <motion.button
                    initial={{ scale: 0.8, opacity: 0 }}
                    animate={{ scale: 1, opacity: 1 }}
                    whileHover={{ scale: 1.1 }}
                    whileTap={{ scale: 0.9 }}
                    onClick={handleExpand}
                    className={clsx(
                        "relative p-3 rounded-full",
                        "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                        "border border-[var(--color-border)]",
                        "text-[var(--color-text-secondary)] hover:text-[var(--color-accent)]",
                        "shadow-lg transition-colors"
                    )}
                    aria-label={t("chat.actions.open")}
                >
                    <MessageCircle size={20} strokeWidth={1.5} />
                    {/* Unread badge */}
                    {unreadCount > 0 && (
                        <motion.div
                            initial={{ scale: 0 }}
                            animate={{ scale: 1 }}
                            className="absolute -top-1 -right-1 w-5 h-5 rounded-full bg-[var(--color-accent)] text-black text-[10px] font-bold flex items-center justify-center shadow-[var(--glow-accent)]"
                        >
                            {unreadCount > 9 ? "9+" : unreadCount}
                        </motion.div>
                    )}
                </motion.button>
            </div>
        );
    }

    // ════════════════════════════════════════════════════════�?
    // Expanded state �?full chat panel
    // ════════════════════════════════════════════════════════�?
    const hasSendableImages = visionEnabled && pendingImages.length > 0;

    return (
        <motion.div
            initial={{ opacity: 0, x: -20 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
            className={clsx(
                "flex flex-col h-full w-full",
                "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                "border border-[var(--color-border)] rounded-xl shadow-lg",
                "relative overflow-hidden"
            )}
        >
            {/* Error toast */}
            <AnimatePresence>
                {error && <ErrorToast message={error} onDismiss={() => setError(null)} />}
            </AnimatePresence>

            {/* 对话历史侧边栏 */}
            <ConversationSidebar
                open={sidebarOpen}
                onClose={() => setSidebarOpen(false)}
                onLoadMessages={(msgs) => {
                    setMessages(msgs);
                    setExpandedTranslations(new Set()); // Reset translation expand state
                    setSidebarOpen(false);
                }}
            />

            {/* Header �?clean and minimal */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border)]">
                <div className="flex items-center gap-2">
                    <div className={clsx(
                        "w-2 h-2 rounded-full",
                        isStreaming
                            ? "bg-amber-500 animate-pulse"
                            : "bg-[var(--color-accent)] shadow-[var(--glow-success)]"
                    )} />
                    <span className="font-heading text-sm font-semibold tracking-wider uppercase text-[var(--color-text-secondary)]">
                        {isStreaming ? t("chat.status.streaming") : t("chat.status.chat")}
                    </span>
                </div>
                <div className="flex items-center gap-1">
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={() => setSidebarOpen(prev => !prev)}
                        className={clsx(
                            "p-2 rounded-md transition-colors",
                            sidebarOpen
                                ? "text-[var(--color-accent)]"
                                : "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                        )}
                        aria-label={t("chat.history.title")}
                        title={t("chat.history.title")}
                    >
                        <History size={14} strokeWidth={1.5} />
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={handleClear}
                        className="p-2 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-error)] transition-colors"
                        aria-label={t("chat.actions.clear")}
                        title={t("chat.actions.clear")}
                    >
                        <Trash2 size={14} strokeWidth={1.5} />
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={() => setCollapsed(true)}
                        className="p-2 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                        aria-label={t("chat.actions.collapse")}
                        title={t("chat.actions.collapse")}
                    >
                        <ChevronLeft size={14} strokeWidth={1.5} />
                    </motion.button>
                </div>
            </div>

            {/* Messages */}
            <div
                ref={messagesContainerRef}
                onScroll={handleScroll}
                className="flex-1 overflow-y-auto p-4 space-y-3 scrollable"
            >
                <AnimatePresence initial={false}>
                    {deferredMessages.slice(-visibleCount).map((msg, i) => {
                        const globalIndex = Math.max(0, deferredMessages.length - visibleCount) + i;
                        return (
                            <MemoizedChatMessage
                                key={`${globalIndex}-${msg.role}`}
                                message={msg}
                                globalIndex={globalIndex}
                                isStreaming={isStreaming}
                                isTranslationExpanded={expandedTranslations.has(globalIndex)}
                                onToggleTranslation={onToggleTranslation}
                                onEdit={onEdit}
                                onRegenerate={onRegenerate}
                                onContinueFrom={onContinueFrom}
                                onApproveTool={onApproveTool}
                                onRejectTool={onRejectTool}
                            />
                        );
                    })}

                    {shouldRenderTypingIndicator({ isThinking, messages: deferredMessages, activeMessageIndex: currentTurnRef.current?.messageIndex ?? null }) && <TypingIndicator />}
                </AnimatePresence>
                <div ref={messagesEndRef} />
            </div>

            {/* Input */}
            <form onSubmit={handleSend} className="border-t border-[var(--color-border)] bg-black/20">
                {/* Pending images preview */}
                <AnimatePresence>
                    {hasSendableImages && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="flex gap-2 px-3 pt-2 overflow-x-auto"
                        >
                            {pendingImages.map((url, idx) => (
                                <div key={idx} className="relative group flex-shrink-0">
                                    <img
                                        src={url}
                                        alt="pending"
                                        className="w-16 h-16 rounded-md object-cover border border-[var(--color-border)]"
                                    />
                                    <button
                                        type="button"
                                        onClick={() => removePendingImage(idx)}
                                        className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-red-500 text-white flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                                    >
                                        <X size={10} />
                                    </button>
                                </div>
                            ))}
                        </motion.div>
                    )}
                </AnimatePresence>

                <div className="relative flex items-center gap-2 p-3">
                    {/* Hidden file input */}
                    <input
                        ref={fileInputRef}
                        type="file"
                        accept="image/*"
                        className="hidden"
                        onChange={handleImageSelect}
                    />

                    {/* Image upload button �?only visible when Vision Mode is ON */}
                    {visionEnabled && (
                        <motion.button
                            type="button"
                            whileHover={{ scale: 1.1 }}
                            whileTap={{ scale: 0.9 }}
                            onClick={() => fileInputRef.current?.click()}
                            disabled={isStreaming || isUploading}
                            className={clsx(
                                "p-2.5 rounded-lg transition-colors",
                                "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]",
                                (isStreaming || isUploading) && "opacity-50 cursor-not-allowed"
                            )}
                            aria-label={t("chat.input.attach_image")}
                            title={t("chat.input.attach_image")}
                        >
                            {isUploading ? (
                                <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
                            ) : (
                                <ImagePlus size={16} strokeWidth={1.5} />
                            )}
                        </motion.button>
                    )}

                    {/* Camera frame indicator — visible when vision + camera both enabled */}
                    {visionEnabled && cameraEnabled && (
                        <div
                            className="flex items-center gap-1 px-1.5 py-1 rounded-md text-[10px] text-[var(--color-accent)] opacity-70 select-none"
                            title={t("chat.input.camera_frame_attached")}
                        >
                            <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)] animate-pulse" />
                            CAM
                        </div>
                    )}

                    {/* Microphone button�?Advanced VAD Mode */}
                    {sttEnabled && (
                        <div className="relative flex items-center justify-center">
                            {/* Volume ring �?visible when listening/speaking */}
                            {voiceState !== VoiceState.Idle && voiceState !== VoiceState.Processing && (
                                <motion.div
                                    className="absolute inset-0 rounded-lg border-2 border-[var(--color-accent)]"
                                    animate={{
                                        opacity: voiceState === VoiceState.Speaking ? [0.3, 0.8, 0.3] : 0.2,
                                        scale: voiceState === VoiceState.Speaking
                                            ? [1, 1 + Math.min(micVolume / 100, 0.5), 1]
                                            : 1,
                                    }}
                                    transition={{ duration: 0.3, repeat: voiceState === VoiceState.Speaking ? Infinity : 0 }}
                                    style={{ pointerEvents: "none" }}
                                />
                            )}
                            <motion.button
                                type="button"
                                whileHover={{ scale: 1.1 }}
                                whileTap={{ scale: 0.9 }}
                                onClick={handleMicToggle}
                                disabled={isStreaming}
                                className={clsx(
                                    "relative p-2.5 rounded-lg transition-all z-10",
                                    voiceState === VoiceState.Idle
                                        ? "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                        : voiceState === VoiceState.Listening
                                            ? "text-[var(--color-accent)] bg-[var(--color-accent)]/15 border border-[var(--color-accent)]/30"
                                            : voiceState === VoiceState.Speaking
                                                ? "text-red-400 bg-red-500/20 border border-red-500/40 shadow-[0_0_12px_rgba(239,68,68,0.3)]"
                                                : "text-amber-400 bg-amber-500/15 border border-amber-500/30",
                                    isStreaming && voiceState === VoiceState.Idle && "opacity-50 cursor-not-allowed"
                                )}
                                aria-label={
                                    voiceState === VoiceState.Idle ? t("chat.input.mic.title.idle") :
                                        voiceState === VoiceState.Listening ? t("chat.input.mic.title.listening") :
                                            voiceState === VoiceState.Speaking ? t("chat.input.mic.title.speaking") :
                                                t("chat.input.mic.title.transcribing")
                                }
                                title={
                                    voiceState === VoiceState.Idle ? t("chat.input.mic.title.idle") :
                                        voiceState === VoiceState.Listening ? t("chat.input.mic.title.listening") :
                                            voiceState === VoiceState.Speaking ? t("chat.input.mic.title.speaking") :
                                                t("chat.input.mic.title.transcribing")
                                }
                            >
                                {voiceState === VoiceState.Processing ? (
                                    <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
                                ) : voiceState === VoiceState.Speaking ? (
                                    <motion.div
                                        animate={{ scale: [1, 1.15, 1] }}
                                        transition={{ duration: 0.6, repeat: Infinity }}
                                    >
                                        <Mic size={16} strokeWidth={1.5} />
                                    </motion.div>
                                ) : voiceState !== VoiceState.Idle ? (
                                    <MicOff size={16} strokeWidth={1.5} />
                                ) : (
                                    <Mic size={16} strokeWidth={1.5} />
                                )}
                            </motion.button>
                        </div>
                    )}

                    <div className="relative flex-1">
                        <input
                            type="text"
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onPaste={handlePaste}
                            placeholder={t("chat.input.placeholder")}
                            disabled={isStreaming}
                            className={clsx(
                                "w-full bg-black/40 border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "text-sm rounded-lg pl-4 pr-8 py-2.5 font-body",
                                "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                                "transition-all",
                                isStreaming && "opacity-50 cursor-not-allowed"
                            )}
                        />
                        <button
                            type="button"
                            onClick={() => {
                                setExpandedInput(true);
                                setTimeout(() => {
                                    const ta = expandedTextareaRef.current;
                                    if (ta) { ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); }
                                }, 50);
                            }}
                            className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                            aria-label={t("chat.input.expand")}
                            title={t("chat.input.expand")}
                        >
                            <Maximize2 size={13} strokeWidth={1.5} />
                        </button>
                    </div>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.9 }}
                        type="submit"
                        disabled={isStreaming || (!input.trim() && !hasSendableImages)}
                        className={clsx(
                            "p-2.5 rounded-lg transition-colors",
                            "bg-[var(--color-accent)] text-black hover:bg-white",
                            (isStreaming || (!input.trim() && !hasSendableImages)) && "opacity-50 cursor-not-allowed"
                        )}
                        aria-label="Send message"
                    >
                        <Send size={16} strokeWidth={1.5} />
                    </motion.button>
                </div>
            </form>

            {/* Expanded input overlay */}
            <AnimatePresence>
                {expandedInput && (
                    <motion.div
                        initial={{ opacity: 0, y: 10 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: 10 }}
                        className="absolute inset-x-0 bottom-0 z-20 p-3 bg-[var(--color-bg-surface)] border-t border-[var(--color-border)] backdrop-blur-[var(--glass-blur)]"
                    >
                        <textarea
                            ref={expandedTextareaRef}
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onPaste={handlePaste}
                            onKeyDown={(e) => {
                                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                                    e.preventDefault();
                                    setExpandedInput(false);
                                    handleSend();
                                }
                                if (e.key === "Escape") setExpandedInput(false);
                            }}
                            placeholder={t("chat.input.placeholder")}
                            disabled={isStreaming}
                            rows={6}
                            className={clsx(
                                "w-full bg-black/40 border border-[var(--color-border)] rounded-lg",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "text-sm px-4 py-3 font-body resize-none",
                                "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                                "transition-all"
                            )}
                        />
                        <div className="flex items-center justify-between mt-2">
                            <span className="text-xs text-[var(--color-text-muted)]">Ctrl+Enter 发送 · Esc 收起</span>
                            <div className="flex gap-2">
                                <motion.button
                                    type="button"
                                    whileHover={{ scale: 1.05 }}
                                    whileTap={{ scale: 0.95 }}
                                    onClick={() => setExpandedInput(false)}
                                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] border border-[var(--color-border)] transition-colors"
                                >
                                    <Minimize2 size={12} strokeWidth={1.5} />
                                    收起
                                </motion.button>
                                <motion.button
                                    type="button"
                                    whileHover={{ scale: 1.05 }}
                                    whileTap={{ scale: 0.95 }}
                                    onClick={() => { setExpandedInput(false); handleSend(); }}
                                    disabled={isStreaming || !input.trim()}
                                    className={clsx(
                                        "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors",
                                        "bg-[var(--color-accent)] text-black hover:bg-white",
                                        (isStreaming || !input.trim()) && "opacity-50 cursor-not-allowed"
                                    )}
                                >
                                    <Send size={12} strokeWidth={1.5} />
                                    发送
                                </motion.button>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div >
    );
}
