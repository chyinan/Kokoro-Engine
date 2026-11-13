import { vi } from "vitest";
import { ChatApplication, emptyChatView, type ChatTransport } from "../application";
import type { ChatTurnSnapshot, ChatViewSnapshot, StreamChatResponse } from "../../kokoro-bridge";

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
export function turn(id: string, revision: number, status: ChatTurnSnapshot["status"] = "streaming"): ChatTurnSnapshot {
  return { revision, client_request_id: id, turn_id: `turn_${id}`, character_id: "character-a", conversation_id: "conversation-a", user_message_id: 1, assistant_message_id: null, message: "question", images: [], hidden: false, status, text: "answer", translation: null, tools: [], error: null };
}
export function harness() {
  let view: ChatViewSnapshot = { ...emptyChatView, character_id: "character-a", conversation_id: "conversation-a", conversation_history_revision: 1, generation: 3 };
  const callbacks = new Set<(snapshot: ChatTurnSnapshot) => void>();
  const io: ChatTransport = {
    read: vi.fn(async () => structuredClone(view)),
    listen: vi.fn(async callback => { callbacks.add(callback); return () => { callbacks.delete(callback); }; }),
    send: vi.fn(async request => ({ status: "completed", conversation_id: "conversation-a", client_request_id: request.client_request_id, text: "answer" })),
    cancel: vi.fn(async () => {}),
  };
  const app = new ChatApplication(io);
  return { app, io, newWindow: () => new ChatApplication(io),
    set(next: Partial<ChatViewSnapshot>) { view = { ...view, ...next }; },
    emit(snapshot: ChatTurnSnapshot) { for (const callback of callbacks) callback(snapshot); },
    pending() { const result = deferred<StreamChatResponse>(); vi.mocked(io.send).mockReturnValueOnce(result.promise); return result; },
  };
}
