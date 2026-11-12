import type { ChatTurnSnapshot, ChatViewSnapshot } from "../kokoro-bridge";
import { buildChatMessagesFromConversation } from "./history";
import { buildToolTraceItem, getToolEventStateUpdate, mergeToolTraceItems, stripStreamingControlMarkup, type ChatPanelMessage, type PendingTurnState } from "./turn-state";

export const isActiveTurn = (turn: ChatTurnSnapshot) => ["pending", "accepted", "streaming"].includes(turn.status);

function isTurnAwaitingHistory(view: ChatViewSnapshot, turn: ChatTurnSnapshot): boolean {
  return turn.status === "completed" && turn.revision > view.revision && turn.character_id === view.character_id
    && turn.conversation_id !== null && turn.conversation_id === view.conversation_id;
}

/** Full snapshots replace older revisions; deltas are never replayed by views. */
export function mergeTurnSnapshot(turns: ReadonlyMap<string, ChatTurnSnapshot>, next: ChatTurnSnapshot): Map<string, ChatTurnSnapshot> {
  const previous = turns.get(next.client_request_id);
  if (previous && previous.revision >= next.revision) return new Map(turns);
  return new Map(turns).set(next.client_request_id, next);
}

export function projectSavedMessages(view: ChatViewSnapshot): ChatPanelMessage[] {
  // The journal holds temporary attachments. Even after completion, associate
  // them with the saved user row without restoring deleted rows or old text.
  const imagesByMessage = new Map<number, string[]>();
  for (const turn of view.turns) {
    if (!turn.hidden && turn.character_id === view.character_id && turn.conversation_id === view.conversation_id
      && turn.user_message_id !== null && turn.images.length) {
      imagesByMessage.set(turn.user_message_id, turn.images);
    }
  }
  return buildChatMessagesFromConversation(view.messages).map(message => {
    const images = message.role === "user" && message.id !== undefined ? imagesByMessage.get(message.id) : undefined;
    return images ? { ...message, images } : message;
  });
}

export function projectMessages(view: ChatViewSnapshot): ChatPanelMessage[] {
  let messages = projectSavedMessages(view);
  const turns = view.turns.filter(turn => turn.character_id === view.character_id && (!turn.conversation_id || turn.conversation_id === view.conversation_id));
  const latestTurnByUser = new Map<number, ChatTurnSnapshot>();
  for (const turn of turns) {
    if (turn.user_message_id === null) continue;
    const previous = latestTurnByUser.get(turn.user_message_id);
    if (!previous || turn.revision > previous.revision) latestTurnByUser.set(turn.user_message_id, turn);
  }
  for (const turn of turns) {
    const active = isActiveTurn(turn);
    // Completion can arrive before the history read. Keep the assistant row
    // mounted until a snapshot at or beyond that event takes over.
    const live = active || isTurnAwaitingHistory(view, turn);
    if (!live && (turn.status !== "completed" || turn.conversation_id !== view.conversation_id || !turn.tools.length)) continue;
    if (active && !turn.hidden && !messages.some(message => message.id === turn.user_message_id && turn.user_message_id !== null)) {
      messages.push({ role: "user", text: turn.message, images: turn.images, clientRequestId: turn.client_request_id });
    }
    const index = messages.findIndex(message => message.role === "kokoro" && (message.turnId === turn.turn_id || (turn.assistant_message_id !== null && message.id === turn.assistant_message_id)));
    if (!live) {
      // Text-mode results have no saved tool row. Supplement the saved reply
      // with finished journal results and let saved results take precedence.
      const tools = turn.tools.filter(event => event.approval_status !== "requested")
        .reduce<NonNullable<ChatPanelMessage["tools"]>>((traces, event) => mergeToolTraceItems(traces, buildToolTraceItem(event)), []);
      if (index >= 0) {
        const saved = messages[index];
        messages[index] = { ...saved, tools: (saved.tools ?? []).reduce(mergeToolTraceItems, tools) };
      } else if (tools.length && !turn.hidden && turn.assistant_message_id === null && turn.user_message_id !== null
        && !stripStreamingControlMarkup(turn.text).trim() && latestTurnByUser.get(turn.user_message_id) === turn) {
        // A tool-only reply never had a saved assistant row. Keep its live key
        // and place it with its saved prompt. Truncation or a newer turn on the
        // same prompt must not bring back the old result.
        const userIndex = messages.findIndex(message => message.role === "user" && message.id === turn.user_message_id);
        if (userIndex >= 0) messages.splice(userIndex + 1, 0, {
          role: "kokoro", text: "", turnId: turn.turn_id ?? undefined, clientRequestId: turn.client_request_id, tools,
        });
      }
      continue;
    }
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
