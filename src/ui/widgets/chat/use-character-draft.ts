// pattern: Imperative Shell

import { useCallback, useEffect, useRef, useState } from "react";
import {
    DEFAULT_CHAT_DRAFT_DEBOUNCE_MS,
    clearCharacterDraft,
    clearCharacterDraftImages,
    loadSavedCharacterDraft,
    loadSavedCharacterDraftImages,
    saveCharacterDraft,
    saveCharacterDraftImages,
} from "./chat-draft-layout";

export interface UseCharacterChatDraftOptions {
    readonly debounceMs?: number;
    readonly storage?: Storage;
}

export interface UseCharacterChatDraftResult {
    readonly input: string;
    readonly setInput: (value: string | ((prev: string) => string)) => void;
    readonly pendingImages: string[];
    readonly setPendingImages: (value: string[] | ((prev: string[]) => string[])) => void;
    readonly clearDraft: () => void;
    readonly flushDraft: () => void;
}

export function useCharacterChatDraft(
    characterId: string,
    options?: UseCharacterChatDraftOptions
): UseCharacterChatDraftResult {
    const debounceMs = options?.debounceMs ?? DEFAULT_CHAT_DRAFT_DEBOUNCE_MS;
    const storage = options?.storage;

    const [input, setInputState] = useState<string>(() =>
        loadSavedCharacterDraft(characterId, storage)
    );
    const [pendingImages, setPendingImagesState] = useState<string[]>(() =>
        loadSavedCharacterDraftImages(characterId, storage)
    );

    const inputRef = useRef(input);
    inputRef.current = input;
    const pendingImagesRef = useRef(pendingImages);
    pendingImagesRef.current = pendingImages;

    const activeCharacterIdRef = useRef(characterId);
    activeCharacterIdRef.current = characterId;

    const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const imageDebounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Flushes pending changes for a given character to storage immediately
    const flushDraftFor = useCallback(
        (targetCharId: string, text: string, images: string[]) => {
            if (debounceTimerRef.current !== null) {
                clearTimeout(debounceTimerRef.current);
                debounceTimerRef.current = null;
            }
            if (imageDebounceTimerRef.current !== null) {
                clearTimeout(imageDebounceTimerRef.current);
                imageDebounceTimerRef.current = null;
            }
            saveCharacterDraft(targetCharId, text, storage);
            saveCharacterDraftImages(targetCharId, images, storage);
        },
        [storage]
    );

    const flushDraft = useCallback(() => {
        flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current);
    }, [flushDraftFor]);

    // Handle character switching
    const prevCharacterIdRef = useRef(characterId);
    useEffect(() => {
        if (prevCharacterIdRef.current !== characterId) {
            // Flush old character's in-flight draft
            flushDraftFor(prevCharacterIdRef.current, inputRef.current, pendingImagesRef.current);

            // Load new character's draft
            const nextDraft = loadSavedCharacterDraft(characterId, storage);
            inputRef.current = nextDraft;
            setInputState(nextDraft);

            const nextImages = loadSavedCharacterDraftImages(characterId, storage);
            pendingImagesRef.current = nextImages;
            setPendingImagesState(nextImages);

            prevCharacterIdRef.current = characterId;
        }
    }, [characterId, flushDraftFor, storage]);

    // Set input with debounced persistence (timer managed outside setState updater)
    const setInput = useCallback(
        (value: string | ((prev: string) => string)) => {
            if (debounceTimerRef.current !== null) {
                clearTimeout(debounceTimerRef.current);
                debounceTimerRef.current = null;
            }

            const next = typeof value === "function" ? value(inputRef.current) : value;
            inputRef.current = next;
            setInputState(next);

            const targetCharId = activeCharacterIdRef.current;
            debounceTimerRef.current = setTimeout(() => {
                debounceTimerRef.current = null;
                saveCharacterDraft(targetCharId, inputRef.current, storage);
            }, debounceMs);
        },
        [debounceMs, storage]
    );

    // Set pending images with debounced persistence
    const setPendingImages = useCallback(
        (value: string[] | ((prev: string[]) => string[])) => {
            if (imageDebounceTimerRef.current !== null) {
                clearTimeout(imageDebounceTimerRef.current);
                imageDebounceTimerRef.current = null;
            }

            const next = typeof value === "function" ? value(pendingImagesRef.current) : value;
            pendingImagesRef.current = next;
            setPendingImagesState(next);

            const targetCharId = activeCharacterIdRef.current;
            imageDebounceTimerRef.current = setTimeout(() => {
                imageDebounceTimerRef.current = null;
                saveCharacterDraftImages(targetCharId, pendingImagesRef.current, storage);
            }, debounceMs);
        },
        [debounceMs, storage]
    );

    // Clear draft immediately (called on submit or auto-send)
    const clearDraft = useCallback(() => {
        if (debounceTimerRef.current !== null) {
            clearTimeout(debounceTimerRef.current);
            debounceTimerRef.current = null;
        }
        if (imageDebounceTimerRef.current !== null) {
            clearTimeout(imageDebounceTimerRef.current);
            imageDebounceTimerRef.current = null;
        }
        clearCharacterDraft(activeCharacterIdRef.current, storage);
        clearCharacterDraftImages(activeCharacterIdRef.current, storage);
        inputRef.current = "";
        setInputState("");
        pendingImagesRef.current = [];
        setPendingImagesState([]);
    }, [storage]);

    // Flush on unmount or beforeunload
    useEffect(() => {
        const handleBeforeUnload = () => {
            flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current);
        };

        if (typeof window !== "undefined") {
            window.addEventListener("beforeunload", handleBeforeUnload);
        }

        return () => {
            if (typeof window !== "undefined") {
                window.removeEventListener("beforeunload", handleBeforeUnload);
            }
            flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current);
        };
    }, [flushDraftFor]);

    return {
        input,
        setInput,
        pendingImages,
        setPendingImages,
        clearDraft,
        flushDraft,
    };
}
