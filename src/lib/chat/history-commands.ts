import type { ChatViewSnapshot, ConversationMessage } from "../kokoro-bridge";
import { isActiveTurn } from "./projection";
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

function rowMetadata(row: ConversationMessage): { client_request_id?: string; turn_id?: string; type?: string;
    event?: { turn_id?: string }; streaming_draft?: boolean; user_message_id?: number | null } | null {
    try {
        return row.metadata ? JSON.parse(row.metadata) : null;
    } catch {
        return null;
    }
}

function isToolRow(row: ConversationMessage, type?: string): boolean {
    return row.role === "tool" || type === "assistant_tool_calls" || type === "tool_result";
}

// Streaming drafts can be assigned an ID before their final tool results.
// Keep those results with the reply when they precede the next turn.
export function getContinueFromRowBoundary(view: ChatViewSnapshot, selectedId: number, keptId: number): number {
    const rows = view.messages;
    const keptIndex = rows.findIndex(row => row.id === keptId);
    if (keptIndex < 0) return keptId;
    let boundaryIndex = keptIndex;
    let turnId = rowMetadata(rows[keptIndex])?.turn_id;
    const selectedIndex = rows.findIndex(row => row.id === selectedId);
    if (rows[selectedIndex]?.role === "user") {
        // A tool-only reply is represented by its technical rows. There is no
        // assistant message ID for the visible history cutoff to retain.
        const next = rows[selectedIndex + 1];
        const meta = next && rowMetadata(next);
        if (next && meta?.turn_id && isToolRow(next, meta.type)) {
            const owner = view.turns.find(turn => turn.turn_id === meta.turn_id
                && turn.character_id === view.character_id && turn.conversation_id === view.conversation_id);
            // Old rows lack an explicit owner. Use the journal when it still
            // has the turn, then fall back to the adjacent prompt.
            const belongsToPrompt = meta.user_message_id !== undefined
                ? meta.user_message_id === selectedId
                : !owner || (!owner.hidden && owner.user_message_id === selectedId);
            const keptTurn = turnId ?? rowMetadata(rows[keptIndex])?.event?.turn_id;
            if (belongsToPrompt && keptTurn !== meta.turn_id) {
                boundaryIndex = selectedIndex;
                turnId = meta.turn_id;
            }
        }
    }
    let boundary = rows[boundaryIndex].id ?? keptId;
    if (!turnId) return boundary;
    for (const row of rows.slice(boundaryIndex + 1)) {
        const meta = rowMetadata(row);
        if (row.id === undefined || meta?.turn_id !== turnId
            || !isToolRow(row, meta.type)) break;
        boundary = row.id;
    }
    return boundary;
}

/**
 * The click includes saved rows and terminal events the panel already knows
 * about. A refresh may reconcile those turns, but must not silently include
 * another window's new messages in a destructive operation.
 */
export function canTruncateRefreshedHistory(before: ChatViewSnapshot, after: ChatViewSnapshot): boolean {
    // A previously active turn may already have persisted its draft in the
    // cached rows, even if its terminal event has not reached this window.
    if ([before, after].some(view => view.turns.some(turn => isActiveTurn(turn)
        && (turn.character_id === null || turn.character_id === view.character_id)
        && (turn.conversation_id === null || turn.conversation_id === view.conversation_id)))) return false;
    const clickRevision = before.turns.reduce((revision, turn) => Math.max(revision, turn.revision), before.revision);
    // The backend samples the journal before reading SQLite. A turn may
    // already have rows in the clicked snapshot but only appear in the
    // journal after the click. Preserve it even if the rows did not change.
    const finishedAtClick = new Set(before.turns.filter(turn =>
        turn.status === "completed" || turn.status === "cancelled" || turn.status === "error"
    ).map(turn => turn.client_request_id));
    if (after.turns.some(turn => turn.character_id === before.character_id
        && turn.conversation_id === before.conversation_id
        && !finishedAtClick.has(turn.client_request_id)
        && (turn.completion_revision ?? turn.revision) > clickRevision
        && (turn.status === "completed" || turn.status === "cancelled" || turn.status === "error")
        && (turn.user_message_id !== null || turn.assistant_message_id !== null || turn.tools.length > 0))) return false;
    const terminal = before.turns.filter(turn =>
        (turn.status === "completed" || turn.status === "cancelled" || turn.status === "error")
        && turn.character_id === before.character_id && turn.conversation_id === before.conversation_id
        && (turn.completion_revision ?? turn.revision) > before.revision);
    if (terminal.some(turn => (turn.completion_revision ?? turn.revision) > after.revision)) return false;
    if (before.conversation_history_revision === after.conversation_history_revision) return true;
    if (!terminal.length) return false;

    const requestIds = new Set(terminal.map(turn => turn.client_request_id));
    const turnIds = new Set(terminal.flatMap(turn => turn.turn_id ? [turn.turn_id] : []));
    const belongsToFinishedTurn = (row: ConversationMessage) => {
        const meta = rowMetadata(row);
        return (meta?.client_request_id !== undefined && requestIds.has(meta.client_request_id))
            || (meta?.turn_id !== undefined && turnIds.has(meta.turn_id))
            || (meta?.type === "failure_event" && meta.event?.turn_id !== undefined && turnIds.has(meta.event.turn_id));
    };
    const rows = new Map(after.messages.map(row => [row.id, row]));
    for (const previous of before.messages) {
        const current = rows.get(previous.id);
        rows.delete(previous.id);
        if (!current) {
            if (!belongsToFinishedTurn(previous)) return false;
        } else if (previous.role !== current.role || previous.content !== current.content
            || previous.metadata !== current.metadata || previous.created_at !== current.created_at) {
            if (!belongsToFinishedTurn(previous) || !belongsToFinishedTurn(current)
                || rowMetadata(previous)?.streaming_draft !== true) return false;
        }
    }
    return [...rows.values()].every(belongsToFinishedTurn);
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
