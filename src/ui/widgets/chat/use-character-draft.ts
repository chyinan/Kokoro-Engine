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
    checkImageAccessible,
} from "./chat-draft-layout";

export interface UseCharacterChatDraftOptions {
    readonly debounceMs?: number;
    readonly storage?: Storage;
    readonly imageStorage?: Storage;
    readonly validateImage?: (url: string) => Promise<boolean>;
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
    const imageStorage = options?.imageStorage ?? options?.storage;
    const validateImage = options?.validateImage ?? checkImageAccessible;

    const [input, setInputState] = useState<string>(() =>
        loadSavedCharacterDraft(characterId, storage)
    );
    const [pendingImages, setPendingImagesState] = useState<string[]>(() =>
        loadSavedCharacterDraftImages(characterId, imageStorage)
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
            saveCharacterDraftImages(targetCharId, images, imageStorage);
        },
        [imageStorage, storage]
    );

    const validateAndPruneImages = useCallback(
        async (targetCharId: string, candidateImages: string[]) => {
            if (candidateImages.length === 0) return;
            const results = await Promise.all(
                candidateImages.map(async (url) => ({
                    url,
                    valid: await validateImage(url),
                }))
            );
            // Ignore if active character changed during asynchronous validation
            if (activeCharacterIdRef.current !== targetCharId) return;

            const surviving = results.filter(r => r.valid).map(r => r.url);
            if (surviving.length !== candidateImages.length) {
                pendingImagesRef.current = surviving;
                setPendingImagesState(surviving);
                saveCharacterDraftImages(targetCharId, surviving, imageStorage);
            }
        },
        [imageStorage, validateImage]
    );

    // Validate initial draft images on mount
    useEffect(() => {
        const initial = loadSavedCharacterDraftImages(characterId, imageStorage);
        if (initial.length > 0) {
            void validateAndPruneImages(characterId, initial);
        }
    }, [characterId, imageStorage, validateAndPruneImages]);

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

            const nextImages = loadSavedCharacterDraftImages(characterId, imageStorage);
            pendingImagesRef.current = nextImages;
            setPendingImagesState(nextImages);
            if (nextImages.length > 0) {
                void validateAndPruneImages(characterId, nextImages);
            }

            prevCharacterIdRef.current = characterId;
        }
    }, [characterId, flushDraftFor, imageStorage, storage, validateAndPruneImages]);

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
                saveCharacterDraftImages(targetCharId, pendingImagesRef.current, imageStorage);
            }, debounceMs);
        },
        [debounceMs, imageStorage]
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
        clearCharacterDraftImages(activeCharacterIdRef.current, imageStorage);
        inputRef.current = "";
        setInputState("");
        pendingImagesRef.current = [];
        setPendingImagesState([]);
    }, [imageStorage, storage]);

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
