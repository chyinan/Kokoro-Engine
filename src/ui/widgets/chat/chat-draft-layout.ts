// pattern: Functional Core

export const CHAT_DRAFT_KEY_PREFIX = "kokoro_chat_draft_";
export const DEFAULT_CHAT_DRAFT_DEBOUNCE_MS = 300;

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
