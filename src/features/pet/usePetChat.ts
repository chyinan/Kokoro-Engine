import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { chatApplication } from "../../lib/chat/application";
import { isActiveTurn } from "../../lib/chat/projection";
import { getKokoroErrorMessage } from "../../lib/kokoro-bridge";
import type { ChatTurnSnapshot } from "../../lib/kokoro-bridge";

export interface PetChatState {
    isStreaming: boolean;
    sendMessage: (text: string) => Promise<void>;
}

const replyRevision = (turn: ChatTurnSnapshot) => turn.completion_revision ?? turn.revision;

/** The hook owns bubble presentation only. Admission/execution live in Rust. */
export function usePetChat(): PetChatState {
    const view = useSyncExternalStore(chatApplication.subscribe, chatApplication.getSnapshot);
    const sending = useRef(false);
    const [request, setRequest] = useState<{ id: string; characterId: string }>();
    const requestId = request?.characterId === view.character_id ? request.id : undefined;
    const presentation = useRef<{ id: string; revision: number; shown: boolean }>(undefined);
    const hideTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
    const bubbleVersion = useRef(0);
    const clearHideTimer = () => {
        ++bubbleVersion.current;
        clearTimeout(hideTimer.current);
        hideTimer.current = undefined;
    };
    const scheduleHide = () => {
        clearHideTimer();
        const version = bubbleVersion.current;
        hideTimer.current = setTimeout(() => {
            if (bubbleVersion.current === version) void invoke("hide_bubble_window").catch(() => {});
        }, 5000);
    };
    const scope = useRef(0);
    useEffect(() => {
        ++scope.current;
        presentation.current = undefined;
        setRequest(undefined);
        return () => {
            ++scope.current;
            clearHideTimer();
            void invoke("hide_bubble_window").catch(() => {});
        };
    }, [view.character_id]);
    const turns = view.turns.filter(item => item.character_id === view.character_id);
    const turn = requestId
        ? turns.find(item => item.client_request_id === requestId)
        : turns.reduce<ChatTurnSnapshot | undefined>((latest, item) =>
            item.client_request_id.startsWith("pet_") && (!latest || replyRevision(item) > replyRevision(latest)) ? item : latest,
        undefined);
    const revision = turn ? replyRevision(turn) : 0;
    const replyRequestId = turn?.client_request_id;
    const text = turn?.text;
    const error = turn?.error;
    const active = turn ? isActiveTurn(turn) : false;
    useEffect(() => {
        if (!replyRequestId) return;
        const previous = presentation.current;
        // Restore active work only. A terminal snapshot may update a reply this
        // view has followed, but must never reopen an old journal entry.
        if (!active && replyRequestId !== requestId && previous?.id !== replyRequestId) return;
        if (previous?.id === replyRequestId && revision <= previous.revision) return;
        clearHideTimer();
        const shown = previous?.id === replyRequestId && previous.shown;
        presentation.current = { id: replyRequestId, revision, shown };
        if (text) {
            // A recreated pet may find its old bubble window hidden.
            if (!requestId && !shown) {
                void invoke("show_bubble_window", { text }).catch(() => {});
            } else {
                const presentingScope = scope.current;
                void invoke("update_bubble_text", { text }).catch(() => {
                    if (scope.current === presentingScope && chatApplication.getSnapshot().character_id === view.character_id) {
                        return invoke("show_bubble_window", { text });
                    }
                }).catch(() => {});
            }
            presentation.current.shown = true;
        } else if (error) {
            void invoke("show_bubble_window", { text: error }).catch(() => {});
        }
        if (!active) {
            scheduleHide();
        }
    }, [revision, replyRequestId, requestId, text, error, active, view.character_id]);
    const sendMessage = async (text: string) => {
        const message = text.trim();
        if (!message || sending.current) return;
        sending.current = true;
        const sendingScope = scope.current;
        clearHideTimer();
        const client_request_id = `pet_${crypto.randomUUID()}`;
        setRequest({ id: client_request_id, characterId: view.character_id });
        void invoke("show_bubble_window", { text: "..." }).catch(() => {});
        try {
            await chatApplication.send({ message, client_request_id,
                character_id: view.character_id, conversation_id: view.conversation_id ?? undefined,
                conversation_generation: view.generation });
        }
        catch (error) {
            if (scope.current === sendingScope && chatApplication.getSnapshot().character_id === view.character_id) {
                void invoke("show_bubble_window", { text: getKokoroErrorMessage(error) }).catch(() => {});
                scheduleHide();
            }
        } finally { sending.current = false; }
    };
    return { isStreaming: view.busy, sendMessage };
}
