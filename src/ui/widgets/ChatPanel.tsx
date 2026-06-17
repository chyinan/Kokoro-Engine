import { useState, useRef, useEffect, useCallback, useDeferredValue, memo, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Send, Trash2, AlertCircle, MessageCircle, ChevronLeft, ImagePlus, X, Mic, MicOff, History, Maximize2, Minimize2 } from "lucide-react";
import { streamChat, cancelChatTurn, onChatTurnStart, onChatTurnDelta, onChatTurnFinish, onChatTurnTextComplete, onChatError, onChatWarning, onChatFailure, onChatTurnTranslation, clearHistory, uploadVisionImage, synthesize, onChatTurnTool, listConversations, loadConversation, onTelegramChatSync, onVisionObservation, deleteLastMessages, approveToolApproval, rejectToolApproval, getMemoryEmbeddingModelStatus, setVisionTextInputFocused } from "../../lib/kokoro-bridge";
import type { FailureEvent, ToolTraceItem } from "../../lib/kokoro-bridge";
import { getLatestCameraFrame } from "../../lib/camera-frame-cache";
import { listen } from "@tauri-apps/api/event";
import { useVoiceInput, VoiceState, useTypingReveal, useWakeWord } from "../hooks";
import { useTranslation } from "react-i18next";
import ConversationSidebar from "./ConversationSidebar";
import { ChatMessage } from "./ChatMessage";
import { buildChatMessagesFromConversation } from "./chat-history";
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
import { requestMemoryModelDialog } from "../../lib/memory-model-gate";
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

export default function ChatPanel({
    width = DEFAULT_CHAT_PANEL_WIDTH,
    minWidth = DEFAULT_CHAT_PANEL_WIDTH,
    onWidthPreview,
    onWidthChange,
}: ChatPanelProps) {
    const { t } = useTranslation();
    const [collapsed, setCollapsed] = useState(false);
    const [messages, setMessages] = useState<ChatMessage[]>([]);
    const deferredMessages = useDeferredValue(messages);
    const [visibleCount, setVisibleCount] = useState(20);
    const [input, setInput] = useState("");
    const [expandedInput, setExpandedInput] = useState(false);
    const compactInputRef = useRef<HTMLInputElement>(null);
    const expandedTextareaRef = useRef<HTMLTextAreaElement>(null);
    const [isStreaming, setIsStreaming] = useState(false);
    const isStreamingRef = useRef(false);
    const [isBusy, setIsBusy] = useState(false);
    const isBusyRef = useRef(false);
    const ttsSpeakingRef = useRef(false);
    const [isStopping, setIsStopping] = useState(false);
    const cancelRequestedRef = useRef(false);
    const messagesRef = useRef<ChatMessage[]>([]);
    const [isThinking, setIsThinking] = useState(false);

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
    const fileInputRef = useRef<HTMLInputElement>(null);
    const resizeCleanupRef = useRef<(() => void) | null>(null);
    const latestResizeWidthRef = useRef(width);
    // Store last failed request for retry
    const lastFailedRequestRef = useRef<{ message: string; images?: string[]; allowImageGen?: boolean } | null>(null);

    const ensureMemoryModelReady = useCallback(async (options?: { silent?: boolean }) => {
        try {
            const status = await getMemoryEmbeddingModelStatus();
            if (status.installed) {
                return true;
            }
        } catch (err) {
            console.error("[ChatPanel] Failed to query memory model status:", err);
            if (!options?.silent) {
                setError(t("chat.errors.memory_model_check_failed"));
            }
            requestMemoryModelDialog();
            return false;
        }

        if (!options?.silent) {
            setError(t("chat.errors.memory_model_required"));
        }
        requestMemoryModelDialog();
        return false;
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
    const [pendingImages, setPendingImages] = useState<string[]>([]);
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

    const handleStopGeneration = useCallback(() => {
        if (!isStreamingRef.current || isStopping) {
            return;
        }

        cancelRequestedRef.current = true;
        setIsStopping(true);
        setIsThinking(false);

        const activeTurnId = currentTurnRef.current?.turnId;
        if (activeTurnId) {
            void requestTurnCancellation(activeTurnId);
        }
    }, [isStopping, requestTurnCancellation]);

    // 自动恢复最近对话
    useEffect(() => {
        const characterId = getActiveCharacterIdForConversationRestore();
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
        if (!trimmed) return;

        if (sttAutoSend) {
            void (async () => {
                if (!await ensureMemoryModelReady()) {
                    setInput(trimmed);
                    return;
                }

                // Auto-send: inject directly into chat
                setInput("");
                setMessages(prev => [...prev, { role: "user", text: trimmed }]);
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;

                const allowImageGen = isGeneratedBackgroundMode();

                streamChat({
                    message: trimmed,
                    allow_image_gen: allowImageGen,
                    character_id: getActiveCharacterIdForRequest(),
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
            // Fill input box for user review
            setInput(trimmed);
        }
    }, [endTurnActivity, ensureMemoryModelReady, sttAutoSend, startStreaming]);

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
            const focused = active === compactInputRef.current || active === expandedTextareaRef.current;
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
                    translationPending: false,
                    tools: [],
                    pendingContext: pendingVisionContextRef.current ?? undefined,
                };
                pendingVisionContextRef.current = null;
                rawResponseRef.current = "";
                if (cancelRequestedRef.current) {
                    void requestTurnCancellation(turn_id);
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
                userScrolledRef.current = false;

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

            const unDone = await onChatTurnFinish(({ turn_id, status }) => {
                if (aborted) return;
                const turn = currentTurnRef.current;
                if (!turn || turn.turnId !== turn_id) return;

                flushReveal();
                endTurnActivity();
                setIsThinking(false);
                userScrolledRef.current = false;

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
            cleanups.forEach(fn => fn());
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // ── Send message ───────────────────────────────────────
    const handleSend = async (e?: React.FormEvent) => {
        e?.preventDefault();
        const trimmed = input.trim();
        const messageImages = visionEnabled ? [...pendingImages] : [];
        if ((!trimmed && messageImages.length === 0) || isBusy) return;
        if (!await ensureMemoryModelReady()) return;

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

        const allowImageGen = isGeneratedBackgroundMode();

        try {
            await streamChat({
                message: trimmed || "(image attached)",
                allow_image_gen: allowImageGen,
                images: imagesToSend.length > 0 ? imagesToSend : undefined,
                character_id: getActiveCharacterIdForRequest(),
            });
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
                                isStreaming={isBusy}
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
                            disabled={isBusy || isUploading}
                            className={clsx(
                                "p-2.5 rounded-lg transition-colors",
                                "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]",
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
                                disabled={isBusy}
                                className={clsx(
                                    "relative p-2.5 rounded-lg transition-all z-10",
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

                    <div className="relative flex-1">
                        <input
                            ref={compactInputRef}
                            type="text"
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onPaste={handlePaste}
                            data-onboarding-id="chat-input"
                            placeholder={t("chat.input.placeholder")}
                            disabled={isBusy}
                            className={clsx(
                                "w-full bg-black/40 border border-[var(--color-border)]",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "text-sm rounded-lg pl-4 pr-8 py-2.5 font-body",
                                "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                                "transition-all",
                                isBusy && "opacity-50 cursor-not-allowed"
                            )}
                        />
                        <button
                            type="button"
                            disabled={isBusy}
                            onClick={() => {
                                setExpandedInput(true);
                                setTimeout(() => {
                                    const ta = expandedTextareaRef.current;
                                    if (ta) { ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); }
                                }, 50);
                            }}
                            className={clsx(
                                "absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors",
                                isBusy && "opacity-50 cursor-not-allowed"
                            )}
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
                        onClick={isStreaming ? (e) => {
                            e.preventDefault();
                            handleStopGeneration();
                        } : undefined}
                        disabled={isStreaming ? isStopping : (isBusy || (!input.trim() && !hasSendableImages))}
                        className={clsx(
                            "p-2.5 rounded-lg transition-colors",
                            isStreaming
                                ? "bg-red-500 text-white hover:bg-red-400"
                                : "bg-[var(--color-accent)] text-black hover:bg-white",
                            (isStreaming ? isStopping : (isBusy || (!input.trim() && !hasSendableImages))) && "opacity-50 cursor-not-allowed"
                        )}
                        aria-label={isStreaming ? t("chat.actions.stop") : "Send message"}
                        title={isStreaming ? (isStopping ? t("chat.actions.stopping") : t("chat.actions.stop")) : undefined}
                    >
                        {isStreaming ? (
                            <X size={16} strokeWidth={1.8} />
                        ) : (
                            <Send size={16} strokeWidth={1.5} />
                        )}
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
                            disabled={isBusy}
                            rows={6}
                            className={clsx(
                                "w-full bg-black/40 border border-[var(--color-border)] rounded-lg",
                                "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                                "text-sm px-4 py-3 font-body resize-none",
                                "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                                "transition-all",
                                isBusy && "opacity-50 cursor-not-allowed"
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
                                    onClick={() => {
                                        if (isStreaming) {
                                            handleStopGeneration();
                                            return;
                                        }
                                        setExpandedInput(false);
                                        handleSend();
                                    }}
                                    disabled={isStreaming ? isStopping : (isBusy || !input.trim())}
                                    className={clsx(
                                        "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors",
                                        isStreaming
                                            ? "bg-red-500 text-white hover:bg-red-400"
                                            : "bg-[var(--color-accent)] text-black hover:bg-white",
                                        (isStreaming ? isStopping : (isBusy || !input.trim())) && "opacity-50 cursor-not-allowed"
                                    )}
                                >
                                    {isStreaming ? (
                                        <>
                                            <X size={12} strokeWidth={1.8} />
                                            {isStopping ? t("chat.actions.stopping") : t("chat.actions.stop")}
                                        </>
                                    ) : (
                                        <>
                                            <Send size={12} strokeWidth={1.5} />
                                            发送
                                        </>
                                    )}
                                </motion.button>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div >
    );
}
