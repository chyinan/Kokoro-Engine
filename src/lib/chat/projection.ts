import type { ChatTurnSnapshot, ChatViewSnapshot } from "../kokoro-bridge";
import { buildChatMessagesFromConversation } from "./history";
import { getToolEventStateUpdate, stripStreamingControlMarkup, type ChatPanelMessage, type PendingTurnState } from "./turn-state";

export const isActiveTurn = (turn: ChatTurnSnapshot) => ["pending", "accepted", "streaming"].includes(turn.status);

/** Full snapshots replace older revisions; deltas are never replayed by views. */
export function mergeTurnSnapshot(turns: ReadonlyMap<string, ChatTurnSnapshot>, next: ChatTurnSnapshot): Map<string, ChatTurnSnapshot> {
  const previous = turns.get(next.client_request_id);
  if (previous && previous.revision >= next.revision) return new Map(turns);
  return new Map(turns).set(next.client_request_id, next);
}

export function projectMessages(view: ChatViewSnapshot): ChatPanelMessage[] {
  let messages: ChatPanelMessage[] = buildChatMessagesFromConversation(view.messages);
  const turns = view.turns.filter(turn => turn.character_id === view.character_id && (!turn.conversation_id || turn.conversation_id === view.conversation_id));
  // The journal holds temporary attachments. Even after completion, associate
  // them with the saved user row without restoring deleted rows or old text.
  const imagesByMessage = new Map<number, string[]>();
  for (const turn of turns) {
    if (!turn.hidden && turn.conversation_id === view.conversation_id && turn.user_message_id !== null && turn.images.length) {
      imagesByMessage.set(turn.user_message_id, turn.images);
    }
  }
  messages = messages.map(message => {
    const images = message.role === "user" && message.id !== undefined ? imagesByMessage.get(message.id) : undefined;
    return images ? { ...message, images } : message;
  });
  for (const turn of turns) {
    const active = isActiveTurn(turn);
    // Completion can arrive before the history read. Keep the assistant row
    // mounted until a snapshot at or beyond that event takes over.
    const awaitingHistory = turn.status === "completed" && turn.revision > view.revision
      && turn.conversation_id !== null && turn.conversation_id === view.conversation_id;
    if (!active && !awaitingHistory) continue;
    if (active && !turn.hidden && !messages.some(message => message.id === turn.user_message_id && turn.user_message_id !== null)) {
      messages.push({ role: "user", text: turn.message, images: turn.images, clientRequestId: turn.client_request_id });
    }
    const index = messages.findIndex(message => message.role === "kokoro" && (message.turnId === turn.turn_id || (turn.assistant_message_id !== null && message.id === turn.assistant_message_id)));
    const text = stripStreamingControlMarkup(turn.text);
    if (!text && !turn.tools.length) continue;
    const assistant: ChatPanelMessage = {
      role: "kokoro", text, turnId: turn.turn_id ?? undefined,
      clientRequestId: turn.client_request_id, id: turn.assistant_message_id ?? undefined,
      translation: turn.translation ?? undefined,
      tools: index >= 0 ? messages[index].tools : undefined,
    };
    const messageIndex = index >= 0 ? index : messages.length;
    if (index >= 0) messages[index] = assistant; else messages.push(assistant);
    const pending: PendingTurnState = {
      turnId: turn.turn_id ?? turn.client_request_id, messageIndex, rawText: turn.text,
      visibleTextStarted: Boolean(text), translationPending: false, tools: [],
    };
    for (const event of turn.tools) messages = getToolEventStateUpdate(event, pending, pending.turnId)(messages);
  }
  return messages;
}
