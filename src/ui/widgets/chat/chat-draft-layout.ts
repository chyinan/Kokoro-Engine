// pattern: Functional Core

export const CHAT_DRAFT_KEY_PREFIX = "kokoro_chat_draft_";
export const CHAT_DRAFT_IMAGES_KEY_PREFIX = "kokoro_chat_draft_images_";
export const DEFAULT_CHAT_DRAFT_DEBOUNCE_MS = 300;

/**
 * Returns the storage key for character image drafts.
 */
export function getCharacterDraftImagesStorageKey(characterId: string): string {
    const sanitized = encodeURIComponent(characterId.trim() || "default");
    return `${CHAT_DRAFT_IMAGES_KEY_PREFIX}${sanitized}`;
}

/**
 * Loads the saved character image draft from storage.
 * Returns array of image URLs if found and valid JSON, or empty array.
 */
export function loadSavedCharacterDraftImages(characterId: string, storage?: Storage): string[] {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return [];
        const key = getCharacterDraftImagesStorageKey(characterId);
        const saved = s.getItem(key);
        if (!saved) return [];
        const parsed = JSON.parse(saved);
        return Array.isArray(parsed)
            ? parsed.filter(item => typeof item === "string" && item.trim().length > 0)
            : [];
    } catch {
        return [];
    }
}

/**
 * Saves or clears the character image draft in storage.
 * If images array is empty, removes the item to avoid polluting storage.
 */
export function saveCharacterDraftImages(characterId: string, images: string[], storage?: Storage): void {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return;
        const key = getCharacterDraftImagesStorageKey(characterId);
        const validImages = images.filter(img => typeof img === "string" && img.trim().length > 0);
        if (validImages.length === 0) {
            s.removeItem(key);
        } else {
            s.setItem(key, JSON.stringify(validImages));
        }
    } catch {
        // storage disabled or quota exceeded
    }
}

/**
 * Clears the character image draft from storage immediately.
 */
export function clearCharacterDraftImages(characterId: string, storage?: Storage): void {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return;
        const key = getCharacterDraftImagesStorageKey(characterId);
        s.removeItem(key);
    } catch {
        // ignore storage errors
    }
}

/**
 * Returns the storage key for a given character id.
 * Uses encodeURIComponent to ensure special characters don't break key lookups.
 */
export function getCharacterDraftStorageKey(characterId: string): string {
    const sanitized = encodeURIComponent(characterId.trim() || "default");
    return `${CHAT_DRAFT_KEY_PREFIX}${sanitized}`;
}

/**
 * Loads the saved character draft from storage.
 * Returns the exact draft string if found, or empty string.
 */
export function loadSavedCharacterDraft(characterId: string, storage?: Storage): string {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return "";
        const key = getCharacterDraftStorageKey(characterId);
        const saved = s.getItem(key);
        return saved ?? "";
    } catch {
        return "";
    }
}

/**
 * Saves or clears the character draft in storage.
 * If text is empty or only whitespace, removes the item to avoid polluting storage.
 */
export function saveCharacterDraft(characterId: string, text: string, storage?: Storage): void {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return;
        const key = getCharacterDraftStorageKey(characterId);
        if (!text || text.trim().length === 0) {
            s.removeItem(key);
        } else {
            s.setItem(key, text);
        }
    } catch {
        // storage disabled or quota exceeded
    }
}

/**
 * Clears the character draft from storage immediately.
 */
export function clearCharacterDraft(characterId: string, storage?: Storage): void {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return;
        const key = getCharacterDraftStorageKey(characterId);
        s.removeItem(key);
    } catch {
        // ignore storage errors
    }
}

/**
 * Safely combines an existing user draft with real-time or final STT speech transcription.
 * Preserves the user's manual typed content while appending recognized speech with natural spacing/punctuation.
 */
export function combineDraftWithTranscription(baseDraft: string, transcription: string): string {
    const trimmedTranscription = transcription.trim();
    if (!trimmedTranscription) return baseDraft;
    if (!baseDraft) return trimmedTranscription;

    const trimmedBase = baseDraft.trimEnd();
    if (!trimmedBase) return trimmedTranscription;

    // 1. If base ends with a newline, preserve trailing newline
    if (/\n/.test(baseDraft.slice(-1))) {
        return baseDraft + trimmedTranscription;
    }

    const lastChar = trimmedBase.slice(-1);

    // 2. If base ends with Chinese/Japanese full-width punctuation
    const cjkPunctuation = /[，。！？；：、“”‘’（）《》【】…—]/;
    if (cjkPunctuation.test(lastChar)) {
        return trimmedBase + trimmedTranscription;
    }

    // 3. If base ends with Western punctuation (. , ! ? ; :)
    const westernPunctuation = /[.,!?;:]/;
    if (westernPunctuation.test(lastChar)) {
        return trimmedBase + " " + trimmedTranscription;
    }

    // 4. If both boundary characters are CJK ideographs
    const isCjkChar = /[\u4e00-\u9fa5\u3040-\u30ff\uac00-\ud7af]/.test(lastChar);
    const firstTransChar = trimmedTranscription.charAt(0);
    const isFirstCjk = /[\u4e00-\u9fa5\u3040-\u30ff\uac00-\ud7af]/.test(firstTransChar);

    if (isCjkChar && isFirstCjk) {
        // If user already typed a trailing space, preserve it
        if (/\s/.test(baseDraft.slice(-1))) {
            return baseDraft + trimmedTranscription;
        }
        return trimmedBase + trimmedTranscription;
    }

    // 5. Default: separate with a single space
    return trimmedBase + " " + trimmedTranscription;
}
