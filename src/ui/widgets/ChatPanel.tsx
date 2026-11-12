import { useChatTextReveal } from "./chat/use-chat-text-reveal";
import { listen } from "@tauri-apps/api/event";
// Display adapter: drafts, voice input, layout and rendering only.
import { useState, useRef, useEffect, useLayoutEffect, useCallback, useDeferredValue, useSyncExternalStore, useMemo, memo, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type SyntheticEvent } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Send, Trash2, AlertCircle, MessageCircle, ChevronLeft, ChevronDown, ImagePlus, X, Mic, MicOff, History } from "lucide-react";
import { onChatWarning, uploadVisionImage, setVisionTextInputFocused } from "../../lib/kokoro-bridge";
import type { ToolTraceItem } from "../../lib/kokoro-bridge";
import { chatApplication, type ChatMessageTarget } from "../../lib/chat/application";
import { projectMessages, isActiveTurn } from "../../lib/chat/projection";
import { canSubmitApproval, getApprovalRequestId, type ChatPanelMessage } from "../../lib/chat/turn-state";
import { getLatestCameraFrame } from "../../lib/camera-frame-cache";
import { useVoiceInput, VoiceState, useWakeWord } from "../hooks";
import { useTranslation } from "react-i18next";
import { ImageLightbox } from "../components/ImageLightbox";
import ConversationSidebar from "./ConversationSidebar";
import { ChatMessage } from "./ChatMessage";
import { shouldRenderTypingIndicator } from "./chat-streaming-state";
import { computeTargetScrollTop, isScrollAtBottom, computeAnchoredScrollTop, type ChatScrollSnapshot } from "./chat/chat-scroll-state";
import { computeResizedInputHeight, loadSavedChatInputHeight, saveChatInputHeight, toggleChatInputResetHeight } from "./chat/chat-input-layout";
import { combineDraftWithTranscription, saveCharacterDraft } from "./chat/chat-draft-layout";
import { useCharacterChatDraft } from "./chat/use-character-draft";
import { useChatDraftSubmission } from "./chat/use-chat-draft-submission";
import { getChatPanelInteractionProps } from "../layout/layout-interaction";
import { APP_SETTING_KEYS, readBooleanSetting, readJsonSetting, readStringSetting } from "../../lib/app-settings";
type ChatMessage = ChatPanelMessage;
export type { ChatPanelMessage };
interface ChatPanelProps {
    width?: number;
    minWidth?: number;
    onWidthPreview?: (width: number) => number;
    onWidthChange?: (width: number) => void;
    /** Blocks background interaction while onboarding owns the first turn. */
    interactionDisabled?: boolean;
}
const DEFAULT_CHAT_PANEL_WIDTH = 350;
const CHAT_PANEL_RESIZE_GUTTER = 160;
const CHAT_PANEL_KEYBOARD_RESIZE_STEP = 24;

const getChatPanelResizeMaxWidth = (minWidth: number) => {
    if (typeof window === "undefined") {
        return minWidth;
    }
    return Math.max(minWidth, window.innerWidth - CHAT_PANEL_RESIZE_GUTTER);
};

function getAsyncErrorMessage(error: unknown): string {
    if (error instanceof Error) {
        return error.message;
    }
    if (typeof error === "object" && error !== null && "message" in error && typeof (error as { message?: unknown }).message === "string") {
        return (error as { message: string }).message;
    }
    return String(error);
}

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
// Claim only when an alert has a mounted renderer. A rejected promise can
// outlive its composer, leaving the saved diagnostic to announce on reopen.
function useFailureAnnouncement(requestId: string | undefined): boolean {
    const [claimedId, setClaimedId] = useState<string>();
    useLayoutEffect(() => {
        if (requestId && chatApplication.claimFailureAnnouncement(requestId)) setClaimedId(requestId);
    }, [requestId]);
    return requestId !== undefined && claimedId === requestId;
}

function ErrorToast({ message, requestId, onDismiss }: { message: string; requestId?: string; onDismiss: () => void }) {
    const claimed = useFailureAnnouncement(requestId);
    useEffect(() => {
        const timer = setTimeout(onDismiss, 4000);
        return () => clearTimeout(timer);
    }, [onDismiss]);

    if (requestId !== undefined && !claimed) return null;
    return (
        <motion.div
            role="alert"
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
    conversationId: string | null;
    generation: number;
    globalIndex: number;
    isStreaming: boolean;
    isTranslationExpanded: boolean;
    onToggleTranslation: (index: number) => void;
    onEdit: (target: ChatMessageTarget, newText: string) => void;
    onRegenerate: (target: ChatMessageTarget) => Promise<void>;
    onContinueFrom: (target: ChatMessageTarget) => Promise<void>;
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
    message, conversationId, generation, globalIndex, isStreaming, isTranslationExpanded,
    onToggleTranslation, onEdit, onRegenerate, onContinueFrom, onApproveTool, onRejectTool,
    onPreviewImage,
}: MemoizedChatMessageProps) {
    const target = conversationId !== null && message.id !== undefined
        ? { conversationId, generation, messageId: message.id }
        : undefined;
    const editableTarget = message.failureScope ? undefined : target;
    const retryTarget = message.failureScope && !message.canRetryFailure ? undefined : target;
    const announceFailure = useFailureAnnouncement(message.failureScope ? message.clientRequestId : undefined);
    return (
        <ChatMessage
            message={message.failureScope ? { ...message, announceError: announceFailure } : message}
            index={globalIndex}
            isStreaming={isStreaming}
            isTranslationExpanded={isTranslationExpanded}
            onToggleTranslation={() => onToggleTranslation(globalIndex)}
            editScope={editableTarget ? JSON.stringify([editableTarget.conversationId, editableTarget.generation, editableTarget.messageId]) : undefined}
            onEdit={editableTarget ? text => onEdit(editableTarget, text) : undefined}
            onRegenerate={retryTarget ? () => onRegenerate(retryTarget) : undefined}
            onContinueFrom={editableTarget ? () => onContinueFrom(editableTarget) : undefined}
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
    const view = useSyncExternalStore(chatApplication.subscribe, chatApplication.getSnapshot);
    const projectedMessages = useMemo(() => projectMessages(view), [view]);
    const activeCharacterId = view.character_id;
    const activeConversationId = view.conversation_id;
    const activeCharacterIdRef = useRef(activeCharacterId);
    const activeConversationIdRef = useRef(activeConversationId);
    const conversationGenerationRef = useRef(view.generation);
    activeCharacterIdRef.current = activeCharacterId;
    activeConversationIdRef.current = activeConversationId;
    conversationGenerationRef.current = view.generation;
    const activeTurn = view.turns.find(turn => isActiveTurn(turn) && turn.character_id === activeCharacterId);
    const messages = useChatTextReveal(projectedMessages, activeTurn?.turn_id);
    const isStreaming = Boolean(activeTurn);
    const isThinking = isStreaming && !activeTurn?.text;
    const [isStopping, setIsStopping] = useState(false);
    const [isSwitchingConversation, setIsSwitchingConversation] = useState(false);
    const [collapsed, setCollapsed] = useState(false);
    const [showClearConfirm, setShowClearConfirm] = useState(false);
    const [expandedTranslations, setExpandedTranslations] = useState<Set<number>>(new Set());
    const messageList = useMemo(() => ({ messages, conversationId: activeConversationId, generation: view.generation }), [messages, activeConversationId, view.generation]);
    const deferredList = useDeferredValue(messageList);
    // Defer updates within a conversation, but never show old rows under a new selection.
    const deferredMessages = deferredList.conversationId === activeConversationId && deferredList.generation === view.generation
        ? deferredList.messages : messages;
    const [visibleCount, setVisibleCount] = useState(20);
    const [showScrollBottom, setShowScrollBottom] = useState(false);
    const [hasNewMessagesBelow, setHasNewMessagesBelow] = useState(false);
    const isPrependingRef = useRef(false);
    const prevScrollHeightRef = useRef(0);
    const prevScrollTopRef = useRef(0);
    const { input, setInput, pendingImages, setPendingImages, consumeSentDraft, flushDraft, clearDraftImages, getImageDraftContext, appendPendingImage } = useCharacterChatDraft(activeCharacterId);
    const { recoveringDraft, rememberSubmission, finishSubmission, reconcileSubmission } = useChatDraftSubmission(activeCharacterId, view.turns, flushDraft, consumeSentDraft);
    const isBusy = view.busy || recoveringDraft;
    const inputRef = useRef(input);
    inputRef.current = input;
    const sttBaseDraftRef = useRef<string | null>(null);
    const sttBaseCharacterIdRef = useRef<string | null>(null);
    const sttBaseConversationIdRef = useRef<string | null>(null);
    const sttBaseGenerationRef = useRef<number | null>(null);
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
    const [error, setError] = useState<string | { message: string; requestId: string } | null>(null);
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

    const reportError = useCallback((error: unknown, requestId?: string) => {
        const message = getAsyncErrorMessage(error);
        setError(requestId ? { message, requestId } : message);
    }, []);
    useEffect(() => {
        let mounted = true;
        const unlisten = onChatWarning(warning => {
            if (mounted) reportError(warning);
        });
        // Registration can finish after the panel closes or is replaced.
        void unlisten.catch(error => console.warn("[Chat] Warning subscription failed", error));
        return () => {
            mounted = false;
            void unlisten.then(stop => stop(), () => {});
        };
    }, [reportError]);
    const handleStopGeneration = useCallback(() => {
        setIsStopping(true);
        void chatApplication.cancel().catch(reportError).finally(() => setIsStopping(false));
    }, [reportError]);
    const handleConversationSelection = useCallback(async (id: string | null) => {
        setIsSwitchingConversation(true);
        try { await chatApplication.selectConversation(id); } catch (error) { reportError(error); }
        finally { setIsSwitchingConversation(false); }
    }, [reportError]);
    const handleStartEmptyConversation = useCallback(async () => {
        await handleConversationSelection(null);
        return true;
    }, [handleConversationSelection]);
    const submittingRef = useRef(false);
    const sendDraft = useCallback(async (draftText: string, draftImages: string[] = [], camera: string | null = null) => {
        if (submittingRef.current) return;
        const clientRequestId = `req_${crypto.randomUUID()}`;
        const character = activeCharacterIdRef.current;
        if (!rememberSubmission(clientRequestId, draftText, draftImages)) return;
        submittingRef.current = true;
        try {
            const response = await chatApplication.send({
                message: draftText.trim() || "(image attached)",
                images: draftImages.length ? draftImages : undefined, camera_frame: camera ?? undefined,
                client_request_id: clientRequestId, character_id: character,
                conversation_id: activeConversationIdRef.current ?? undefined,
                conversation_generation: conversationGenerationRef.current,
                allow_image_gen: readJsonSetting<{ mode?: string }>(APP_SETTING_KEYS.bgConfig, {}).mode === "generated",
            });
            if (response.user_message_id != null) finishSubmission(character, clientRequestId, true);
        } catch (error) {
            reportError(error, clientRequestId);
        } finally {
            reconcileSubmission(character, clientRequestId);
            submittingRef.current = false;
        }
    }, [rememberSubmission, finishSubmission, reconcileSubmission, reportError]);
    const handleTranscription = useCallback((text: string) => {
        const character = sttBaseCharacterIdRef.current ?? activeCharacterIdRef.current;
        const conversation = sttBaseConversationIdRef.current ?? activeConversationIdRef.current;
        const generation = sttBaseGenerationRef.current ?? conversationGenerationRef.current;
        const base = sttBaseDraftRef.current ?? "";
        sttBaseDraftRef.current = null;
        sttBaseCharacterIdRef.current = null;
        sttBaseConversationIdRef.current = null;
        sttBaseGenerationRef.current = null;
        const fullMessage = combineDraftWithTranscription(base, text.trim());
        const current = character === activeCharacterIdRef.current && conversation === activeConversationIdRef.current && generation === conversationGenerationRef.current;
        if (!current || !sttAutoSend || interactionDisabled || recoveringDraft || submittingRef.current || chatApplication.getSnapshot().busy || !text.trim()) {
            if (character !== activeCharacterIdRef.current) saveCharacterDraft(character, fullMessage);
            else {
                setInput(previous => {
                    const preserved = previous && previous !== base && !previous.includes(text.trim()) ? combineDraftWithTranscription(previous, text.trim()) : fullMessage;
                    saveCharacterDraft(character, preserved);
                    return preserved;
                });
            }
            return;
        }
        setInput(fullMessage);
        void sendDraft(fullMessage).catch(reportError);
    }, [sttAutoSend, interactionDisabled, recoveringDraft, setInput, sendDraft, reportError]);
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

    useEffect(() => {
        const unlisten = listen("voice-interrupt-stt", () => {
            if (!sttEnabledRef.current) return;
            sttBaseDraftRef.current = inputRef.current;
            sttBaseCharacterIdRef.current = activeCharacterIdRef.current;
            sttBaseConversationIdRef.current = activeConversationIdRef.current;
            sttBaseGenerationRef.current = conversationGenerationRef.current;
            startVoiceRef.current({ autoStopOnSilence: true });
        });
        return () => { void unlisten.then(stop => stop()); };
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
            sttBaseCharacterIdRef.current = activeCharacterIdRef.current;
            sttBaseConversationIdRef.current = activeConversationIdRef.current;
            sttBaseGenerationRef.current = conversationGenerationRef.current;
            startVoice({ autoStopOnSilence: true });
        }, [continuousListening, handleTranscription, startVoice]),
    });

    // Effect: Sync partial STT text to input box for real-time feedback
    useEffect(() => {
        if (voiceState === VoiceState.Listening && sttPartialText) {
            const targetCharId = sttBaseCharacterIdRef.current ?? activeCharacterIdRef.current;
            const targetGen = sttBaseGenerationRef.current ?? conversationGenerationRef.current;
            const targetConvId = sttBaseConversationIdRef.current ?? activeConversationIdRef.current;
            if (
                activeCharacterIdRef.current === targetCharId &&
                conversationGenerationRef.current === targetGen &&
                activeConversationIdRef.current === targetConvId
            ) {
                const base = sttBaseDraftRef.current ?? "";
                const combined = combineDraftWithTranscription(base, sttPartialText);
                setInput(combined);
            }
        }
    }, [sttPartialText, voiceState, setInput]);

    // 听音生命周期退出兜底：若未完成识别退出且存在基准草稿，自动无损回滚
    useEffect(() => {
        if (prevVoiceStateRef.current === VoiceState.Listening && voiceState === VoiceState.Idle) {
            if (sttBaseDraftRef.current !== null) {
                const targetCharId = sttBaseCharacterIdRef.current ?? activeCharacterIdRef.current;
                if (activeCharacterIdRef.current === targetCharId) {
                    setInput(sttBaseDraftRef.current);
                } else {
                    saveCharacterDraft(targetCharId, sttBaseDraftRef.current);
                }
            }
            sttBaseDraftRef.current = null;
            sttBaseCharacterIdRef.current = null;
            sttBaseConversationIdRef.current = null;
            sttBaseGenerationRef.current = null;
        }
        prevVoiceStateRef.current = voiceState;
    }, [voiceState, setInput]);

    // Sync vision state when localStorage changes (from Settings panel)
    useEffect(() => {
        const checkVision = () => {
            const nextVisionEnabled = readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false);
            setVisionEnabled(nextVisionEnabled);
            if (!nextVisionEnabled) clearDraftImages();
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

    const handleSend = async (event?: React.FormEvent) => {
        event?.preventDefault();
        if (interactionDisabled || recoveringDraft || submittingRef.current) return;
        const sendWithVision = readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false);
        const images = sendWithVision ? [...pendingImages] : [];
        if (!input.trim() && !images.length) return;
        const camera = sendWithVision ? getLatestCameraFrame() : null;
        userScrolledRef.current = false;
        savedScrollSnapshotRef.current = null;
        try { await sendDraft(input, images, camera); }
        catch (error) { reportError(error); }
    };
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

        // 在首个 await 之前捕获草稿上下文:上传期间切换角色时,图片仍归发起角色
        const draftContext = getImageDraftContext();
        setIsUploading(true);
        try {
            const buffer = await file.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));
            const url = await uploadVisionImage(bytes, file.name);
            if (!appendPendingImage(draftContext, url)) {
                setError(t("chat.errors.image_upload_cancelled", "图片上传已取消"));
            }
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

        // 在首个 await 之前捕获草稿上下文:上传期间切换角色时,图片仍归发起角色
        const draftContext = getImageDraftContext();
        setIsUploading(true);
        try {
            const buffer = await file.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));
            const filename = `paste_${Date.now()}.png`;
            const url = await uploadVisionImage(bytes, filename);
            if (!appendPendingImage(draftContext, url)) {
                setError(t("chat.errors.image_upload_cancelled", "图片上传已取消"));
            }
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

        // 一次拖放 = 一个手势 = 一个发起角色:循环前捕获一次,中途切换后剩余文件仍归发起角色
        const draftContext = getImageDraftContext();
        setIsUploading(true);
        try {
            for (const file of files) {
                if (file.size > 5 * 1024 * 1024) {
                    setError(t("chat.errors.image_too_large"));
                    continue;
                }

                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));
                    const url = await uploadVisionImage(bytes, file.name);
                    if (!appendPendingImage(draftContext, url)) {
                        setError(t("chat.errors.image_upload_cancelled", "图片上传已取消"));
                    }
                } catch (err) {
                    setError(err instanceof Error ? err.message : t("chat.errors.upload_failed"));
                }
            }
        } finally {
            setIsUploading(false);
        }
    }, [visionEnabled, t, getImageDraftContext, appendPendingImage]);

    // ── STT: Advanced VAD Microphone toggle ─────────────────
    const handleMicToggle = useCallback(() => {
        if (voiceState === VoiceState.Idle) {
            sttBaseDraftRef.current = inputRef.current;
            sttBaseCharacterIdRef.current = activeCharacterIdRef.current;
            sttBaseConversationIdRef.current = activeConversationIdRef.current;
            sttBaseGenerationRef.current = conversationGenerationRef.current;
            startVoice({ autoStopOnSilence: true });
        } else {
            stopVoice();
        }
    }, [voiceState, startVoice, stopVoice]);

    // ── Clear history ──────────────────────────────────────
    const handleClearClick = () => setShowClearConfirm(true);
    const executeClear = async () => {
        setShowClearConfirm(false);
        try { await chatApplication.clear(); } catch (error) { reportError(error); }
    };
    const onToggleTranslation = useCallback((index: number) => {
        setExpandedTranslations(previous => { const next = new Set(previous); if (next.has(index)) next.delete(index); else next.add(index); return next; });
    }, []);
    const onEdit = useCallback(async (target: ChatMessageTarget, text: string) => {
        try { await chatApplication.edit(target, text); } catch (error) { reportError(error); }
    }, [reportError]);
    const onRegenerate = useCallback(async (target: ChatMessageTarget) => {
        const requestId = `req_${crypto.randomUUID()}`;
        try {
            await chatApplication.continueFrom(target, true, {
                client_request_id: requestId,
                allow_image_gen: readJsonSetting<{ mode?: string }>(APP_SETTING_KEYS.bgConfig, {}).mode === "generated",
            });
        } catch (error) { reportError(error, requestId); }
    }, [reportError]);
    const onContinueFrom = useCallback(async (target: ChatMessageTarget) => {
        try { await chatApplication.continueFrom(target); } catch (error) { reportError(error); }
    }, [reportError]);
    const onApproveTool = useCallback(async (_index: number, tool: ToolTraceItem) => {
        const id = getApprovalRequestId(tool);
        if (canSubmitApproval(tool) && id) try { await chatApplication.approveTool(id); } catch (error) { reportError(error); }
    }, [reportError]);
    const onRejectTool = useCallback(async (_index: number, tool: ToolTraceItem) => {
        const id = getApprovalRequestId(tool);
        if (canSubmitApproval(tool) && id) try { await chatApplication.rejectTool(id); } catch (error) { reportError(error); }
    }, [reportError]);
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
    const hasSendableImages = !recoveringDraft && visionEnabled && pendingImages.length > 0;
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
                {error && <ErrorToast
                    key={typeof error === "string" ? undefined : error.requestId}
                    message={typeof error === "string" ? error : error.message}
                    requestId={typeof error === "string" ? undefined : error.requestId}
                    onDismiss={() => setError(null)}
                />}
            </AnimatePresence>

            {/* 对话历史侧边栏 */}
            <ConversationSidebar
                open={sidebarOpen}
                onClose={() => setSidebarOpen(false)}
                characterId={activeCharacterId}
                activeConversationId={activeConversationId}
                isSwitchingConversation={isSwitchingConversation}
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
                                key={msg.failureScope
                                    ? JSON.stringify([activeCharacterId, activeConversationId, msg.failureScope.turn_id, "failure"])
                                    : msg.role === "kokoro" && msg.turnId
                                    ? JSON.stringify([activeCharacterId, msg.turnId])
                                    : JSON.stringify([activeConversationId, view.generation, msg.id ?? `${globalIndex}-${msg.role}`])}
                                message={msg}
                                conversationId={activeConversationId}
                                generation={view.generation}
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

                    {shouldRenderTypingIndicator({ isThinking, messages: deferredMessages, activeMessageIndex: activeTurn ? deferredMessages.findIndex(message => message.turnId === activeTurn.turn_id) : null }) && <TypingIndicator />}
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
                                        onError={() => {
                                            console.warn("[ChatPanel] Draft image failed to render, removing:", url);
                                            removePendingImage(idx);
                                        }}
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
                        value={recoveringDraft ? "" : input}
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
