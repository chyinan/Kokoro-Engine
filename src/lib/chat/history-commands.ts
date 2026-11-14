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

// A suffix deletion is safe only if it can keep the selected tool exchange
// without also keeping intervening rows from a different turn.
export function getContinueFromRowBoundary(view: ChatViewSnapshot, selectedId: number, keptId: number): number | null {
    const selectedIndex = view.messages.findIndex(row => row.id === selectedId);
    const selected = view.messages[selectedIndex];
    if (!selected) return null;
    const rows = view.messages.map(row => {
        const meta = rowMetadata(row);
        return { row, turnId: meta?.turn_id ?? meta?.event?.turn_id, promptId: meta?.user_message_id,
            tool: isToolRow(row, meta?.type) };
    });
    const turns = view.turns.filter(turn => turn.character_id === view.character_id
        && turn.conversation_id === view.conversation_id);
    const toolTurns = new Set<string>();
    const toolOwners = new Map<string, Set<number>>();
    const recordToolOwner = (turnId: string, promptId: number | null) => {
        const owners = toolOwners.get(turnId) ?? new Set<number>();
        if (promptId !== null) owners.add(promptId);
        toolOwners.set(turnId, owners);
    };
    for (const turn of turns) {
        if (turn.turn_id) {
            recordToolOwner(turn.turn_id, turn.hidden ? null : turn.user_message_id);
        }
    }
    for (const { tool, turnId, promptId } of rows) {
        if (!tool || !turnId) continue;
        toolTurns.add(turnId);
        if (promptId !== undefined) recordToolOwner(turnId, promptId);
    }

    const keptIndex = keptId === selectedId ? -1 : rows.findIndex(({ row }) => row.id === keptId);
    const keptTurn = rows[keptIndex]?.row.role === "assistant" ? rows[keptIndex].turnId : undefined;
    if (selected.role === "user") {
        if (keptIndex >= 0) {
            // The first visible reply after this prompt may belong to an earlier turn.
            const owners = keptTurn ? toolOwners.get(keptTurn) : undefined;
            if ((owners && [...owners].some(id => id !== selectedId)) || turns.some(turn =>
                turn.assistant_message_id === keptId && !turn.hidden
                && turn.user_message_id !== null && turn.user_message_id !== selectedId)) return null;
        }
        const adjacent = rows[selectedIndex + 1];
        if (adjacent?.tool && adjacent.turnId && !toolOwners.has(adjacent.turnId)) {
            // Legacy tool rows without saved or journal ownership use adjacency.
            recordToolOwner(adjacent.turnId, selectedId);
        }
    }
    const selectedTurn = selected.role === "assistant" ? rows[selectedIndex].turnId : undefined;
    const ownsToolTurn = (turnId: string | undefined) =>
        turnId !== undefined && toolTurns.has(turnId)
        && (selected.role === "user" ? toolOwners.get(turnId)?.has(selectedId) === true : turnId === selectedTurn);
    let boundary = -1;
    for (let index = selectedIndex + 1; index < rows.length; index += 1) {
        const entry = rows[index];
        if (ownsToolTurn(entry.turnId) && (selected.role === "user" || entry.tool)) boundary = index;
    }
    if (boundary >= 0 && rows.slice(selectedIndex + 1, boundary + 1).some(({ turnId }) => !ownsToolTurn(turnId))) {
        return null;
    }
    if (boundary < 0 && keptTurn) {
        // Legacy replies may link their tool rows only by turn ID. Preserve
        // the trailing rows of the reply already chosen by the visible cutoff.
        for (let index = keptIndex + 1; index < rows.length; index += 1) {
            const entry = rows[index];
            if (entry.turnId !== keptTurn || !entry.tool) break;
            boundary = index;
        }
    }
    const cutoff = boundary >= 0 ? rows[boundary].row.id : keptId;
    if (cutoff === undefined) return null;
    // Selecting the interleaved message itself can also split an earlier
    // tool exchange whose prompt, assistant reply or tool rows remain.
    const keptToolTurns = new Set(rows.flatMap(({ row, tool, turnId }) =>
        turnId && toolTurns.has(turnId) && (tool || row.role === "assistant")
            && row.id !== undefined && row.id <= cutoff ? [turnId] : []));
    const cutsEarlierTurn = rows.some(({ row, turnId }) => {
        if (!turnId || !toolTurns.has(turnId) || row.id === undefined || row.id <= cutoff) return false;
        const owners = toolOwners.get(turnId);
        return keptToolTurns.has(turnId) || (owners !== undefined && [...owners].some(id => id <= cutoff));
    });
    return cutsEarlierTurn ? null : cutoff;
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
