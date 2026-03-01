import { useState, useRef, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Send, Trash2, AlertCircle, MessageCircle, ChevronLeft, ImagePlus, X, Mic, MicOff, Languages, History, RefreshCw } from "lucide-react";
import { streamChat, onChatDelta, onChatDone, onChatError, onChatTranslation, clearHistory, uploadVisionImage, synthesize, onToolCallResult, listConversations, loadConversation, onTelegramChatSync } from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";
import { useVoiceInput, VoiceState, useTypingReveal } from "../hooks";
import { useTranslation } from "react-i18next";
import ConversationSidebar from "./ConversationSidebar";

// ── Types ──────────────────────────────────────────────────
interface ChatMessage {
    role: "user" | "kokoro";
    text: string;
    images?: string[];
    translation?: string;
    isError?: boolean;
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
export default function ChatPanel() {
    const { t } = useTranslation();
    const [collapsed, setCollapsed] = useState(false);
    const [messages, setMessages] = useState<ChatMessage[]>([]);
    const [input, setInput] = useState("");
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
    // Translation received from backend chat-translation event
    const translationRef = useRef<string | undefined>(undefined);
    // When true, the next delta must create a new kokoro bubble (even if the last message is already kokoro)
    const forceNewBubbleRef = useRef(false);

    // Typing reveal: per-character animation
    const { pushDelta, flush: flushReveal, reset: resetReveal } = useTypingReveal({
        active: isStreaming,
        onReveal: (visibleText: string) => {
            setMessages(prev => {
                const last = prev[prev.length - 1];
                if (last && last.role === "kokoro" && isStreamingRef.current) {
                    return [...prev.slice(0, -1), { ...last, text: visibleText }];
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
    const fileInputRef = useRef<HTMLInputElement>(null);
    // Store last failed request for retry
    const lastFailedRequestRef = useRef<{ message: string; images?: string[]; allowImageGen?: boolean } | null>(null);

    // Vision Mode
    const [visionEnabled, setVisionEnabled] = useState(() => localStorage.getItem("kokoro_vision_enabled") === "true");
    const [pendingImages, setPendingImages] = useState<string[]>([]);
    const [isUploading, setIsUploading] = useState(false);

    // 对话历史侧边栏
    const [sidebarOpen, setSidebarOpen] = useState(false);

    // 自动恢复最近对话
    useEffect(() => {
        const characterId = localStorage.getItem("kokoro_active_character_id") || "default";
        listConversations(characterId).then(convs => {
            if (convs.length > 0) {
                loadConversation(convs[0].id).then(msgs => {
                    const chatMsgs: ChatMessage[] = msgs.map(m => {
                        if (m.role !== "user") {
                            // Read translation from metadata (persisted as JSON)
                            let translation: string | undefined;
                            if (m.metadata) {
                                try {
                                    const meta = JSON.parse(m.metadata);
                                    if (meta.translation) translation = meta.translation;
                                } catch { /* ignore malformed metadata */ }
                            }
                            const text = m.content
                                .replace(/\[ACTION:\w+\]\s*/g, "")
                                .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
                                .replace(/\[EMOTION:[^\]]*\]/g, "")
                                .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
                                .trim();
                            return { role: "kokoro" as const, text, translation };
                        }
                        return { role: "user" as const, text: m.content };
                    });
                    setMessages(chatMsgs);
                }).catch(err => console.error("[ChatPanel] Failed to restore conversation:", err));
            }
        }).catch(() => { /* backend not ready */ });
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // STT (Speech-to-Text) �?Advanced VAD Mode
    const sttEnabled = localStorage.getItem("kokoro_stt_enabled") === "true";
    const sttAutoSend = localStorage.getItem("kokoro_stt_auto_send") === "true";

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
            setVisionEnabled(localStorage.getItem("kokoro_vision_enabled") === "true");
        };
        window.addEventListener("storage", checkVision);
        // Also poll on focus since Tauri doesn't fire storage events within same webview
        window.addEventListener("focus", checkVision);
        return () => {
            window.removeEventListener("storage", checkVision);
            window.removeEventListener("focus", checkVision);
        };
    }, []);

    // ── Auto-scroll ────────────────────────────────────────
    const scrollToBottom = useCallback(() => {
        if (!userScrolledRef.current) {
            messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
        }
    }, []);

    useEffect(scrollToBottom, [messages, scrollToBottom]);

    const handleScroll = useCallback(() => {
        const container = messagesContainerRef.current;
        if (!container) return;
        const atBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 40;
        userScrolledRef.current = !atBottom;
    }, []);

    // Track unread messages while collapsed
    useEffect(() => {
        if (collapsed && messages.length > 0) {
            const last = messages[messages.length - 1];
            if (last.role === "kokoro") {
                setUnreadCount(prev => prev + 1);
            }
        }
    }, [messages.length, collapsed]);

    // Sync messages ref for use in event callbacks (avoids stale closure)
    useEffect(() => {
        messagesRef.current = messages;
    }, [messages]);

    // ── Chat event listeners ───────────────────────────────
    useEffect(() => {
        let aborted = false;
        const cleanups: (() => void)[] = [];

        const setup = async () => {
            const unDelta = await onChatDelta((rawDelta: string) => {
                if (aborted || !isStreamingRef.current) return;
                // Strip [ACTION:xxx], [TOOL_CALL:...], and [TRANSLATE:...] tags — these are control signals, not dialogue
                const delta = rawDelta
                    .replace(/\[ACTION:\w+\]\s*/g, "")
                    .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
                    .replace(/\[TRANSLATE:[^\]]*\]\s*/g, "");
                if (!delta) return;
                setIsThinking(false);

                // Accumulate raw text for TTS
                rawResponseRef.current += delta;

                // Ensure a kokoro message exists for the reveal to update into
                setMessages(prev => {
                    const last = prev[prev.length - 1];
                    if (last && last.role === "kokoro" && !forceNewBubbleRef.current) {
                        return prev; // message already exists — don't create a duplicate
                    }
                    forceNewBubbleRef.current = false;
                    return [...prev, { role: "kokoro", text: "" }];
                });

                // Push to typing reveal buffer
                pushDelta(delta);
            });
            if (aborted) { unDelta(); return; }
            cleanups.push(unDelta);

            const unTranslation = await onChatTranslation((translation: string) => {
                if (aborted) return;
                translationRef.current = translation;
            });
            if (aborted) { unTranslation(); return; }
            cleanups.push(unTranslation);

            const unDone = await onChatDone(() => {
                if (aborted) return;
                // Flush any remaining buffered text immediately
                flushReveal();
                stopStreaming();
                setIsThinking(false);
                userScrolledRef.current = false;

                // Ensure the final message text is reliably set.
                // flushReveal's setMessages may be discarded due to React batching
                // (isStreamingRef becomes false before React processes the update).
                // This explicit update bypasses that check.
                const fullText = rawResponseRef.current;
                if (fullText) {
                    // Use translation from backend event (handles multi-round tool calls correctly)
                    const translationText = translationRef.current;

                    setMessages(prev => {
                        const last = prev[prev.length - 1];
                        if (last && last.role === "kokoro") {
                            const cleanText = fullText
                                .replace(/\[ACTION:\w+\]\s*/g, "")
                                .replace(/\[TOOL_CALL:[^\]]*\]\s*/g, "")
                                .replace(/\[EMOTION:[^\]]*\]/g, "")
                                .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
                                .replace(/\[TRANSLATE:[\s\S]*?\]/gi, "");
                            return [...prev.slice(0, -1), { ...last, text: cleanText, translation: translationText }];
                        }
                        return prev;
                    });
                } else {
                    // No content received (e.g. network error) — remove empty placeholder
                    setMessages(prev => {
                        const last = prev[prev.length - 1];
                        if (last && last.role === "kokoro" && !last.text) {
                            return prev.slice(0, -1);
                        }
                        return prev;
                    });
                }

                // ── Auto-TTS: speak the completed response ──
                if (localStorage.getItem("kokoro_tts_enabled") === "true") {
                    // Read last message directly from state ref (avoids StrictMode accumulation bugs)
                    const currentMessages = messagesRef.current;
                    const lastMsg = currentMessages[currentMessages.length - 1];
                    if (lastMsg && lastMsg.role === "kokoro") {
                        // Use raw accumulated text for TTS (not the partially-revealed text)
                        const fullText = rawResponseRef.current || lastMsg.text;
                        const cleanText = fullText
                            .replace(/\[EMOTION:[^\]]*\]/g, "")
                            .replace(/\[IMAGE_PROMPT:[^\]]*\]/g, "")
                            .replace(/\[ACTION:\w+\]/g, "")
                            .replace(/\[TOOL_CALL:[^\]]*\]/g, "")
                            .replace(/\[TRANSLATE:[\s\S]*?\]/gi, "")
                            .replace(/[\u{1F600}-\u{1F64F}\u{1F300}-\u{1F5FF}\u{1F680}-\u{1F6FF}\u{1F1E0}-\u{1F1FF}\u{2600}-\u{27BF}\u{2700}-\u{27BF}\u{FE00}-\u{FE0F}\u{1F900}-\u{1F9FF}\u{1FA00}-\u{1FA6F}\u{1FA70}-\u{1FAFF}\u{200D}\u{20E3}\u{E0020}-\u{E007F}]/gu, "")
                            .trim();

                        if (cleanText) {
                            console.log("[TTS] Auto-speak triggered, text length:", cleanText.length);
                            synthesize(cleanText, {
                                provider_id: localStorage.getItem("kokoro_tts_provider") || undefined,
                                voice: localStorage.getItem("kokoro_tts_voice") || undefined,
                                speed: parseFloat(localStorage.getItem("kokoro_tts_speed") || "1.0"),
                                pitch: parseFloat(localStorage.getItem("kokoro_tts_pitch") || "1.0"),
                            }).catch(err => console.error("[TTS] Auto-speak failed:", err));
                        }
                    }
                }
            });
            if (aborted) { unDone(); return; }
            cleanups.push(unDone);

            const unError = await onChatError((err: string) => {
                if (aborted) return;
                stopStreaming();
                setIsThinking(false);
                setError(err);
                // Remove empty kokoro placeholder left by proactive or normal streaming
                setMessages(prev => {
                    const last = prev[prev.length - 1];
                    if (last && last.role === "kokoro" && !last.text) {
                        return prev.slice(0, -1);
                    }
                    return prev;
                });
            });
            if (aborted) { unError(); return; }
            cleanups.push(unError);

            const unToolResult = await onToolCallResult((event) => {
                if (aborted) return;
                if (event.result) {
                    console.log(`[ToolCall] ${event.tool}: ${event.result.message}`);
                } else if (event.error) {
                    console.error(`[ToolCall] ${event.tool} failed: ${event.error}`);
                }
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

                // Mark that the next delta should create a new bubble
                // (don't push an empty message now — avoids blank bubble + double bubble)
                forceNewBubbleRef.current = true;

                // Start streaming — compose_prompt() handles full context (system prompt, memory, emotion, history, language)
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;
                resetReveal();
                rawResponseRef.current = "";
                translationRef.current = undefined;

                streamChat({
                    message: instruction,
                    hidden: true,
                    character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
                }).catch(err => {
                    stopStreaming();
                    setIsThinking(false);
                    setError(err instanceof Error ? err.message : String(err));
                    forceNewBubbleRef.current = false;
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

                forceNewBubbleRef.current = true;
                startStreaming();
                setIsThinking(true);
                userScrolledRef.current = false;
                resetReveal();
                rawResponseRef.current = "";
                translationRef.current = undefined;
            });
            cleanups.push(() => unInteraction());
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
        if ((!trimmed && pendingImages.length === 0) || isStreaming) return;

        setMessages(prev => [...prev, { role: "user", text: trimmed, images: pendingImages.length > 0 ? [...pendingImages] : undefined }]);
        const imagesToSend = [...pendingImages];
        setInput("");
        setPendingImages([]);
        startStreaming();
        setIsThinking(true);
        userScrolledRef.current = false;
        resetReveal();
        rawResponseRef.current = "";
        translationRef.current = undefined;

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

    // ── Retry last failed message ─────────────────────────────
    const handleRetry = async () => {
        const req = lastFailedRequestRef.current;
        if (!req || isStreaming) return;

        // Remove the error message
        setMessages(prev => {
            const last = prev[prev.length - 1];
            if (last && last.isError) return prev.slice(0, -1);
            return prev;
        });

        lastFailedRequestRef.current = null;
        startStreaming();
        setIsThinking(true);
        userScrolledRef.current = false;
        resetReveal();
        rawResponseRef.current = "";
        translationRef.current = undefined;

        try {
            await streamChat({
                message: req.message,
                allow_image_gen: req.allowImageGen,
                images: req.images,
                character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
            });
        } catch (err) {
            stopStreaming();
            setIsThinking(false);
            setError(err instanceof Error ? err.message : String(err));
            lastFailedRequestRef.current = req;

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

    // ── STT: Advanced VAD Microphone toggle ─────────────────
    const handleMicToggle = useCallback(() => {
        if (voiceState === VoiceState.Idle) {
            startVoice();
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
                            : "bg-emerald-500 shadow-[var(--glow-success)]"
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
                    {messages.map((msg, i) => (
                        <motion.div
                            key={`${i}-${msg.role}`}
                            initial={{ opacity: 0, y: 10, scale: 0.95 }}
                            animate={{ opacity: 1, y: 0, scale: 1 }}
                            transition={{ duration: 0.3 }}
                            className={clsx(
                                "max-w-[85%] p-3 rounded-lg text-sm leading-relaxed font-body",
                                msg.role === "user"
                                    ? "ml-auto bg-[var(--color-accent)]/10 border border-[var(--color-accent)]/30 text-[var(--color-accent)] rounded-tr-none"
                                    : "mr-auto bg-slate-900/50 border border-slate-700/50 text-slate-300 rounded-tl-none"
                            )}
                        >
                            {msg.images && msg.images.length > 0 && (
                                <div className="flex flex-wrap gap-1.5 mb-2">
                                    {msg.images.map((url, imgIdx) => (
                                        <img
                                            key={imgIdx}
                                            src={url}
                                            alt="attached"
                                            className="max-w-[180px] max-h-[120px] rounded-md object-cover border border-white/10"
                                        />
                                    ))}
                                </div>
                            )}
                            {msg.text}
                            {/* Retry button for error messages */}
                            {msg.isError && (
                                <button
                                    onClick={handleRetry}
                                    className="flex items-center gap-1.5 mt-2 px-3 py-1.5 rounded-md text-[11px] font-medium bg-[var(--color-accent)]/15 border border-[var(--color-accent)]/30 text-[var(--color-accent)] hover:bg-[var(--color-accent)]/25 transition-colors"
                                >
                                    <RefreshCw size={12} strokeWidth={2} />
                                    {t("chat.actions.retry")}
                                </button>
                            )}
                            {/* Translation toggle �?WeChat style */}
                            {msg.role === "kokoro" && msg.translation && (
                                <div className="mt-2 -mb-1">
                                    <button
                                        onClick={() => setExpandedTranslations(prev => {
                                            const next = new Set(prev);
                                            if (next.has(i)) next.delete(i);
                                            else next.add(i);
                                            return next;
                                        })}
                                        className="flex items-center gap-1 text-[10px] text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                                    >
                                        <Languages size={11} strokeWidth={1.5} />
                                        {expandedTranslations.has(i) ? t("chat.translation.hide") : t("chat.translation.show")}
                                    </button>
                                    <AnimatePresence>
                                        {expandedTranslations.has(i) && (
                                            <motion.div
                                                initial={{ height: 0, opacity: 0 }}
                                                animate={{ height: "auto", opacity: 1 }}
                                                exit={{ height: 0, opacity: 0 }}
                                                transition={{ duration: 0.2 }}
                                                className="overflow-hidden"
                                            >
                                                <div className="mt-1.5 pt-1.5 border-t border-slate-700/40 text-xs text-[var(--color-text-muted)] leading-relaxed">
                                                    {msg.translation}
                                                </div>
                                            </motion.div>
                                        )}
                                    </AnimatePresence>
                                </div>
                            )}
                        </motion.div>
                    ))}

                    {isThinking && <TypingIndicator />}
                </AnimatePresence>
                <div ref={messagesEndRef} />
            </div>

            {/* Input */}
            <form onSubmit={handleSend} className="border-t border-[var(--color-border)] bg-black/20">
                {/* Pending images preview */}
                <AnimatePresence>
                    {pendingImages.length > 0 && (
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

                    {/* Microphone button �?Advanced VAD Mode */}
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
                                            ? "text-emerald-400 bg-emerald-500/15 border border-emerald-500/30"
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

                    <input
                        type="text"
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        placeholder={t("chat.input.placeholder")}
                        disabled={isStreaming}
                        className={clsx(
                            "flex-1 bg-black/40 border border-[var(--color-border)]",
                            "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
                            "text-sm rounded-lg px-4 py-2.5 font-body",
                            "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                            "transition-all",
                            isStreaming && "opacity-50 cursor-not-allowed"
                        )}
                    />
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.9 }}
                        type="submit"
                        disabled={isStreaming || (!input.trim() && pendingImages.length === 0)}
                        className={clsx(
                            "p-2.5 rounded-lg transition-colors",
                            "bg-[var(--color-accent)] text-black hover:bg-white",
                            (isStreaming || (!input.trim() && pendingImages.length === 0)) && "opacity-50 cursor-not-allowed"
                        )}
                        aria-label="Send message"
                    >
                        <Send size={16} strokeWidth={1.5} />
                    </motion.button>
                </div>
            </form>
        </motion.div >
    );
}
