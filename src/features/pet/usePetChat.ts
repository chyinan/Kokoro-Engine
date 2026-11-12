import { useEffect, useRef, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { chatApplication } from "../../lib/chat/application";
import { isActiveTurn } from "../../lib/chat/projection";
import { getKokoroErrorMessage } from "../../lib/kokoro-bridge";

export interface PetChatState {
    isStreaming: boolean;
    sendMessage: (text: string) => Promise<void>;
}

/** The hook owns bubble presentation only. Admission/execution live in Rust. */
export function usePetChat(): PetChatState {
    const view = useSyncExternalStore(chatApplication.subscribe, chatApplication.getSnapshot);
    const sending = useRef(false);
    const lastPresentedRevision = useRef(0);
    const turn = [...view.turns].reverse().find(item => item.client_request_id.startsWith("pet_"));
    const revision = turn?.completion_revision ?? turn?.revision ?? 0;
    const text = turn?.text;
    const error = turn?.error;
    const active = turn ? isActiveTurn(turn) : false;
    useEffect(() => {
        if (revision <= lastPresentedRevision.current) return;
        lastPresentedRevision.current = revision;
        if (text) {
            void invoke("update_bubble_text", { text }).catch(() => invoke("show_bubble_window", { text })).catch(() => {});
        } else if (error) {
            void invoke("show_bubble_window", { text: error }).catch(() => {});
        }
        if (!active) {
            const timer = setTimeout(() => { void invoke("hide_bubble_window").catch(() => {}); }, 5000);
            return () => clearTimeout(timer);
        }
    }, [revision, text, error, active]);
    const sendMessage = async (text: string) => {
        const message = text.trim();
        if (!message || sending.current) return;
        sending.current = true;
        const client_request_id = `pet_${crypto.randomUUID()}`;
        void invoke("show_bubble_window", { text: "..." }).catch(() => {});
        try { await chatApplication.send({ message, client_request_id }); }
        catch (error) {
            void invoke("show_bubble_window", { text: getKokoroErrorMessage(error) }).catch(() => {});
        } finally { sending.current = false; }
    };
    return { isStreaming: view.busy, sendMessage };
}
