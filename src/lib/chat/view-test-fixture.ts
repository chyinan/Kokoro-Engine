import { vi } from "vitest";
import type { ChatRequest, ChatViewSnapshot, StreamChatResponse } from "../kokoro-bridge";

const initial = (): ChatViewSnapshot => ({ revision: 1, history_revision: 0, character_id: "char-1", conversation_id: "conv-1", generation: 1, busy: false, messages: [], turns: [] });
let snapshot = initial();
const listeners = new Set<() => void>();
export const chatViewFixture = {
  getSnapshot: () => snapshot,
  subscribe: (callback: () => void) => { listeners.add(callback); return () => listeners.delete(callback); },
  set: (patch: Partial<ChatViewSnapshot>) => { snapshot = { ...snapshot, ...patch }; listeners.forEach(callback => callback()); },
  send: vi.fn<(request: ChatRequest) => Promise<StreamChatResponse>>(async () => ({ conversation_id: "conv-1", status: "completed", text: "reply" })),
  cancel: vi.fn(async () => {}), selectConversation: vi.fn(async () => {}),
  clear: vi.fn(async () => {}), edit: vi.fn(async () => {}), continueFrom: vi.fn(async () => {}),
  reset: () => { snapshot = initial(); for (const fn of [chatViewFixture.send, chatViewFixture.cancel, chatViewFixture.selectConversation, chatViewFixture.clear, chatViewFixture.edit, chatViewFixture.continueFrom]) fn.mockClear(); },
};
