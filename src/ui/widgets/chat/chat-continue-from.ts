import type { ChatPanelMessage } from "./turn-state";

/**
 * Return the number of visible messages to keep when jumping back to a
 * message. For a user message, keep its existing assistant reply (and any
 * context/tool rows between them) instead of generating a new reply.
 */
export function getContinueFromCutoffIndex(
    messages: ReadonlyArray<ChatPanelMessage>,
    globalIndex: number,
): number | null {
    const selected = messages[globalIndex];
    if (!selected) return null;

    if (selected.role !== "user") {
        return globalIndex + 1;
    }

    let nextIndex = globalIndex + 1;
    while (nextIndex < messages.length) {
        const next = messages[nextIndex];
        if (next.role === "kokoro") {
            return nextIndex + 1;
        }
        if (next.role !== "context" && next.role !== "tool") {
            break;
        }
        nextIndex += 1;
    }

    return globalIndex + 1;
}
