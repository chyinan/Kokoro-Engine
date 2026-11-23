import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { recoverChatDraftSubmission, type ChatTurnSnapshot } from "../../../lib/kokoro-bridge";
import { isActiveTurn } from "../../../lib/chat/projection";

interface DraftSubmission {
    characterId: string;
    requestId: string;
    text: string;
    images: string[];
}

const submissionKey = (characterId: string) => `kokoro_chat_submission_${encodeURIComponent(characterId)}`;

function readSubmittedImages(key: string, requestId: string): string[] {
    try {
        const saved = JSON.parse(sessionStorage.getItem(key) ?? "null");
        if (saved?.requestId === requestId && Array.isArray(saved.images)) {
            return saved.images.filter((url: unknown): url is string => typeof url === "string");
        }
    } catch {
        // Session storage can be unavailable even when the text draft is saved.
    }
    return [];
}

function readStoredSubmission(characterId: string): DraftSubmission | null {
    const key = submissionKey(characterId);
    try {
        const saved = JSON.parse(localStorage.getItem(key) ?? "null");
        if (typeof saved?.requestId !== "string" || typeof saved.text !== "string") return null;
        return {
            characterId,
            requestId: saved.requestId,
            text: saved.text,
            images: readSubmittedImages(key, saved.requestId),
        };
    } catch {
        return null;
    }
}

export function useChatDraftSubmission(
    characterId: string,
    turns: readonly ChatTurnSnapshot[],
    flushDraft: () => void,
    consumeSentDraft: (characterId: string, text: string, images: string[]) => void,
) {
    const current = useRef<DraftSubmission | null>(null);
    const [recoveringDraft, setRecoveringDraft] = useState(() => readStoredSubmission(characterId) !== null);

    const rememberSubmission = useCallback((requestId: string, text: string, images: string[]) => {
        if (current.current?.characterId === characterId) return false;
        flushDraft();
        current.current = { characterId, requestId, text, images };
        const key = submissionKey(characterId);
        try {
            localStorage.setItem(key, JSON.stringify({ requestId, text }));
            // Keep attachments in the same session storage as the image draft.
            sessionStorage.setItem(key, JSON.stringify({ requestId, images }));
        } catch {
            // Storage failures still allow the live panel to consume the draft.
        }
        return true;
    }, [characterId, flushDraft]);

    const finishSubmission = useCallback((owner: string, requestId: string, persisted: boolean) => {
        const submission = current.current;
        if (submission?.characterId !== owner || submission.requestId !== requestId) return;
        const stored = readStoredSubmission(owner);
        if (persisted && (!stored || stored.requestId === requestId)) {
            consumeSentDraft(owner, submission.text, submission.images);
        }
        current.current = null;
        setRecoveringDraft(false);
        try {
            const key = submissionKey(owner);
            if (stored?.requestId === requestId) localStorage.removeItem(key);
            const savedImages = JSON.parse(sessionStorage.getItem(key) ?? "null");
            if (savedImages?.requestId === requestId) sessionStorage.removeItem(key);
        } catch {
            // Draft persistence is best effort when storage is unavailable.
        }
    }, [consumeSentDraft]);

    const reconcileSubmission = useCallback((owner: string, requestId: string) => {
        if (current.current?.characterId === owner && current.current.requestId === requestId) {
            setRecoveringDraft(true);
        }
    }, []);

    useLayoutEffect(() => {
        const submission = readStoredSubmission(characterId);
        current.current = submission;
        setRecoveringDraft(submission !== null);
        return () => { current.current = null; };
    }, [characterId]);

    useEffect(() => {
        const submission = current.current;
        if (!recoveringDraft || !submission || submission.characterId !== characterId) return;
        const { requestId } = submission;
        let stopped = false;
        let retry: ReturnType<typeof setTimeout> | undefined;
        async function check() {
            if (stopped || current.current !== submission) return;
            try {
                const status = await recoverChatDraftSubmission(characterId, requestId);
                if (stopped || current.current !== submission) return;
                if (status !== "pending") {
                    finishSubmission(characterId, requestId, status === "persisted");
                    return;
                }
            } catch {
                // A failed lookup cannot tell us whether admission or persistence won.
            }
            if (!stopped && current.current === submission) retry = setTimeout(check, 750);
        }
        void check();
        return () => {
            stopped = true;
            clearTimeout(retry);
        };
    }, [characterId, recoveringDraft, finishSubmission]);

    useLayoutEffect(() => {
        const submission = current.current;
        if (!submission) return;
        const turn = turns.find(turn => turn.client_request_id === submission.requestId);
        if (turn?.user_message_id != null) {
            finishSubmission(submission.characterId, submission.requestId, true);
        } else if (turn?.user_save_failed) {
            finishSubmission(submission.characterId, submission.requestId, false);
        } else if (turn && !isActiveTurn(turn)) {
            reconcileSubmission(submission.characterId, submission.requestId);
        }
    }, [turns, characterId, reconcileSubmission, finishSubmission]);

    return { recoveringDraft, rememberSubmission, finishSubmission, reconcileSubmission };
}
