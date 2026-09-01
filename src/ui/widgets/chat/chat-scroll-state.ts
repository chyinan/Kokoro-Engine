// pattern: Functional Core

export interface ChatScrollSnapshot {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
    isAtBottom: boolean;
}

/**
 * Returns true if the scroll position is within `threshold` pixels of the bottom.
 */
export function isScrollAtBottom(
    scrollTop: number,
    scrollHeight: number,
    clientHeight: number,
    threshold: number = 40
): boolean {
    return scrollHeight - scrollTop - clientHeight < threshold;
}

/**
 * Computes the target scrollTop and userScrolled state when restoring scroll position.
 * If there is no snapshot or the user was at the bottom, restores to the bottom (currentScrollHeight).
 * If the user was scrolled up reading history, restores to the previous snapshot.scrollTop.
 */
export function computeTargetScrollTop(
    snapshot: ChatScrollSnapshot | null,
    currentScrollHeight: number,
    currentClientHeight: number
): { scrollTop: number; userScrolled: boolean } {
    if (!snapshot || snapshot.isAtBottom) {
        return {
            scrollTop: currentScrollHeight,
            userScrolled: false,
        };
    }

    // Clamp between 0 and maximum possible scroll
    const maxScroll = Math.max(0, currentScrollHeight - currentClientHeight);
    const clampedScrollTop = Math.min(Math.max(0, snapshot.scrollTop), maxScroll);

    return {
        scrollTop: clampedScrollTop,
        userScrolled: true,
    };
}
