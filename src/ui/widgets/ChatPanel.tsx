// pattern: Imperative Shell

import { useState, useRef, useEffect, useLayoutEffect, useCallback, useDeferredValue, memo, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type SyntheticEvent } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Send, Trash2, AlertCircle, MessageCircle, ChevronLeft, ChevronDown, ImagePlus, X, Mic, MicOff, History } from "lucide-react";
import { streamChat, cancelChatTurn, onChatTurnStart, onChatTurnDelta, onChatTurnFinish, onChatTurnTextComplete, onChatError, onChatWarning, onChatFailure, onChatTurnTranslation, clearHistory, uploadVisionImage, synthesize, onChatTurnTool, listConversations, loadConversation, editConversationMessage, listCharacters, onTelegramChatSync, onVisionObservation, deleteLastMessages, approveToolApproval, rejectToolApproval, getMemoryEmbeddingModelStatus, setVisionTextInputFocused } from "../../lib/kokoro-bridge";
import type { CommittedCharacterRuntime, FailureEvent, ToolTraceItem } from "../../lib/kokoro-bridge";
import { getLatestCameraFrame } from "../../lib/camera-frame-cache";
import { listen } from "@tauri-apps/api/event";
import { useVoiceInput, VoiceState, useTypingReveal, useWakeWord } from "../hooks";
import { useTranslation } from "react-i18next";
import { ImageLightbox } from "../components/ImageLightbox";
import ConversationSidebar from "./ConversationSidebar";
import { ChatMessage } from "./ChatMessage";
import { createChatCharacterSynchronizer, type ChatCharacterSynchronizer } from "./chat-character-sync";
import {
    getCharacterHeaderDisplayName,
    getInitialCharacterConversationTarget,
    isFailureForActiveChat,
    shouldIgnoreLegacyChatError,
    shouldSynchronizeOnRuntimeChanged,
} from "./chat-character-sync-core";
import { getStreamingRevealText, hasActiveKokoroBubble, shouldRenderTypingIndicator } from "./chat-streaming-state";
import {
    canSubmitApproval,
    ensureTurnMessage,
    getApprovalErrorMessage,
    getApprovalRequestId,
    getToolEventStateUpdate,
    hasRenderableTurnContent,
    removeTurnMessages,
    stripStoredMarkup,
    stripStreamingMarkup,
    updateApprovalToolLocally,
    updateTurnMessage,
    type ChatPanelMessage,
    type PendingTurnState,
} from "./chat/turn-state";
import {
    computeTargetScrollTop,
    isScrollAtBottom,
    computeAnchoredScrollTop,
    type ChatScrollSnapshot,
} from "./chat/chat-scroll-state";
import {
    computeResizedInputHeight,
    loadSavedChatInputHeight,
    saveChatInputHeight,
    toggleChatInputResetHeight,
} from "./chat/chat-input-layout";
import { combineDraftWithTranscription } from "./chat/chat-draft-layout";
import { useCharacterChatDraft } from "./chat/use-character-draft";
import { requestMemoryModelDialog } from "../../lib/memory-model-gate";
import { getChatPanelInteractionProps } from "../layout/layout-interaction";
import { audioPlayer } from "../../core/services";
import {
    APP_SETTING_KEYS,
    readBooleanSetting,
    readJsonSetting,
    readNumberSetting,
    readStringSetting,
} from "../../lib/app-settings";

// ── Types ──────────────────────────────────────────────────
type ChatMessage = ChatPanelMessage;

interface ChatPanelProps {
    width?: number;
    minWidth?: number;
    onWidthPreview?: (width: number) => number;
    onWidthChange?: (width: number) => void;
    /** Blocks background interaction while onboarding owns the first turn. */
    interactionDisabled?: boolean;
}

export type { ChatPanelMessage };

const DEFAULT_CHAT_PANEL_WIDTH = 350;
const CHAT_PANEL_RESIZE_GUTTER = 160;
const CHAT_PANEL_KEYBOARD_RESIZE_STEP = 24;

const getChatPanelResizeMaxWidth = (minWidth: number) => {
    if (typeof window === "undefined") {
        return minWidth;
    }
    return Math.max(minWidth, window.innerWidth - CHAT_PANEL_RESIZE_GUTTER);
};

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

function logToolEvent(event: { tool: string; result?: { message: string }; error?: string }): void {
    if (shouldLogToolEventSuccess(event)) {
        console.log(`[ToolCall] ${event.tool}: ${getToolEventSuccessMessage(event)}`);
        return;
    }
    if (shouldLogToolEventError(event)) {
        console.error(`[ToolCall] ${event.tool} failed: ${getToolEventErrorMessage(event)}`);
    }
}

function getAsyncErrorMessage(error: unknown): string {
    if (error instanceof Error) {
        return error.message;
    }
    if (typeof error === "object" && error !== null && "message" in error && typeof (error as { message?: unknown }).message === "string") {
        return (error as { message: string }).message;
    }
    return String(error);
}

function isTurnCancelledError(error: unknown): boolean {
    const message = getAsyncErrorMessage(error).toLowerCase();
    return message.includes("turn cancelled by user") || message.includes("turn canceled by user");
}


// ── Typing Indicator ───────────────────────────────────────
const getActiveCharacterIdForRequest = () =>
    readStringSetting(APP_SETTING_KEYS.activeCharacterId, "") || undefined;

const getActiveCharacterIdForConversationRestore = () =>
    readStringSetting(APP_SETTING_KEYS.activeCharacterId, "default") || "default";

const getTtsPlaybackSettings = () => ({
    enabled: readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false),
    provider_id: readStringSetting(APP_SETTING_KEYS.ttsProvider, "") || undefined,
    voice: readStringSetting(APP_SETTING_KEYS.ttsVoice, "") || undefined,
    speed: readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1.0),
    pitch: readNumberSetting(APP_SETTING_KEYS.ttsPitch, 1.0),
});

const isGeneratedBackgroundMode = () =>
    readJsonSetting<{ mode?: string }>(APP_SETTING_KEYS.bgConfig, {}).mode === "generated";

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
            className="absolute top-2 left-2 right-2 z-[110] flex items-start gap-2 px-4 py-2 rounded-lg bg-red-900/80 border border-red-500/50 text-red-200 text-xs shadow-lg"
        >
            <AlertCircle size={14} strokeWidth={1.5} className="mt-0.5 shrink-0" />
            <span className="min-w-0 flex-1 break-words leading-relaxed [overflow-wrap:anywhere]">
                {message}
            </span>
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
    onPreviewImage?: (url: string) => void;
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
    onPreviewImage,
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
            onPreviewImage={onPreviewImage}
        />
    );
});

export default function ChatPanel({
    width = DEFAULT_CHAT_PANEL_WIDTH,
    minWidth = DEFAULT_CHAT_PANEL_WIDTH,
    onWidthPreview,
    onWidthChange,
    interactionDisabled = false,
}: ChatPanelProps) {
    const { t } = useTranslation();
    const interactionProps = getChatPanelInteractionProps(interactionDisabled);
    const blockDisabledInteraction = useCallback((event: SyntheticEvent<HTMLElement>) => {
        if (!interactionDisabled) return;
        event.preventDefault();
        event.stopPropagation();
        if (event.target instanceof HTMLElement) {
            event.target.blur();
        }
    }, [interactionDisabled]);
    const [collapsed, setCollapsed] = useState(false);
    const [messages, setMessages] = useState<ChatMessage[]>([]);
    const [activeCharacterId, setActiveCharacterId] = useState(
        getActiveCharacterIdForConversationRestore,
    );
    const activeCharacterIdRef = useRef(activeCharacterId);
    const [characterName, setCharacterName] = useState<string>(() => {
        const committed = readJsonSetting<CommittedCharacterRuntime | null>(
            APP_SETTING_KEYS.characterRuntimeCache,
            null,
        );
        return getCharacterHeaderDisplayName(committed?.runtime?.character_name);
    });

    useEffect(() => {
        let active = true;
        if (!characterName || activeCharacterId) {
            void listCharacters().then(chars => {
                if (!active) return;
                const match = chars.find(c => c.id === activeCharacterId);
                if (match?.name) {
                    setCharacterName(getCharacterHeaderDisplayName(match.name));
                }
            }).catch(() => {});
        }
        return () => { active = false; };
    }, [activeCharacterId, characterName]);

    const [activeConversationId, setActiveConversationId] = useState<string | null>(null);
    const activeConversationIdRef = useRef(activeConversationId);
    activeConversationIdRef.current = activeConversationId;
    const deferredMessages = useDeferredValue(messages);
    const [visibleCount, setVisibleCount] = useState(20);
    const [showScrollBottom, setShowScrollBottom] = useState(false);
    const [hasNewMessagesBelow, setHasNewMessagesBelow] = useState(false);
    const isPrependingRef = useRef(false);
    const prevScrollHeightRef = useRef(0);
    const prevScrollTopRef = useRef(0);
    const { input, setInput, pendingImages, setPendingImages, clearDraft } = useCharacterChatDraft(activeCharacterId);
    const inputRef = useRef(input);
    inputRef.current = input;
    const sttBaseDraftRef = useRef<string | null>(null);
    const prevVoiceStateRef = useRef<VoiceState>(VoiceState.Idle);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const [inputHeight, setInputHeight] = useState<number>(loadSavedChatInputHeight);
    const inputHeightRef = useRef(inputHeight);
    useEffect(() => {
        inputHeightRef.current = inputHeight;
    }, [inputHeight]);
    const isDraggingInputResizeRef = useRef(false);
    const inputResizeStartYRef = useRef(0);
    const inputResizeStartHeightRef = useRef(0);

    const handleInputResizeStart = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
        e.preventDefault();
        isDraggingInputResizeRef.current = true;
        inputResizeStartYRef.current = e.clientY;
        inputResizeStartHeightRef.current = inputHeightRef.current;
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    }, []);

    const handleInputResizeMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
        if (!isDraggingInputResizeRef.current) return;
        const nextHeight = computeResizedInputHeight(
            inputResizeStartHeightRef.current,
            inputResizeStartYRef.current,
            e.clientY,
        );
        setInputHeight(nextHeight);
    }, []);

    const handleInputResizeEnd = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
        if (!isDraggingInputResizeRef.current) return;
        isDraggingInputResizeRef.current = false;
        try {
            (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
        } catch {
            // pointer capture already released
        }
        saveChatInputHeight(inputHeightRef.current);
    }, []);

    const handleInputResizeReset = useCallback(() => {
        setInputHeight(prev => {
            const next = toggleChatInputResetHeight(prev);
            saveChatInputHeight(next);
            return next;
        });
    }, []);

    const [previewImageUrl, setPreviewImageUrl] = useState<string | null>(null);
    const [isDraggingOver, setIsDraggingOver] = useState(false);
    const dragCounterRef = useRef(0);
    const [isStreaming, setIsStreaming] = useState(false);
    const isStreamingRef = useRef(false);
    const [isBusy, setIsBusy] = useState(false);
    const isBusyRef = useRef(false);
    const ttsSpeakingRef = useRef(false);
    const [isStopping, setIsStopping] = useState(false);
    const cancelRequestedRef = useRef(false);
    const cancellationWatchdogTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const messagesRef = useRef<ChatMessage[]>([]);
    const [isThinking, setIsThinking] = useState(false);
    const [showClearConfirm, setShowClearConfirm] = useState(false);

    useEffect(() => {
        if (!showClearConfirm) return;
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                setShowClearConfirm(false);
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [showClearConfirm]);

    // Per-message translation expand state (set of message indices)
    const [expandedTranslations, setExpandedTranslations] = useState<Set<number>>(new Set());

    const startStreaming = useCallback(() => {
        cancelRequestedRef.current = false;
        setIsStopping(false);
        isBusyRef.current = true;
        setIsBusy(true);
        isStreamingRef.current = true;
        setIsStreaming(true);
    }, []);
    const stopStreaming = useCallback(() => {
        setIsStopping(false);
        isStreamingRef.current = false;
        setIsStreaming(false);
    }, []);
    const endTurnActivity = useCallback(() => {
        if (cancellationWatchdogTimerRef.current !== null) {
            clearTimeout(cancellationWatchdogTimerRef.current);
            cancellationWatchdogTimerRef.current = null;
        }
        cancelRequestedRef.current = false;
        setIsStopping(false);
        isStreamingRef.current = false;
        setIsStreaming(false);
        isBusyRef.current = false;
        setIsBusy(false);
    }, []);

    // Raw (unfiltered) full response text — accumulated from all deltas
    const rawResponseRef = useRef("");
    const currentTurnRef = useRef<PendingTurnState | null>(null);
    const pendingVisionContextRef = useRef<ChatMessage | null>(null);

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
    const savedScrollSnapshotRef = useRef<ChatScrollSnapshot | null>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const resizeCleanupRef = useRef<(() => void) | null>(null);
    const latestResizeWidthRef = useRef(width);
    // Store last failed request for retry
    const lastFailedRequestRef = useRef<{ message: string; images?: string[]; allowImageGen?: boolean } | null>(null);

    const ensureMemoryModelReady = useCallback((options?: { silent?: boolean }): boolean => {
        // Semantic memory is an optional enhancement. Never hold a base LLM turn
        // on model discovery; status and download continue in the application shell.
        void getMemoryEmbeddingModelStatus()
            .then((status) => {
                if (!status.installed) {
                    requestMemoryModelDialog();
                }
            })
            .catch((err) => {
                console.error("[ChatPanel] Failed to query memory model status:", err);
                if (!options?.silent) {
                    setError(t("chat.errors.memory_model_check_failed"));
                }
                requestMemoryModelDialog();
            });
        return true;
    }, [t]);

    // Vision Mode
    const [visionEnabled, setVisionEnabled] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false)
    );
    const [cameraEnabled, setCameraEnabled] = useState(() =>
        readJsonSetting<{ camera_enabled?: boolean }>(
            APP_SETTING_KEYS.visionConfig,
            {},
        ).camera_enabled === true
    );
    // pendingImages is managed by useCharacterChatDraft with character-scoped persistence
    const [isUploading, setIsUploading] = useState(false);

    // 对话历史侧边栏
    const [sidebarOpen, setSidebarOpen] = useState(false);

    const requestTurnCancellation = useCallback(async (turnId: string) => {
        try {
            await cancelChatTurn(turnId, "stopped_from_chat_panel");
        } catch (error) {
            if (!isTurnCancelledError(error)) {
                endTurnActivity();
                currentTurnRef.current = null;
                setIsThinking(false);
                setError(getAsyncErrorMessage(error));
            }
        }
    }, [endTurnActivity]);

    const conversationSyncRef = useRef<ChatCharacterSynchronizer | null>(null);
    if (conversationSyncRef.current === null) {
        conversationSyncRef.current = createChatCharacterSynchronizer({
            listConversations,
            loadConversation,
            clearVisibleConversation: (characterId) => {
                const turnId = currentTurnRef.current?.turnId;
                endTurnActivity();
                cancelRequestedRef.current = true;
                if (turnId) {
                    void cancelChatTurn(turnId, "character_switched")
                        .catch(error => console.error("[ChatPanel] Failed to cancel prior character turn:", error));
                }
                currentTurnRef.current = null;
                pendingVisionContextRef.current = null;
                rawResponseRef.current = "";
                resetReveal();
                setIsThinking(false);
                setActiveCharacterId(characterId);
                activeCharacterIdRef.current = characterId;
                setActiveConversationId(null);
                activeConversationIdRef.current = null;
                setMessages([]);
                setExpandedTranslations(new Set());
            },
            applyVisibleConversation: (conversation) => {
                setActiveCharacterId(conversation.characterId);
                activeCharacterIdRef.current = conversation.characterId;
                setActiveConversationId(conversation.conversationId);
                activeConversationIdRef.current = conversation.conversationId;
                setMessages([...conversation.messages]);
                setExpandedTranslations(new Set());
            },
        });
    }

    const handleStopGeneration = useCallback(() => {
        if (!isStreamingRef.current || isStopping) {
            return;
        }

        cancelRequestedRef.current = true;
        setIsStopping(true);
        setIsThinking(false);

        // 启动安全看门狗：如果 5 秒内后端由于异常未能正常结束 turn，强制复位 UI 状态
        if (cancellationWatchdogTimerRef.current !== null) {
            clearTimeout(cancellationWatchdogTimerRef.current);
        }
        cancellationWatchdogTimerRef.current = setTimeout(() => {
            console.warn("[ChatPanel] Cancellation watchdog triggered - forcing UI reset");
            endTurnActivity();
            currentTurnRef.current = null;
            setIsThinking(false);
        }, 5000);

        const activeTurnId = currentTurnRef.current?.turnId;
        if (activeTurnId) {
            void requestTurnCancellation(activeTurnId);
        }
    }, [isStopping, requestTurnCancellation, endTurnActivity]);

    // 自动恢复最近对话
    useEffect(() => {
        const synchronizer = conversationSyncRef.current;
        if (synchronizer === null) return;
        const activeSynchronizer = synchronizer;

        function synchronize(characterId: string, preferredConversationId: string | null): void {
            void activeSynchronizer.synchronize({ characterId, preferredConversationId })
                .catch(err => console.error("[ChatPanel] Failed to restore conversation:", err));
        }

        const activeCharacter = getActiveCharacterIdForConversationRestore();
        const committed = readJsonSetting<CommittedCharacterRuntime | null>(
            APP_SETTING_KEYS.characterRuntimeCache,
            null,
        );
        const initialTarget = getInitialCharacterConversationTarget(activeCharacter, committed);
        synchronize(initialTarget.characterId, initialTarget.preferredConversationId);
        const handleRuntimeChanged = (event: Event): void => {
            const detail = (event as CustomEvent<CommittedCharacterRuntime>).detail;
            const eventCharacterId = detail?.runtime?.character_id;
            const eventCharacterName = detail?.runtime?.character_name;
            if (eventCharacterName) {
                setCharacterName(getCharacterHeaderDisplayName(eventCharacterName));
            }
            const targetConversationId = detail?.target_conversation_id ?? null;
            if (!shouldSynchronizeOnRuntimeChanged(
                activeCharacterIdRef.current,
                eventCharacterId,
                activeConversationIdRef.current,
                targetConversationId,
            )) {
                return;
            }
            synchronize(eventCharacterId, targetConversationId);
        };
        window.addEventListener("kokoro-character-runtime-changed", handleRuntimeChanged);
        return () => {
            window.removeEventListener("kokoro-character-runtime-changed", handleRuntimeChanged);
            activeSynchronizer.invalidate();
        };
    }, []);

    const handleConversationSelection = useCallback(async (
        preferredConversationId: string | null,
    ): Promise<void> => {
        await conversationSyncRef.current?.synchronize({
            characterId: activeCharacterId,
            preferredConversationId,
        });
    }, [activeCharacterId]);

    const handleStartEmptyConversation = useCallback((): void => {
        conversationSyncRef.current?.startEmptyConversation(activeCharacterId);
        void clearHistory().catch((err) => {
            console.error("[ChatPanel] Failed to clear backend history for empty conversation:", err);
        });
    }, [activeCharacterId]);

    // STT (Speech-to-Text) — Advanced VAD Mode
    const [sttEnabled, setSttEnabled] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.sttEnabled, false)
    );
    const [sttAutoSend, setSttAutoSend] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.sttAutoSend, false)
    );
    const [continuousListening, setContinuousListening] = useState(
        () => readBooleanSetting(APP_SETTING_KEYS.sttContinuousListening, false)
    );

    useEffect(() => {
        const syncSttSettings = () => {
            setSttEnabled(readBooleanSetting(APP_SETTING_KEYS.sttEnabled, false));
            setSttAutoSend(readBooleanSetting(APP_SETTING_KEYS.sttAutoSend, false));
            setContinuousListening(readBooleanSetting(APP_SETTING_KEYS.sttContinuousListening, false));
            setWakeWordEnabled(readBooleanSetting(APP_SETTING_KEYS.wakeWordEnabled, false));
            setWakeWord(readStringSetting(APP_SETTING_KEYS.wakeWord, ""));
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
        if (!trimmed) {
            // 空文本或未识别：恢复原草稿并重置快照
            if (sttBaseDraftRef.current !== null) {
                setInput(sttBaseDraftRef.current);
                sttBaseDraftRef.current = null;
            }
            return;
        }

        const base = sttBaseDraftRef.current ?? "";
        sttBaseDraftRef.current = null; // 正常结算，解除锁定
        const fullMessage = combineDraftWithTranscription(base, trimmed);

        if (sttAutoSend) {
            void (async () => {
                if (!await ensureMemoryModelReady()) {
                    setInput(fullMessage);
                    return;
                }

                // Auto-send: inject directly into chat
                const clientRequestId = `stt_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
                clearDraft();
                setMessages(prev => [...prev, { role: "user", text: fullMessage, clientRequestId }]);
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;

                const allowImageGen = isGeneratedBackgroundMode();

                streamChat({
                    message: fullMessage,
                    allow_image_gen: allowImageGen,
                    character_id: getActiveCharacterIdForRequest(),
                    client_request_id: clientRequestId,
                }).then(res => {
                    if (res?.conversation_id) {
                        setActiveConversationId(res.conversation_id);
                        activeConversationIdRef.current = res.conversation_id;
                    }
                    if (res?.user_message_id) {
                        setMessages(prev => {
                            const idx = prev.findIndex(m => m.clientRequestId === clientRequestId);
                            if (idx !== -1 && !prev[idx].id) {
                                const updated = [...prev];
                                updated[idx] = { ...updated[idx], id: res.user_message_id ?? undefined };
                                return updated;
                            }
                            return prev;
                        });
                    }
                }).catch(err => {
                    if (isTurnCancelledError(err) || cancelRequestedRef.current) {
                        endTurnActivity();
                        currentTurnRef.current = null;
                        setIsThinking(false);
                        return;
                    }
                    endTurnActivity();
                    currentTurnRef.current = null;
                    setIsThinking(false);
                    setError(getAsyncErrorMessage(err));
                });
            })();
        } else {
            // Fill input box with merged text for user review
            setInput(fullMessage);
        }
    }, [endTurnActivity, ensureMemoryModelReady, sttAutoSend, startStreaming, clearDraft, setInput]);

    const { state: voiceState, volume: micVolume, partialText: sttPartialText, start: startVoice, stop: stopVoice } = useVoiceInput(handleTranscription);

    // Refs to avoid stale closures in the voice-interrupt-stt listener
    const startVoiceRef = useRef(startVoice);
    const sttAutoSendRef = useRef(sttAutoSend);
    const sttEnabledRef = useRef(sttEnabled);
    useEffect(() => { startVoiceRef.current = startVoice; }, [startVoice]);
    useEffect(() => { sttAutoSendRef.current = sttAutoSend; }, [sttAutoSend]);
    useEffect(() => { sttEnabledRef.current = sttEnabled; }, [sttEnabled]);

    useEffect(() => {
        const syncTextInputFocus = () => {
            const active = document.activeElement;
            const focused = active === textareaRef.current;
            setVisionTextInputFocused(focused).catch(error => {
                console.error("[ChatPanel] Failed to sync text input focus:", error);
            });
        };

        syncTextInputFocus();
        window.addEventListener("focusin", syncTextInputFocus);
        window.addEventListener("focusout", syncTextInputFocus);

        return () => {
            window.removeEventListener("focusin", syncTextInputFocus);
            window.removeEventListener("focusout", syncTextInputFocus);
            setVisionTextInputFocused(false).catch(() => { /* best effort */ });
        };
    }, []);

    // Wake word detection — starts main STT when keyword is heard
    const [wakeWordEnabled, setWakeWordEnabled] = useState(() =>
        readBooleanSetting(APP_SETTING_KEYS.wakeWordEnabled, false)
    );
    const [wakeWord, setWakeWord] = useState(() =>
        readStringSetting(APP_SETTING_KEYS.wakeWord, "")
    );
    useWakeWord({
        enabled:
            sttEnabled &&
            !isBusy &&
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
            sttBaseDraftRef.current = inputRef.current;
            startVoice({ autoStopOnSilence: true });
        }, [continuousListening, handleTranscription, startVoice]),
    });

    // Effect: Sync partial STT text to input box for real-time feedback
    useEffect(() => {
        if (voiceState === VoiceState.Listening && sttPartialText) {
            const base = sttBaseDraftRef.current ?? "";
            const combined = combineDraftWithTranscription(base, sttPartialText);
            setInput(combined);
        }
    }, [sttPartialText, voiceState, setInput]);

    // 听音生命周期退出兜底：若未完成识别退出且存在基准草稿，自动无损回滚
    useEffect(() => {
        if (prevVoiceStateRef.current === VoiceState.Listening && voiceState === VoiceState.Idle) {
            if (sttBaseDraftRef.current !== null) {
                setInput(sttBaseDraftRef.current);
                sttBaseDraftRef.current = null;
            }
        }
        prevVoiceStateRef.current = voiceState;
    }, [voiceState, setInput]);

    // Sync vision state when localStorage changes (from Settings panel)
    useEffect(() => {
        const checkVision = () => {
            const nextVisionEnabled = readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false);
            setVisionEnabled(nextVisionEnabled);
            if (!nextVisionEnabled) setPendingImages([]);
            const cfg = readJsonSetting<{ camera_enabled?: boolean }>(
                APP_SETTING_KEYS.visionConfig,
                {},
            );
            setCameraEnabled(cfg.camera_enabled === true);
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

    // ── Restore scroll display position on expand ───────────
    useLayoutEffect(() => {
        if (collapsed) return;
        const container = messagesContainerRef.current;
        if (!container) return;

        isProgrammaticScrollRef.current = true;
        const restoreScroll = () => {
            const el = messagesContainerRef.current;
            if (!el) return;
            const target = computeTargetScrollTop(
                savedScrollSnapshotRef.current,
                el.scrollHeight,
                el.clientHeight
            );
            el.scrollTop = target.scrollTop;
            userScrolledRef.current = target.userScrolled;
        };

        restoreScroll();

        const rafId = requestAnimationFrame(() => {
            restoreScroll();
            requestAnimationFrame(restoreScroll);
        });

        const timer = setTimeout(() => {
            isProgrammaticScrollRef.current = false;
        }, 120);

        return () => {
            cancelAnimationFrame(rafId);
            clearTimeout(timer);
        };
    }, [collapsed]);

    const handleScroll = useCallback(() => {
        // Ignore scroll events triggered by our own scrollToBottom or restore
        if (isProgrammaticScrollRef.current) return;
        const container = messagesContainerRef.current;
        if (!container) return;
        const atBottom = isScrollAtBottom(
            container.scrollTop,
            container.scrollHeight,
            container.clientHeight,
            120
        );
        userScrolledRef.current = !atBottom;
        setShowScrollBottom(!atBottom);
        if (atBottom) {
            setHasNewMessagesBelow(false);
        }

        savedScrollSnapshotRef.current = {
            scrollTop: container.scrollTop,
            scrollHeight: container.scrollHeight,
            clientHeight: container.clientHeight,
            isAtBottom: atBottom,
        };

        // 向上滚动加载分页：增加边界与防抖检查，记录基准高度
        const hasMore = visibleCount < deferredMessages.length;
        if (container.scrollTop < 100 && hasMore && !isPrependingRef.current) {
            isPrependingRef.current = true;
            prevScrollHeightRef.current = container.scrollHeight;
            prevScrollTopRef.current = container.scrollTop;
            setVisibleCount(prev => prev + 20);
        }
    }, [deferredMessages.length, visibleCount]);

    // 滚动锚定：在前置插入旧消息后，在浏览器绘制前补偿 scrollTop，杜绝视口抖动
    useLayoutEffect(() => {
        if (!isPrependingRef.current) return;
        isPrependingRef.current = false;
        const container = messagesContainerRef.current;
        if (!container) return;

        const targetScrollTop = computeAnchoredScrollTop(
            prevScrollTopRef.current,
            prevScrollHeightRef.current,
            container.scrollHeight
        );

        if (targetScrollTop !== container.scrollTop) {
            isProgrammaticScrollRef.current = true;
            container.scrollTop = targetScrollTop;
            requestAnimationFrame(() => {
                isProgrammaticScrollRef.current = false;
            });
        }
    }, [visibleCount]);

    // 离开底部时侦测新到达消息以点亮悬浮指示灯
    useEffect(() => {
        if (userScrolledRef.current && messages.length > 0) {
            setHasNewMessagesBelow(true);
        }
    }, [messages.length]);

    const scrollToBottomSmooth = useCallback(() => {
        const container = messagesContainerRef.current;
        if (!container) return;
        userScrolledRef.current = false;
        setShowScrollBottom(false);
        setHasNewMessagesBelow(false);
        isProgrammaticScrollRef.current = true;
        container.scrollTo({
            top: container.scrollHeight,
            behavior: "smooth",
        });
        setTimeout(() => {
            isProgrammaticScrollRef.current = false;
        }, 300);
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

            const unTurnStart = await onChatTurnStart(({ turn_id, client_request_id, conversation_id, user_message_id }) => {
                if (aborted) return;
                if (conversation_id) {
                    setActiveConversationId(conversation_id);
                    activeConversationIdRef.current = conversation_id;
                }
                if (user_message_id) {
                    setMessages(prev => {
                        let idx = client_request_id ? prev.findIndex(m => m.clientRequestId === client_request_id) : -1;
                        if (idx === -1) {
                            idx = prev.map(m => m.role).lastIndexOf("user");
                        }
                        if (idx !== -1 && !prev[idx].id) {
                            const updated = [...prev];
                            updated[idx] = { ...updated[idx], id: user_message_id };
                            return updated;
                        }
                        return prev;
                    });
                }
                currentTurnRef.current = {
                    turnId: turn_id,
                    messageIndex: null,
                    rawText: "",
                    visibleTextStarted: false,
                    translation: undefined,
                    translationPending: false,
                    tools: [],
                    pendingContext: pendingVisionContextRef.current ?? undefined,
                };
                pendingVisionContextRef.current = null;
                rawResponseRef.current = "";

                if (cancelRequestedRef.current) {
                    void requestTurnCancellation(turn_id);
                    return;
                }
            });
            if (aborted) { unTurnStart(); return; }
            cleanups.push(unTurnStart);

            const unDelta = await onChatTurnDelta(({ turn_id, delta: rawDelta }) => {
                if (aborted || !isStreamingRef.current || cancelRequestedRef.current) return;
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
                if (userScrolledRef.current) {
                    setHasNewMessagesBelow(true);
                }
            });
            if (aborted) { unDelta(); return; }
            cleanups.push(unDelta);

            const unTextComplete = await onChatTurnTextComplete(({ turn_id, text, translation_pending, translation }) => {
                if (aborted || cancelRequestedRef.current) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;

                turn.rawText = text;
                if (translation) {
                    turn.translation = translation;
                }
                turn.translationPending = translation_pending;
                rawResponseRef.current = text;

                flushReveal();
                stopStreaming();
                setIsThinking(false);

                const cleanText = stripStoredMarkup(text);
                const hasContent = hasRenderableTurnContent(turn, cleanText);
                if (!hasContent) {
                    setMessages(prev => removeTurnMessages(prev, turn));
                    return;
                }

                setMessages(prev => {
                    const ensured = ensureTurnMessage(prev, turn);
                    return updateTurnMessage(ensured, turn, (current) => ({
                        ...current,
                        text: cleanText,
                        translation: turn.translation,
                        translationPending: translation_pending,
                        tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
                    }));
                });
            });
            if (aborted) { unTextComplete(); return; }
            cleanups.push(unTextComplete);

            const unTranslation = await onChatTurnTranslation(({ turn_id, translation }) => {
                if (aborted || cancelRequestedRef.current) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;
                turn.translation = translation;
                turn.translationPending = false;
                setMessages(prev => updateTurnMessage(prev, turn, (current) => ({
                    ...current,
                    translation,
                    translationPending: false,
                })));
            });
            if (aborted) { unTranslation(); return; }
            cleanups.push(unTranslation);

            const unDone = await onChatTurnFinish(({ turn_id, status, conversation_id, assistant_message_id }) => {
                if (aborted) return;
                if (conversation_id) {
                    setActiveConversationId(conversation_id);
                    activeConversationIdRef.current = conversation_id;
                }
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) {
                    if (cancelRequestedRef.current) {
                        endTurnActivity();
                        currentTurnRef.current = null;
                        setIsThinking(false);
                    }
                    return;
                }

                flushReveal();
                endTurnActivity();
                setIsThinking(false);

                const fullText = turn.rawText;
                rawResponseRef.current = fullText;
                const cleanText = stripStoredMarkup(fullText);

                setMessages(prev => {
                    const hasContent = hasRenderableTurnContent(turn, cleanText);

                    if (hasActiveKokoroBubble(prev, turn.messageIndex)) {
                        if (!hasContent) {
                            return removeTurnMessages(prev, turn);
                        }

                        return updateTurnMessage(prev, turn, (current) => ({
                            ...current,
                            id: assistant_message_id ?? current.id,
                            text: cleanText,
                            translation: turn.translation,
                            translationPending: false,
                            tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
                        }));
                    }

                    if (hasContent) {
                        const next = [...prev];
                        if (turn.pendingContext && !next.some(message => message.role === "context" && message.turnId === turn.turnId)) {
                            next.push({
                                ...turn.pendingContext,
                                turnId: turn.turnId,
                            });
                        }
                        next.push({
                            id: assistant_message_id ?? undefined,
                            role: "kokoro",
                            text: cleanText,
                            translation: turn.translation,
                            translationPending: false,
                            tools: turn.tools.length > 0 ? [...turn.tools] : undefined,
                        });
                        return next;
                    }

                    return prev;
                });

                currentTurnRef.current = null;

                const playback = getTtsPlaybackSettings();
                if (status === "completed" && playback.enabled && cleanText.trim()) {
                    console.log("[TTS] Auto-speak triggered, text length:", cleanText.length);
                    const { enabled: _enabled, ...ttsConfig } = playback;
                    synthesize(cleanText.trim(), ttsConfig).catch(err => console.error("[TTS] Auto-speak failed:", err));
                }
            });
            if (aborted) { unDone(); return; }
            cleanups.push(unDone);

            const unFailure = await onChatFailure((failure: FailureEvent) => {
                if (aborted) return;
                if (!isFailureForActiveChat(
                    failure,
                    activeCharacterIdRef.current,
                    currentTurnRef.current?.turnId ?? null,
                )) return;
                endTurnActivity();
                setIsThinking(false);
                const suffix = failure.stage ? ` (${failure.stage})` : "";
                setError(`${failure.message}${suffix}`);
                currentTurnRef.current = null;
            });
            if (aborted) { unFailure(); return; }
            cleanups.push(unFailure);

            const unError = await onChatError((err: string) => {
                if (aborted) return;
                if (shouldIgnoreLegacyChatError(currentTurnRef.current?.turnId ?? null)) return;
                endTurnActivity();
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
                if (aborted || cancelRequestedRef.current) return;
                logToolEvent(event);
                const turn = currentTurnRef.current;
                setMessages(prev => getToolEventStateUpdate(event, turn, event.turn_id)(prev));
            });
            if (aborted) { unToolResult(); return; }
            cleanups.push(unToolResult);

            const unVisionObservation = await onVisionObservation((observation) => {
                if (aborted) return;
                const summary = observation.summary.trim();
                if (!summary) return;
                pendingVisionContextRef.current = {
                    role: "context",
                    text: summary,
                    capturedAt: observation.captured_at,
                    source: observation.source,
                };
            });
            if (aborted) { unVisionObservation(); return; }
            cleanups.push(unVisionObservation);

            const unTtsStart = await listen("tts:start", () => {
                if (aborted) return;
                ttsSpeakingRef.current = true;
            });
            if (aborted) { unTtsStart(); return; }
            cleanups.push(unTtsStart);

            const unTtsEnd = await listen("tts:end", () => {
                if (aborted) return;
                ttsSpeakingRef.current = false;
            });
            if (aborted) { unTtsEnd(); return; }
            cleanups.push(unTtsEnd);

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
                const browserSpeaking = typeof window !== "undefined"
                    && Boolean(window.speechSynthesis?.speaking);
                if (aborted || isBusyRef.current || ttsSpeakingRef.current || audioPlayer.isPlaying || browserSpeaking) return;
                void (async () => {
                    if (!await ensureMemoryModelReady({ silent: true })) {
                        return;
                    }

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
                        character_id: getActiveCharacterIdForRequest(),
                    }).catch(err => {
                        if (isTurnCancelledError(err) || cancelRequestedRef.current) {
                            endTurnActivity();
                            currentTurnRef.current = null;
                            return;
                        }
                        endTurnActivity();
                        setIsThinking(false);
                        setError(getAsyncErrorMessage(err));
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
                })();
            });
            cleanups.push(() => unProactive());

            // Listen for interaction triggers (touch/click on Live2D model)
            // interaction-service already calls streamChat, we just need to prepare ChatPanel for receiving deltas
            const unInteraction = await listen<any>("interaction-trigger", () => {
                if (aborted || isBusyRef.current) return;

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
                if (aborted || isBusyRef.current) return;
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
            if (cancellationWatchdogTimerRef.current !== null) {
                clearTimeout(cancellationWatchdogTimerRef.current);
                cancellationWatchdogTimerRef.current = null;
            }
            cleanups.forEach(fn => fn());
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // ── Send message ───────────────────────────────────────
    const handleSend = async (e?: React.FormEvent) => {
        e?.preventDefault();
        if (interactionDisabled) return;
        const trimmed = input.trim();
        const messageImages = visionEnabled ? [...pendingImages] : [];
        if ((!trimmed && messageImages.length === 0) || isBusy) return;
        if (!await ensureMemoryModelReady()) return;

        const clientRequestId = `req_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
        setMessages(prev => [...prev, {
            role: "user",
            text: trimmed,
            images: messageImages.length > 0 ? messageImages : undefined,
            clientRequestId,
        }]);
        const cameraFrame = visionEnabled ? getLatestCameraFrame() : null;
        const imagesToSend = cameraFrame ? [...messageImages, cameraFrame] : messageImages;
        clearDraft();
        setPendingImages([]);
        startStreaming();
        setIsThinking(true);
        userScrolledRef.current = false;
        savedScrollSnapshotRef.current = null;
        // Lock out handleScroll until deferredMessages DOM update settles (~200ms)
        isProgrammaticScrollRef.current = true;
        setTimeout(() => { isProgrammaticScrollRef.current = false; }, 200);
        resetReveal();
        rawResponseRef.current = "";
        currentTurnRef.current = null;

        const allowImageGen = isGeneratedBackgroundMode();

        try {
            const res = await streamChat({
                message: trimmed || "(image attached)",
                allow_image_gen: allowImageGen,
                images: imagesToSend.length > 0 ? imagesToSend : undefined,
                character_id: getActiveCharacterIdForRequest(),
                client_request_id: clientRequestId,
            });
            if (res?.conversation_id) {
                setActiveConversationId(res.conversation_id);
                activeConversationIdRef.current = res.conversation_id;
            }
            if (res?.user_message_id) {
                setMessages(prev => {
                    const idx = prev.findIndex(m => m.clientRequestId === clientRequestId);
                    if (idx !== -1 && !prev[idx].id) {
                        const updated = [...prev];
                        updated[idx] = { ...updated[idx], id: res.user_message_id ?? undefined };
                        return updated;
                    }
                    return prev;
                });
            }
        } catch (err) {
            if (isTurnCancelledError(err) || cancelRequestedRef.current) {
                endTurnActivity();
                currentTurnRef.current = null;
                setIsThinking(false);
                return;
            }
            endTurnActivity();
            currentTurnRef.current = null;
            setIsThinking(false);
            setError(getAsyncErrorMessage(err));

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

    // ── Drag and Drop image upload ──────────────────────────
    const handleDragEnter = useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        dragCounterRef.current += 1;
        if (e.dataTransfer.types.includes("Files")) {
            setIsDraggingOver(true);
        }
    }, []);

    const handleDragLeave = useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        dragCounterRef.current -= 1;
        if (dragCounterRef.current <= 0) {
            dragCounterRef.current = 0;
            setIsDraggingOver(false);
        }
    }, []);

    const handleDragOver = useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "copy";
    }, []);

    const handleDrop = useCallback(async (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        dragCounterRef.current = 0;
        setIsDraggingOver(false);

        if (!visionEnabled) {
            setError(t("chat.errors.vision_disabled") ?? "Vision is not enabled");
            return;
        }

        const rawFiles = Array.from(e.dataTransfer.files);
        const files = rawFiles.filter(f => f.type.startsWith("image/"));
        if (files.length === 0) {
            if (rawFiles.length > 0) {
                setError(t("chat.errors.only_images"));
            }
            return;
        }

        for (const file of files) {
            if (file.size > 5 * 1024 * 1024) {
                setError(t("chat.errors.image_too_large"));
                continue;
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
            }
        }
    }, [visionEnabled, t]);

    // ── STT: Advanced VAD Microphone toggle ─────────────────
    const handleMicToggle = useCallback(() => {
        if (voiceState === VoiceState.Idle) {
            sttBaseDraftRef.current = inputRef.current;
            startVoice({ autoStopOnSilence: true });
        } else {
            stopVoice();
        }
    }, [voiceState, startVoice, stopVoice]);

    // ── Clear history ──────────────────────────────────────
    const handleClearClick = () => {
        if (messages.length === 0 || isBusy || isStreaming) return;
        setShowClearConfirm(true);
    };

    const executeClear = async () => {
        setShowClearConfirm(false);
        try {
            await clearHistory();
        } catch {
            // Backend might not be ready
        }
        setMessages([]);
        setShowScrollBottom(false);
        setHasNewMessagesBelow(false);
        savedScrollSnapshotRef.current = null;
        userScrolledRef.current = false;
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

    const onEdit = useCallback(async (globalIndex: number, newText: string) => {
        const trimmed = newText.trim();
        if (!trimmed) return;

        const targetMsg = messagesRef.current[globalIndex];
        if (!targetMsg) return;

        const previousText = targetMsg.text;
        const targetId = targetMsg.id;
        const targetClientRequestId = targetMsg.clientRequestId;

        // 1. 本地乐观更新 UI
        setMessages(prev => {
            const updated = [...prev];
            if (updated[globalIndex]) {
                updated[globalIndex] = { ...updated[globalIndex], text: trimmed };
            }
            return updated;
        });

        // 2. 异步持久化到 SQLite 并同步后端 LLM 上下文
        try {
            let messageId = targetMsg.id;
            const convId = activeConversationIdRef.current ?? undefined;
            if (!messageId) {
                // 若刚发送未完成握手，等待极短时间（最多 600ms）确保 ID 到达
                for (let i = 0; i < 12; i++) {
                    await new Promise(r => setTimeout(r, 50));
                    const latest = messagesRef.current[globalIndex];
                    if (latest?.id) {
                        messageId = latest.id;
                        break;
                    }
                }
            }

            if (!messageId) {
                // 坚决禁止在无数据库 message_id 的情况下盲改数据库
                throw new Error("Message ID not yet synchronized, cannot edit");
            }

            const res = await editConversationMessage({
                conversation_id: convId,
                message_id: messageId,
                new_content: trimmed,
            });
            // 3. 回填生成的新 message_id 并同步后端截断后的内容
            if (res?.message_id) {
                setMessages(prev => {
                    const targetIdx = prev.findIndex(m => m.id === res.message_id);
                    const idx = targetIdx !== -1 ? targetIdx : globalIndex;
                    if (prev[idx]) {
                        const updated = [...prev];
                        updated[idx] = {
                            ...updated[idx],
                            id: res.message_id,
                            text: res.updated_content ?? updated[idx].text,
                        };
                        return updated;
                    }
                    return prev;
                });
            }
        } catch (e) {
            console.error("[ChatPanel] Failed to persist message edit:", e);
            // 1. 回滚恢复旧消息文本，避免乐观更新在持久化失败后残留脏数据
            setMessages(prev => {
                let targetIdx = targetId ? prev.findIndex(m => m.id === targetId) : -1;
                if (targetIdx === -1 && targetClientRequestId) {
                    targetIdx = prev.findIndex(m => m.clientRequestId === targetClientRequestId);
                }
                if (targetIdx === -1 && prev[globalIndex] && prev[globalIndex].text === trimmed) {
                    targetIdx = globalIndex;
                }
                if (targetIdx !== -1 && prev[targetIdx].text === trimmed) {
                    const updated = [...prev];
                    updated[targetIdx] = { ...updated[targetIdx], text: previousText };
                    return updated;
                }
                return prev;
            });

            setError(t("chat.errors.edit_failed") ?? "Failed to save edited message");

            // 2. 若当前不在流式生成中，尝试重新同步会话以确保与数据库绝对对齐
            if (!isStreamingRef.current && activeConversationIdRef.current) {
                void conversationSyncRef.current?.synchronize({
                    characterId: activeCharacterId,
                    preferredConversationId: activeConversationIdRef.current,
                }).catch(err => {
                    console.warn("[ChatPanel] Background re-synchronization after edit failure failed:", err);
                });
            }
        }
    }, [activeCharacterId, t]);

    const onRegenerate = useCallback(async (globalIndex: number) => {
        const msgs = messagesRef.current;
        const lastUserIndex = msgs.slice(0, globalIndex).reverse().findIndex(m => m.role === "user");
        if (lastUserIndex === -1) return;
        const userMsgIndex = globalIndex - 1 - lastUserIndex;
        const userMsg = msgs[userMsgIndex];
        if (!await ensureMemoryModelReady()) return;

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

        const allowImageGen = isGeneratedBackgroundMode();

        streamChat({
            message: userMsg.text,
            images: userMsg.images,
            allow_image_gen: allowImageGen,
            character_id: getActiveCharacterIdForRequest(),
            regenerate: true,
        }).catch(err => {
            if (isTurnCancelledError(err) || cancelRequestedRef.current) {
                endTurnActivity();
                currentTurnRef.current = null;
                setIsThinking(false);
                return;
            }
            endTurnActivity();
            currentTurnRef.current = null;
            setIsThinking(false);
            setError(getAsyncErrorMessage(err));
        });
    }, [endTurnActivity, ensureMemoryModelReady, startStreaming, resetReveal, setError]);

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

    useEffect(() => {
        latestResizeWidthRef.current = width;
    }, [width]);

    const handleResizePointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
        if (!onWidthChange || event.button !== 0) {
            return;
        }

        event.preventDefault();
        event.stopPropagation();
        resizeCleanupRef.current?.();

        const startX = event.clientX;
        const startWidth = Math.max(minWidth, width);
        let pendingWidth = startWidth;
        let animationFrame: number | null = null;
        const previousCursor = document.body.style.cursor;
        const previousUserSelect = document.body.style.userSelect;

        document.body.style.cursor = "ew-resize";
        document.body.style.userSelect = "none";

        const previewWidth = (nextWidth: number) => {
            const appliedWidth = onWidthPreview ? onWidthPreview(nextWidth) : nextWidth;
            latestResizeWidthRef.current = appliedWidth;
            return appliedWidth;
        };

        const flushPreview = () => {
            animationFrame = null;
            previewWidth(pendingWidth);
        };

        const handlePointerMove = (moveEvent: PointerEvent) => {
            pendingWidth = startWidth + moveEvent.clientX - startX;
            if (animationFrame === null) {
                animationFrame = window.requestAnimationFrame(flushPreview);
            }
        };

        const cleanup = () => {
            if (animationFrame !== null) {
                window.cancelAnimationFrame(animationFrame);
                animationFrame = null;
            }
            const finalWidth = previewWidth(pendingWidth);
            window.removeEventListener("pointermove", handlePointerMove);
            window.removeEventListener("pointerup", cleanup);
            window.removeEventListener("pointercancel", cleanup);
            document.body.style.cursor = previousCursor;
            document.body.style.userSelect = previousUserSelect;
            resizeCleanupRef.current = null;
            onWidthChange(finalWidth);
        };

        resizeCleanupRef.current = cleanup;
        window.addEventListener("pointermove", handlePointerMove);
        window.addEventListener("pointerup", cleanup, { once: true });
        window.addEventListener("pointercancel", cleanup, { once: true });
    }, [minWidth, onWidthChange, onWidthPreview, width]);

    const handleResizeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
        if (!onWidthChange || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")) {
            return;
        }

        event.preventDefault();
        const direction = event.key === "ArrowRight" ? 1 : -1;
        const multiplier = event.shiftKey ? 2 : 1;
        const nextWidth = latestResizeWidthRef.current + direction * CHAT_PANEL_KEYBOARD_RESIZE_STEP * multiplier;
        const finalWidth = onWidthPreview ? onWidthPreview(nextWidth) : nextWidth;
        latestResizeWidthRef.current = finalWidth;
        onWidthChange(finalWidth);
    }, [onWidthChange, onWidthPreview]);

    useEffect(() => {
        return () => {
            resizeCleanupRef.current?.();
        };
    }, []);

    // ── Collapse / Expand handlers ─────────────────────────
    const handleCollapse = useCallback(() => {
        const container = messagesContainerRef.current;
        if (container) {
            const atBottom = isScrollAtBottom(
                container.scrollTop,
                container.scrollHeight,
                container.clientHeight
            );
            userScrolledRef.current = !atBottom;
            savedScrollSnapshotRef.current = {
                scrollTop: container.scrollTop,
                scrollHeight: container.scrollHeight,
                clientHeight: container.clientHeight,
                isAtBottom: atBottom,
            };
        }
        setCollapsed(true);
    }, []);

    const handleExpand = useCallback(() => {
        setCollapsed(false);
        setUnreadCount(0);
    }, []);

    // ════════════════════════════════════════════════════════�?
    // Collapsed state �?small floating chat bubble
    // ════════════════════════════════════════════════════════�?
    if (collapsed) {
        return (
            <div
                {...interactionProps}
                onClickCapture={blockDisabledInteraction}
                onPointerDownCapture={blockDisabledInteraction}
                onKeyDownCapture={blockDisabledInteraction}
                onFocusCapture={blockDisabledInteraction}
                className={clsx("flex flex-col items-start justify-start h-full pt-4 pl-4", interactionDisabled && "pointer-events-none opacity-60")}
            >
                <motion.button
                    initial={{ scale: 0.8, opacity: 0 }}
                    animate={{ scale: 1, opacity: 1 }}
                    whileHover={{ scale: 1.1 }}
                    whileTap={{ scale: 0.9 }}
                    onClick={handleExpand}
                    data-onboarding-id="chat-open-button"
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
    const panelResizeMaxWidth = getChatPanelResizeMaxWidth(minWidth);
    const panelResizeValue = Math.min(Math.max(Math.round(width), minWidth), panelResizeMaxWidth);

    return (
        <motion.div
            {...interactionProps}
            onClickCapture={blockDisabledInteraction}
            onPointerDownCapture={blockDisabledInteraction}
            onKeyDownCapture={blockDisabledInteraction}
            onFocusCapture={blockDisabledInteraction}
            onDragEnter={handleDragEnter}
            onDragLeave={handleDragLeave}
            onDragOver={handleDragOver}
            onDrop={handleDrop}
            initial={{ opacity: 0, x: -20 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
            className={clsx(
                "flex flex-col h-full w-full",
                "bg-[var(--color-bg-surface)] backdrop-blur-[var(--glass-blur)]",
                "border border-[var(--color-border)] rounded-xl shadow-lg",
                "relative overflow-hidden",
                interactionDisabled && "pointer-events-none opacity-60"
            )}
        >
            {onWidthChange && (
                <div
                    role="separator"
                    aria-label={t("chat.actions.resize")}
                    aria-orientation="vertical"
                    aria-valuemin={minWidth}
                    aria-valuemax={panelResizeMaxWidth}
                    aria-valuenow={panelResizeValue}
                    tabIndex={0}
                    onPointerDown={handleResizePointerDown}
                    onKeyDown={handleResizeKeyDown}
                    className={clsx(
                        "absolute right-0 top-0 bottom-0 z-30 w-2 cursor-ew-resize touch-none",
                        "focus-visible:outline-none",
                        "after:absolute after:right-0 after:top-4 after:bottom-4 after:w-px",
                        "after:bg-transparent after:transition-colors after:duration-150",
                        "hover:after:bg-[var(--color-accent)]/80 focus-visible:after:bg-[var(--color-accent)]"
                    )}
                />
            )}

            {/* 拖拽放置指示遮罩 */}
            <AnimatePresence>
                {isDraggingOver && (
                    <motion.div
                        key="chat-dropzone-overlay"
                        data-testid="chat-dropzone-overlay"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="absolute inset-0 z-40 bg-black/70 backdrop-blur-sm border-2 border-dashed border-[var(--color-accent)] rounded-xl flex flex-col items-center justify-center p-6 text-center pointer-events-none"
                    >
                        <div className="p-4 rounded-full bg-[var(--color-accent)]/20 text-[var(--color-accent)] mb-3">
                            <ImagePlus size={36} strokeWidth={1.5} className="animate-pulse" />
                        </div>
                        <p className="text-sm font-semibold text-white">
                            {t("chat.input.drop_image_title", "松开鼠标上传图片")}
                        </p>
                        <p className="text-xs text-[var(--color-text-muted)] mt-1">
                            {t("chat.input.drop_image_hint", "支持 PNG, JPG, WebP 格式 (单张最大 5MB)")}
                        </p>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* 大图预览 Lightbox */}
            <ImageLightbox
                imageUrl={previewImageUrl}
                onClose={() => setPreviewImageUrl(null)}
            />

            {/* Error toast */}
            <AnimatePresence>
                {error && <ErrorToast message={error} onDismiss={() => setError(null)} />}
            </AnimatePresence>

            {/* 对话历史侧边栏 */}
            <ConversationSidebar
                open={sidebarOpen}
                onClose={() => setSidebarOpen(false)}
                characterId={activeCharacterId}
                activeConversationId={activeConversationId}
                onStartEmptyConversation={handleStartEmptyConversation}
                onSelectConversation={async (conversationId) => {
                    await handleConversationSelection(conversationId);
                    setSidebarOpen(false);
                }}
            />

            {/* 清空会话二次确认模态窗 */}
            <AnimatePresence>
                {showClearConfirm && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="absolute inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4"
                        onClick={() => setShowClearConfirm(false)}
                    >
                        <motion.div
                            initial={{ scale: 0.9, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.9, opacity: 0 }}
                            onClick={(e) => e.stopPropagation()}
                            className="w-full max-w-[280px] bg-[var(--color-bg-secondary,#1e293b)] border border-[var(--color-border)] rounded-xl p-4 shadow-2xl space-y-3"
                        >
                            <div className="flex items-center gap-2 text-[var(--color-error,#ef4444)]">
                                <Trash2 size={18} />
                                <span className="font-semibold text-sm">
                                    {t("chat.actions.confirm_clear_title")}
                                </span>
                            </div>
                            <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                                {t("chat.actions.confirm_clear")}
                            </p>
                            <div className="flex items-center justify-end gap-2 pt-1">
                                <button
                                    onClick={() => setShowClearConfirm(false)}
                                    className="px-3 py-1.5 rounded-lg text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-slate-700/50 transition-colors"
                                >
                                    {t("chat.actions.cancel")}
                                </button>
                                <button
                                    onClick={executeClear}
                                    className="px-3 py-1.5 rounded-lg text-xs font-medium bg-red-500/20 text-red-400 hover:bg-red-500/30 border border-red-500/30 transition-colors"
                                >
                                    {t("chat.actions.confirm_clear_button")}
                                </button>
                            </div>
                        </motion.div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Header �?clean and minimal */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border)]">
                <div className="flex items-center gap-2 min-w-0">
                    <div className={clsx(
                        "w-2 h-2 rounded-full flex-shrink-0",
                        isStreaming
                            ? "bg-amber-500 animate-pulse"
                            : "bg-[var(--color-accent)] shadow-[var(--glow-success)]"
                    )} />
                    <span className="font-heading text-sm font-semibold tracking-wider uppercase text-[var(--color-text-secondary)] flex-shrink-0">
                        {isStreaming ? t("chat.status.streaming") : t("chat.status.chat")}
                    </span>
                    {characterName && (
                        <span className="flex items-center gap-1.5 min-w-0">
                            <span className="text-[var(--color-text-muted)] opacity-30 text-xs select-none">/</span>
                            <span
                                className="text-xs font-medium text-[var(--color-text-primary)] truncate max-w-[130px]"
                                title={characterName}
                            >
                                {characterName}
                            </span>
                        </span>
                    )}
                </div>
                <div className="flex items-center gap-1">
                    <motion.button
                        data-chat-history-toggle="true"
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
                        whileHover={messages.length > 0 && !isBusy && !isStreaming ? { scale: 1.1 } : undefined}
                        whileTap={messages.length > 0 && !isBusy && !isStreaming ? { scale: 0.95 } : undefined}
                        onClick={handleClearClick}
                        disabled={messages.length === 0 || isBusy || isStreaming}
                        className={clsx(
                            "p-2 rounded-md transition-colors",
                            messages.length === 0 || isBusy || isStreaming
                                ? "text-[var(--color-text-muted)]/30 cursor-not-allowed"
                                : "text-[var(--color-text-muted)] hover:text-[var(--color-error)]"
                        )}
                        aria-label={t("chat.actions.clear")}
                        title={t("chat.actions.clear")}
                    >
                        <Trash2 size={14} strokeWidth={1.5} />
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={handleCollapse}
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
                onPointerDown={(e) => {
                    if (e.target === e.currentTarget) {
                        textareaRef.current?.blur();
                    }
                }}
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
                                isStreaming={isBusy}
                                isTranslationExpanded={expandedTranslations.has(globalIndex)}
                                onToggleTranslation={onToggleTranslation}
                                onEdit={onEdit}
                                onRegenerate={onRegenerate}
                                onContinueFrom={onContinueFrom}
                                onApproveTool={onApproveTool}
                                onRejectTool={onRejectTool}
                                onPreviewImage={setPreviewImageUrl}
                            />
                        );
                    })}

                    {shouldRenderTypingIndicator({ isThinking, messages: deferredMessages, activeMessageIndex: currentTurnRef.current?.messageIndex ?? null }) && <TypingIndicator />}
                </AnimatePresence>
                <div ref={messagesEndRef} />
            </div>

            {/* Input */}
            <form onSubmit={handleSend} className="relative border-t border-[var(--color-border)] bg-black/20 pt-1">
                {/* Messages 区域上方悬浮的回到底部 / 新消息胶囊 */}
                <AnimatePresence>
                    {showScrollBottom && (
                        <div className="relative w-full">
                            <motion.button
                                type="button"
                                initial={{ opacity: 0, y: 10, scale: 0.9 }}
                                animate={{ opacity: 1, y: 0, scale: 1 }}
                                exit={{ opacity: 0, y: 10, scale: 0.9 }}
                                whileHover={{ scale: 1.05 }}
                                whileTap={{ scale: 0.95 }}
                                onClick={scrollToBottomSmooth}
                                className={clsx(
                                    "absolute right-5 -top-12 z-20 flex items-center gap-1.5 px-3 py-1.5 rounded-full shadow-xl backdrop-blur-md transition-colors",
                                    hasNewMessagesBelow
                                        ? "bg-[var(--color-accent,#6366f1)] text-white font-medium border border-white/20 shadow-[0_0_15px_rgba(99,102,241,0.5)]"
                                        : "bg-slate-900/80 border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:border-[var(--color-accent)]"
                                )}
                                title={hasNewMessagesBelow ? t("chat.actions.new_messages") : t("chat.actions.to_bottom")}
                            >
                                <ChevronDown size={14} className={hasNewMessagesBelow ? "animate-bounce" : ""} />
                                <span className="text-xs">
                                    {hasNewMessagesBelow ? t("chat.actions.new_messages") : t("chat.actions.to_bottom")}
                                </span>
                            </motion.button>
                        </div>
                    )}
                </AnimatePresence>

                {/* Drag handle on top edge */}
                <div
                    onPointerDown={handleInputResizeStart}
                    onPointerMove={handleInputResizeMove}
                    onPointerUp={handleInputResizeEnd}
                    onPointerCancel={handleInputResizeEnd}
                    onDoubleClick={handleInputResizeReset}
                    className="w-full h-3 cursor-ns-resize flex items-center justify-center group select-none -mt-1 touch-none"
                    title={t("chat.input.resize_hint", "拖拽调整高度 · 双击切换/复位")}
                >
                    <div className="w-10 h-1 rounded-full bg-white/10 group-hover:bg-[var(--color-accent)]/60 transition-colors" />
                </div>

                {/* Pending images preview */}
                <AnimatePresence>
                    {hasSendableImages && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="flex gap-2 px-3 pb-2 overflow-x-auto"
                        >
                            {pendingImages.map((url, idx) => (
                                <div key={idx} className="relative group flex-shrink-0">
                                    <img
                                        src={url}
                                        alt="pending"
                                        className="w-14 h-14 rounded-md object-cover border border-[var(--color-border)]"
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

                <div
                    style={{ height: `${inputHeight}px` }}
                    className={clsx(
                        "relative mx-3 mb-3 p-2.5 bg-black/40 border border-[var(--color-border)] rounded-2xl flex flex-col",
                        "hover:border-white/20",
                        "focus-within:!border-[var(--color-accent)] focus-within:shadow-[0_0_10px_rgba(0,240,255,0.25)]",
                        "transition-colors",
                        isBusy && "opacity-50 cursor-not-allowed"
                    )}
                >
                    <textarea
                        ref={textareaRef}
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        onPaste={handlePaste}
                        onKeyDown={(e) => {
                            if ((e.key === "Enter" && !e.shiftKey) || (e.key === "Enter" && (e.ctrlKey || e.metaKey))) {
                                if (e.nativeEvent.isComposing) return;
                                e.preventDefault();
                                handleSend();
                            }
                        }}
                        data-onboarding-id="chat-input"
                        placeholder={t("chat.input.placeholder")}
                        disabled={isBusy}
                        style={{ outline: "none", boxShadow: "none" }}
                        className={clsx(
                            "w-full flex-1 bg-transparent border-none text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                            "text-sm font-body resize-none p-0 pr-1 leading-normal",
                            "!outline-none focus:!outline-none focus-visible:!outline-none focus:ring-0 focus-visible:ring-0",
                            "scrollbar-thin scrollbar-thumb-white/10 scrollbar-track-transparent",
                            "[&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-white/10 [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-button]:hidden"
                        )}
                    />

                    <div className="flex items-center justify-between pt-1.5 mt-auto">
                        <div className="flex items-center gap-1.5">
                            {/* Hidden file input */}
                            <input
                                ref={fileInputRef}
                                type="file"
                                accept="image/*"
                                className="hidden"
                                onChange={handleImageSelect}
                            />

                            {/* Image upload button — only visible when Vision Mode is ON */}
                            {visionEnabled && (
                                <motion.button
                                    type="button"
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => fileInputRef.current?.click()}
                                    disabled={isBusy || isUploading}
                                    className={clsx(
                                        "p-1.5 rounded-lg transition-colors text-[var(--color-text-muted)] hover:text-[var(--color-accent)]",
                                        (isBusy || isUploading) && "opacity-50 cursor-not-allowed"
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

                            {/* Camera frame indicator */}
                            {visionEnabled && cameraEnabled && (
                                <div
                                    className="flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] text-[var(--color-accent)] opacity-70 select-none"
                                    title={t("chat.input.camera_frame_attached")}
                                >
                                    <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)] animate-pulse" />
                                    CAM
                                </div>
                            )}

                            {/* Microphone button — Advanced VAD Mode */}
                            {sttEnabled && (
                                <div className="relative flex items-center justify-center">
                                    {/* Volume ring */}
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
                                        disabled={isBusy}
                                        className={clsx(
                                            "relative p-1.5 rounded-lg transition-all z-10",
                                            voiceState === VoiceState.Idle
                                                ? "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                                : voiceState === VoiceState.Listening
                                                    ? "text-[var(--color-accent)] bg-[var(--color-accent)]/15 border border-[var(--color-accent)]/30"
                                                    : voiceState === VoiceState.Speaking
                                                        ? "text-red-400 bg-red-500/20 border border-red-500/40 shadow-[0_0_12px_rgba(239,68,68,0.3)]"
                                                        : "text-amber-400 bg-amber-500/15 border border-amber-500/30",
                                            isBusy && voiceState === VoiceState.Idle && "opacity-50 cursor-not-allowed"
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
                        </div>

                        {/* Send / Stop button */}
                        <motion.button
                            whileHover={{ scale: 1.1 }}
                            whileTap={{ scale: 0.9 }}
                            type="submit"
                            onClick={isStreaming ? (e) => {
                                e.preventDefault();
                                handleStopGeneration();
                            } : undefined}
                            disabled={isStreaming ? isStopping : (isBusy || (!input.trim() && !hasSendableImages))}
                            className={clsx(
                                "p-2 rounded-xl transition-colors",
                                isStreaming
                                    ? "bg-red-500 text-white hover:bg-red-400"
                                    : "bg-[var(--color-accent)] text-black hover:bg-white",
                                (isStreaming ? isStopping : (isBusy || (!input.trim() && !hasSendableImages))) && "opacity-50 cursor-not-allowed"
                            )}
                            aria-label={isStreaming ? t("chat.actions.stop") : "Send message"}
                            title={isStreaming ? (isStopping ? t("chat.actions.stopping") : t("chat.actions.stop")) : undefined}
                        >
                            {isStreaming ? (
                                <X size={15} strokeWidth={1.8} />
                            ) : (
                                <Send size={15} strokeWidth={1.5} />
                            )}
                        </motion.button>
                    </div>
                </div>
            </form>
        </motion.div >
    );
}
