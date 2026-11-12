import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { getChatDraftSubmissionStatus, type ChatTurnSnapshot } from "../../../lib/kokoro-bridge";
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
    }, [characterId, flushDraft]);

    const finishSubmission = useCallback((owner: string, requestId: string, persisted: boolean) => {
        const submission = current.current;
        if (submission?.characterId !== owner || submission.requestId !== requestId) return;
        if (persisted) consumeSentDraft(owner, submission.text, submission.images);
        current.current = null;
        setRecoveringDraft(false);
        try {
            const key = submissionKey(owner);
            localStorage.removeItem(key);
            sessionStorage.removeItem(key);
        } catch {
            // Draft persistence is best effort when storage is unavailable.
        }
    }, [consumeSentDraft]);

    const checkSubmission = useCallback(async (submission: DraftSubmission, terminal: boolean) => {
        const { characterId, requestId } = submission;
        try {
            const status = await getChatDraftSubmissionStatus(characterId, requestId);
            if (current.current !== submission) return;
            if (status === "persisted" || (terminal && status === "unsent")) {
                finishSubmission(characterId, requestId, status === "persisted");
            } else if (status === "unsent") {
                // Keep watching the request in case admission races the lookup.
                setRecoveringDraft(false);
            }
        } catch {
            if (current.current === submission) setRecoveringDraft(false);
        }
    }, [finishSubmission]);

    useLayoutEffect(() => {
        const submission = readStoredSubmission(characterId);
        current.current = submission;
        setRecoveringDraft(submission !== null);
        if (submission) void checkSubmission(submission, false);
        return () => { current.current = null; };
    }, [characterId, checkSubmission]);

    useLayoutEffect(() => {
        const submission = current.current;
        if (!submission) return;
        const turn = turns.find(turn => turn.client_request_id === submission.requestId);
        if (turn?.user_message_id != null) {
            finishSubmission(submission.characterId, submission.requestId, true);
        } else if (turn && !isActiveTurn(turn)) {
            void checkSubmission(submission, true);
        }
    }, [turns, characterId, checkSubmission, finishSubmission]);

    return { recoveringDraft, rememberSubmission, finishSubmission };
}
