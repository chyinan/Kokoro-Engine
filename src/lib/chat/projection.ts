import type { ChatTurnSnapshot, ChatViewSnapshot } from "../kokoro-bridge";
import { buildChatMessagesFromConversation } from "./history";
import { buildToolTraceItem, getToolEventStateUpdate, mergeToolTraceItems, stripStreamingControlMarkup, type ChatFailureScope, type ChatPanelMessage, type PendingTurnState } from "./turn-state";

export const isActiveTurn = (turn: ChatTurnSnapshot) => ["pending", "accepted", "streaming"].includes(turn.status);

const replyRevision = (turn: ChatTurnSnapshot) => turn.completion_revision ?? turn.revision;

function isTurnAwaitingHistory(view: ChatViewSnapshot, turn: ChatTurnSnapshot): boolean {
  return turn.status === "completed" && replyRevision(turn) > view.revision && turn.character_id === view.character_id
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
  const speechErrorsByMessage = new Map<number, string>();
  const savedRequests = new Map<number, string>();
  for (const message of view.messages) {
    if (message.role !== "user" || message.id === undefined || !message.metadata) continue;
    try {
      const requestId: unknown = JSON.parse(message.metadata)?.client_request_id;
      if (typeof requestId === "string" && requestId.trim()) savedRequests.set(message.id, requestId);
    } catch { /* Legacy or malformed metadata cannot prove attachment ownership. */ }
  }
  for (const turn of view.turns) {
    if (turn.character_id !== view.character_id || turn.conversation_id !== view.conversation_id) continue;
    if (!turn.hidden && turn.user_message_id !== null && turn.images.length
      && savedRequests.get(turn.user_message_id) === turn.client_request_id) {
      imagesByMessage.set(turn.user_message_id, turn.images);
    }
    if (turn.status === "completed" && turn.assistant_message_id !== null && turn.speech_error) {
      speechErrorsByMessage.set(turn.assistant_message_id, turn.speech_error);
    }
  }
  return buildChatMessagesFromConversation(view.messages).map(message => {
    if (message.id === undefined) return message;
    const images = message.role === "user" ? imagesByMessage.get(message.id) : undefined;
    const speechError = message.role === "kokoro" ? speechErrorsByMessage.get(message.id) : undefined;
    return images ? { ...message, images } : speechError ? { ...message, speechError } : message;
  });
}

function isSelectedFailure(view: ChatViewSnapshot, scope: ChatFailureScope): boolean {
  return scope.character_id === view.character_id && scope.conversation_id === view.conversation_id;
}

function findFailureTurn(view: ChatViewSnapshot, scope: ChatFailureScope): ChatTurnSnapshot | undefined {
  return view.turns.find(turn => turn.status === "error" && turn.turn_id === scope.turn_id
    && turn.character_id === scope.character_id && turn.conversation_id === scope.conversation_id);
}

export function findFailurePromptIndex(view: ChatViewSnapshot, messages: readonly ChatPanelMessage[], diagnosticIndex: number): number {
  const scope = messages[diagnosticIndex]?.failureScope;
  if (!scope || !isSelectedFailure(view, scope)) return -1;
  // Older diagnostics can recover their exact prompt from a retained journal entry.
  let userId = scope.user_message_id;
  if (userId === undefined) {
    const legacyTurn = findFailureTurn(view, scope);
    if (!legacyTurn || legacyTurn.hidden) return -1;
    userId = legacyTurn.user_message_id;
  }
  if (userId == null) return -1;
  return messages.findIndex((message, index) => index < diagnosticIndex && message.role === "user" && message.id === userId);
}

export function projectMessages(view: ChatViewSnapshot): ChatPanelMessage[] {
  const turns = view.turns.filter(turn => turn.character_id === view.character_id && (!turn.conversation_id || turn.conversation_id === view.conversation_id));
  const shownFailures = new Set<string>();
  let messages = projectSavedMessages(view).flatMap((message, index, saved) => {
    const scope = message.failureScope;
    if (!scope) return [message];
    if (!isSelectedFailure(view, scope) || shownFailures.has(scope.turn_id)) return [];
    shownFailures.add(scope.turn_id);
    return [{ ...message, isError: true, announceError: false,
      clientRequestId: findFailureTurn(view, scope)?.client_request_id,
      canRetryFailure: findFailurePromptIndex(view, saved, index) >= 0 }];
  });
  const latestTurnByUser = new Map<number, ChatTurnSnapshot>();
  for (const turn of turns) {
    if (turn.user_message_id === null) continue;
    const previous = latestTurnByUser.get(turn.user_message_id);
    if (!previous || replyRevision(turn) > replyRevision(previous)) latestTurnByUser.set(turn.user_message_id, turn);
  }
  for (const turn of turns) {
    // A preparation timeout may have no SQLite diagnostic or even a user row.
    // Keep it scoped to the selection where preparation took place.
    const preparationFailure = turn.preparation_timed_out && turn.conversation_generation === view.generation;
    const legacyFailure = turn.client_request_id.startsWith("mod_") && turn.user_message_id !== null;
    const persistenceFailure = turn.error_code === "CHAT_PERSISTENCE_ERROR"
      && (turn.conversation_id !== null || turn.conversation_generation === view.generation);
    if (turn.status === "error" && turn.error && !turn.hidden && (preparationFailure || legacyFailure || persistenceFailure)
      && turn.turn_id && turn.conversation_id === view.conversation_id && !shownFailures.has(turn.turn_id)) {
      const promptIndex = turn.user_message_id !== null && latestTurnByUser.get(turn.user_message_id) === turn
        ? messages.findIndex(message => message.role === "user" && message.id === turn.user_message_id) : -1;
      const initialSaveFailure = persistenceFailure && turn.user_message_id === null
        && turn.conversation_generation === view.generation;
      if (promptIndex >= 0 || initialSaveFailure || (preparationFailure && turn.user_message_id === null)) {
        let replyIndex = messages.findIndex(message => message.role === "kokoro" && message.turnId === turn.turn_id);
        const partial = persistenceFailure && stripStreamingControlMarkup(turn.text).trim();
        if (partial && replyIndex < 0) {
          replyIndex = promptIndex >= 0 ? promptIndex + 1 : messages.length;
          messages.splice(replyIndex, 0, { role: "kokoro", text: partial, turnId: turn.turn_id, unfinished: true });
        }
        const precedingIndex = Math.max(promptIndex, replyIndex);
        const index = precedingIndex >= 0 ? precedingIndex + 1 : messages.length;
        messages.splice(index, 0, {
          role: "kokoro", text: turn.error, isError: true, announceError: false, clientRequestId: turn.client_request_id,
          ...(persistenceFailure ? { persistenceFailure: true } : {}),
          ...(persistenceFailure && turn.persistence_retryable ? { canRetryPersistence: true } : {}),
          failureScope: { character_id: view.character_id, conversation_id: turn.conversation_id, turn_id: turn.turn_id },
        });
        shownFailures.add(turn.turn_id);
      }
    }
    const active = isActiveTurn(turn);
    // Completion can arrive before the history read. Keep the assistant row
    // mounted until a snapshot at or beyond that event takes over.
    const live = active || isTurnAwaitingHistory(view, turn);
    if (!live && (turn.status !== "completed" || turn.conversation_id !== view.conversation_id || !turn.tools.length)) continue;
    if (active && !turn.hidden && !turn.reusing_saved_prompt && !messages.some(message => message.id === turn.user_message_id && turn.user_message_id !== null)) {
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
      speechError: index >= 0 ? messages[index].speechError : undefined,
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
