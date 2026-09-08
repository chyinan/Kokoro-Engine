import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

// pattern: Imperative Shell

type PetChatTurnEvent = {
    turn_id: string;
    client_request_id?: string | null;
};

type PetChatErrorEvent = Omit<PetChatTurnEvent, "turn_id"> & {
    turn_id?: string;
    trace_id?: string;
};

function isOwnPetChatTurnEvent(
    activeClientRequestId: string | null,
    activeTurnId: string | null,
    event: PetChatTurnEvent,
): boolean {
    if (activeClientRequestId === null) return false;
    if (event.client_request_id !== null && event.client_request_id !== undefined) {
        return event.client_request_id === activeClientRequestId;
    }
    return event.turn_id === activeTurnId;
}

function isOwnPetChatErrorEvent(
    activeClientRequestId: string | null,
    activeTurnId: string | null,
    event: PetChatErrorEvent,
): boolean {
    if (activeClientRequestId === null) return false;
    if (event.client_request_id !== null && event.client_request_id !== undefined) {
        return event.client_request_id === activeClientRequestId;
    }
    const eventTurnId = event.turn_id ?? event.trace_id;
    return eventTurnId === activeTurnId;
}

export interface PetChatState {
    isStreaming: boolean;
    sendMessage: (text: string) => Promise<void>;
}

export function usePetChat(): PetChatState {
    const [isStreaming, setIsStreaming] = useState(false);
    const activeTurnIdRef = useRef<string | null>(null);
    const activeClientRequestIdRef = useRef<string | null>(null);
    const accumulatedRef = useRef("");
    const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const pendingHandshakesRef = useRef<Map<string, (val: { accepted: true; conversation_id?: string } | { accepted: false; reason?: string; timeout?: boolean }) => void>>(new Map());

    useEffect(() => {
        const unlistenStart = listen<PetChatTurnEvent>("chat-turn-start", async (event) => {
            if (!isOwnPetChatTurnEvent(activeClientRequestIdRef.current, activeTurnIdRef.current, event.payload)) return;
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
            activeTurnIdRef.current = event.payload.turn_id;
            accumulatedRef.current = "";
            setIsStreaming(true);
        });

        const unlistenDelta = listen<PetChatTurnEvent & { delta: string }>("chat-turn-delta", async (event) => {
            if (!isOwnPetChatTurnEvent(activeClientRequestIdRef.current, activeTurnIdRef.current, event.payload)) return;
            accumulatedRef.current += event.payload.delta;
            setIsStreaming(true);
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);

            // Update bubble window with accumulated text
            try {
                await invoke("update_bubble_text", { text: accumulatedRef.current });
            } catch {
                // Bubble window may not exist yet, show it
                await invoke("show_bubble_window", { text: accumulatedRef.current }).catch(() => {});
            }
        });

        const unlistenDone = listen<PetChatTurnEvent & { turn_id: string; status: "completed" | "error" | "cancelled" }>("chat-turn-finish", (event) => {
            if (!isOwnPetChatTurnEvent(activeClientRequestIdRef.current, activeTurnIdRef.current, event.payload)) return;
            setIsStreaming(false);
            activeClientRequestIdRef.current = null;
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
            hideTimerRef.current = setTimeout(async () => {
                activeTurnIdRef.current = null;
                accumulatedRef.current = "";
                await invoke("hide_bubble_window").catch(() => {});
            }, 5000);
        });

        const unlistenError = listen<PetChatErrorEvent>("chat-error", (event) => {
            if (!isOwnPetChatErrorEvent(activeClientRequestIdRef.current, activeTurnIdRef.current, event.payload)) return;
            setIsStreaming(false);
            activeTurnIdRef.current = null;
            activeClientRequestIdRef.current = null;
            accumulatedRef.current = "";
            invoke("hide_bubble_window").catch(() => {});
        });

        const unlistenAccepted = listen<{ client_request_id?: string; conversation_id?: string }>("pet-chat-accepted", (event) => {
            const reqId = event.payload?.client_request_id;
            if (reqId && pendingHandshakesRef.current.has(reqId)) {
                pendingHandshakesRef.current.get(reqId)!({
                    accepted: true,
                    conversation_id: event.payload?.conversation_id,
                });
                pendingHandshakesRef.current.delete(reqId);
            }
        });

        const unlistenRejected = listen<{ client_request_id?: string; reason?: string }>("pet-chat-rejected", (event) => {
            const reqId = event.payload?.client_request_id;
            if (reqId && pendingHandshakesRef.current.has(reqId)) {
                pendingHandshakesRef.current.get(reqId)!({
                    accepted: false,
                    reason: event.payload?.reason,
                });
                pendingHandshakesRef.current.delete(reqId);
            }
            if (!reqId || activeClientRequestIdRef.current !== reqId) {
                return;
            }
            invoke("cancel_chat_turn", { turnId: reqId, reason: "pet_chat_rejected" }).catch(() => {});
            setIsStreaming(false);
            activeTurnIdRef.current = null;
            activeClientRequestIdRef.current = null;
            accumulatedRef.current = "";
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
            invoke("hide_bubble_window").catch(() => {});
        });

        return () => {
            unlistenStart.then(fn => fn());
            unlistenDelta.then(fn => fn());
            unlistenDone.then(fn => fn());
            unlistenError.then(fn => fn());
            unlistenAccepted.then(fn => fn());
            unlistenRejected.then(fn => fn());
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        };
    }, []);

    const sendMessage = async (text: string) => {
        const trimmed = text.trim();
        if (!trimmed || isStreaming || activeTurnIdRef.current !== null || activeClientRequestIdRef.current !== null) return;

        try {
            const busy = await invoke<boolean>("is_chat_busy");
            if (busy) return;
        } catch {
            // If check fails (e.g. IPC mock), proceed
        }

        const clientRequestId = `pet_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
        activeClientRequestIdRef.current = clientRequestId;
        activeTurnIdRef.current = null;
        accumulatedRef.current = "";
        setIsStreaming(true);

        // Show bubble window immediately with empty state
        await invoke("show_bubble_window", { text: "..." }).catch(() => {});

        const handshakePromise = new Promise<{ accepted: true; conversation_id?: string } | { accepted: false; reason?: string; timeout?: boolean }>((resolve) => {
            pendingHandshakesRef.current.set(clientRequestId, resolve);
        });

        let timeoutId: ReturnType<typeof setTimeout> | null = null;
        const timeoutPromise = new Promise<{ accepted: false; timeout: true }>((resolve) => {
            timeoutId = setTimeout(() => resolve({ accepted: false, timeout: true }), 1000);
        });

        try {
            await emit("pet-chat-start", { message: trimmed, client_request_id: clientRequestId });
            const handshake = await Promise.race([handshakePromise, timeoutPromise]);
            if (timeoutId) clearTimeout(timeoutId);
            pendingHandshakesRef.current.delete(clientRequestId);

            if (!handshake.accepted) {
                console.warn("[PetChat] Chat rejected or timed out by UI:", handshake);
                setIsStreaming(false);
                activeTurnIdRef.current = null;
                activeClientRequestIdRef.current = null;
                accumulatedRef.current = "";
                if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
                invoke("hide_bubble_window").catch(() => {});
                if ("timeout" in handshake && handshake.timeout) {
                    emit("pet-chat-failed", {
                        client_request_id: clientRequestId,
                        error: "handshake_timeout",
                    }).catch(() => {});
                }
                return;
            }

            await invoke("stream_chat", {
                request: {
                    message: trimmed,
                    character_id: localStorage.getItem("kokoro_active_character_id") || undefined,
                    client_request_id: clientRequestId,
                    conversation_id: handshake.conversation_id,
                },
            });
        } catch (e) {
            console.error("[PetChat] stream_chat error:", e);
            pendingHandshakesRef.current.delete(clientRequestId);
            if (timeoutId) clearTimeout(timeoutId);
            setIsStreaming(false);
            activeTurnIdRef.current = null;
            activeClientRequestIdRef.current = null;
            accumulatedRef.current = "";
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
            invoke("hide_bubble_window").catch(() => {});
            emit("pet-chat-failed", {
                client_request_id: clientRequestId,
                error: e instanceof Error ? e.message : String(e),
            }).catch(() => {});
        }
    };

    return { isStreaming, sendMessage };
}
