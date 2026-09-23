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

/**
 * Return the last persisted visible message ID captured by the UI. The backend
 * uses it as a compare-and-swap guard before deleting a conversation suffix.
 */
export function getExpectedTailMessageId(
    messages: ReadonlyArray<ChatPanelMessage>,
): number | null {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
        const id = messages[index]?.id;
        if (typeof id === "number") return id;
    }
    return null;
}

export type ChatMessageSnapshot = ReadonlyArray<string>;

/**
 * Capture the fields that can change the meaning or position of a history
 * mutation. The snapshot deliberately includes unpersisted messages too:
 * those are exactly the rows that a backend CAS cannot see.
 */
export function getChatMessageSnapshot(
    messages: ReadonlyArray<ChatPanelMessage>,
): ChatMessageSnapshot {
    return messages.map(message => JSON.stringify([
        message.id ?? null,
        message.clientRequestId ?? null,
        message.turnId ?? null,
        message.role,
        message.text,
    ]));
}

export function matchesChatMessageSnapshot(
    messages: ReadonlyArray<ChatPanelMessage>,
    snapshot: ChatMessageSnapshot,
): boolean {
    const current = getChatMessageSnapshot(messages);
    return current.length === snapshot.length
        && current.every((value, index) => value === snapshot[index]);
}
