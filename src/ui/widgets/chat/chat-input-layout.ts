// pattern: Functional Core

export const CHAT_INPUT_HEIGHT_STORAGE_KEY = "kokoro_chat_input_height";
export const DEFAULT_CHAT_INPUT_HEIGHT = 88;
export const MIN_CHAT_INPUT_HEIGHT = 82;
export const MAX_CHAT_INPUT_HEIGHT = 300;
export const EXPANDED_CHAT_INPUT_HEIGHT = 180;

/**
 * Clamps the input card height to valid bounds [MIN_CHAT_INPUT_HEIGHT, MAX_CHAT_INPUT_HEIGHT].
 * Falls back to DEFAULT_CHAT_INPUT_HEIGHT if value is NaN or invalid.
 */
export function clampChatInputHeight(height: number): number {
    if (typeof height !== "number" || Number.isNaN(height)) {
        return DEFAULT_CHAT_INPUT_HEIGHT;
    }
    return Math.max(MIN_CHAT_INPUT_HEIGHT, Math.min(MAX_CHAT_INPUT_HEIGHT, Math.round(height)));
}

/**
 * Computes next height when user double-clicks the drag handle to reset/toggle height.
 * If currently expanded (> DEFAULT), collapses back to DEFAULT. Otherwise expands to EXPANDED.
 */
export function toggleChatInputResetHeight(currentHeight: number): number {
    return currentHeight > DEFAULT_CHAT_INPUT_HEIGHT
        ? DEFAULT_CHAT_INPUT_HEIGHT
        : EXPANDED_CHAT_INPUT_HEIGHT;
}

/**
 * Computes resized height during drag gesture based on start height and pointer movement.
 */
export function computeResizedInputHeight(
    startHeight: number,
    startY: number,
    currentY: number
): number {
    const deltaY = startY - currentY;
    return clampChatInputHeight(startHeight + deltaY);
}

/**
 * Loads the saved input card height from storage, validating and clamping.
 */
export function loadSavedChatInputHeight(storage?: Storage): number {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return DEFAULT_CHAT_INPUT_HEIGHT;
        const raw = s.getItem(CHAT_INPUT_HEIGHT_STORAGE_KEY);
        if (!raw) return DEFAULT_CHAT_INPUT_HEIGHT;
        const parsed = Number.parseFloat(raw);
        return clampChatInputHeight(parsed);
    } catch {
        return DEFAULT_CHAT_INPUT_HEIGHT;
    }
}

/**
 * Saves the input card height to storage.
 */
export function saveChatInputHeight(height: number, storage?: Storage): void {
    try {
        const s = storage ?? (typeof window !== "undefined" ? window.localStorage : undefined);
        if (!s) return;
        const clamped = clampChatInputHeight(height);
        s.setItem(CHAT_INPUT_HEIGHT_STORAGE_KEY, String(clamped));
    } catch {
        // storage quota exceeded or disabled
    }
}
