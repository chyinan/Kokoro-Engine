import type { ChatPanelMessage } from "./turn-state";

/**
 * A user message is the start of a turn, so continuing from it means
 * regenerating the assistant reply for that turn. Other message types keep
 * the existing branch behavior and only truncate the conversation.
 */
export function getContinueFromRegenerationIndex(
    messages: ReadonlyArray<ChatPanelMessage>,
    globalIndex: number,
): number | null {
    return messages[globalIndex]?.role === "user" ? globalIndex + 1 : null;
}
