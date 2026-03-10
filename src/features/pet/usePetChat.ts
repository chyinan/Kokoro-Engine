import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

export interface PetChatState {
    isStreaming: boolean;
    sendMessage: (text: string) => Promise<void>;
}

export function usePetChat(): PetChatState {
    const [isStreaming, setIsStreaming] = useState(false);
    const accumulatedRef = useRef("");
    const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    useEffect(() => {
        const unlistenDelta = listen<string>("chat-delta", async (event) => {
            accumulatedRef.current += event.payload;
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

        const unlistenDone = listen("chat-done", () => {
            setIsStreaming(false);
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
            hideTimerRef.current = setTimeout(async () => {
                accumulatedRef.current = "";
                await invoke("hide_bubble_window").catch(() => {});
            }, 5000);
        });

        const unlistenError = listen("chat-error", () => {
            setIsStreaming(false);
            accumulatedRef.current = "";
            invoke("hide_bubble_window").catch(() => {});
        });

        return () => {
            unlistenDelta.then(fn => fn());
            unlistenDone.then(fn => fn());
            unlistenError.then(fn => fn());
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        };
    }, []);

    const sendMessage = async (text: string) => {
        if (!text.trim()) return;
        accumulatedRef.current = "";
        setIsStreaming(true);

        // Show bubble window immediately with empty state
        await invoke("show_bubble_window", { text: "..." }).catch(() => {});

        try {
            await emit("pet-chat-start", { message: text });
            await invoke("stream_chat", { request: { message: text } });
        } catch (e) {
            console.error("[PetChat] stream_chat error:", e);
            setIsStreaming(false);
            invoke("hide_bubble_window").catch(() => {});
        }
    };

    return { isStreaming, sendMessage };
}

