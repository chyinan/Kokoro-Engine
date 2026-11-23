// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback } from "@tauri-apps/api/event";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { chatApplication, emptyChatView } from "@/lib/chat/application";
import { deferred, turn } from "@/lib/chat/__tests__/harness";
import type { ChatTurnSnapshot, ChatViewSnapshot, EditConversationMessageRequest, EditConversationMessageResponse } from "@/lib/kokoro-bridge";
import ChatPanel from "../ChatPanel";

vi.mock("@tauri-apps/api/core", async original => ({ ...await original<typeof import("@tauri-apps/api/core")>(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] }));
afterEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockReset();
  vi.useRealTimers();
});

async function mountChat(initial: Partial<ChatViewSnapshot> = {}, messageId = 101) {
  const snapshot: ChatViewSnapshot = {
    ...emptyChatView, character_id: "character-a", conversation_id: "conversation-a", generation: 1,
    revision: chatApplication.getSnapshot().revision + 1,
    history_revision: 1, conversation_history_revision: 1,
    messages: [{ id: 101, role: "user", content: "Original message", created_at: "2026-11-18T00:00:00Z", metadata: null }],
    ...initial,
  };
  const read = vi.fn(async () => structuredClone(snapshot));
  async function commitEdit(request: EditConversationMessageRequest) {
    const row = snapshot.messages.find(message => message.id === request.message_id);
    if (!row) throw new Error("Message not found");
    row.content = request.new_content;
    snapshot.history_revision++;
    snapshot.conversation_history_revision = snapshot.history_revision;
    return { message_id: row.id, updated_content: request.new_content };
  }
  const write = vi.fn(commitEdit);
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "get_chat_view") return read();
    if (command === "edit_conversation_message") return write((args as { request: EditConversationMessageRequest }).request);
    if (command === "list_conversations") return [];
  });

  let onRuntimeChanged: EventCallback<unknown> | undefined;
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    if (event === "chat-runtime-changed") onRuntimeChanged = handler;
    return () => {};
  });

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<ChatPanel />));
  const row = () => container.querySelector(`[data-chat-message-id="${messageId}"]`)!;
  const action = (name: string) => row().querySelector<HTMLButtonElement>(`button[title="chat.actions.${name}"]`);
  const editor = () => row().querySelector<HTMLTextAreaElement>("textarea");
  return {
    container, snapshot, read, write, commitEdit, row, action, editor,
    complete(reply: ChatTurnSnapshot) {
      if (!onRuntimeChanged) throw new Error("Chat runtime listener not registered");
      onRuntimeChanged({ event: "chat-runtime-changed", id: reply.revision, payload: reply });
    },
    async change(text: string) {
      await act(async () => {
        const input = editor()!;
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

const readError = { code: "Database", message: "History temporarily unavailable" };
const notice = "chat.errors.edit_refresh_failed";

it("shows a committed edit while history is pending and retries only the history read", async () => {
  const chat = await mountChat();
  const editing = deferred<void>();
  const history = deferred<ChatViewSnapshot>();
  try {
    await act(async () => chat.action("edit")!.click());
    await chat.change("Saved message");
    const initialReads = chat.read.mock.calls.length;
    chat.write.mockImplementationOnce(async request => {
      await editing.promise;
      return chat.commitEdit(request);
    });
    chat.read.mockReturnValueOnce(history.promise);
    await act(async () => chat.action("save")!.click());
    expect(chat.action("save")?.disabled).toBe(true);
    expect(chat.read).toHaveBeenCalledTimes(initialReads);

    await act(async () => editing.resolve());

    expect(chat.read).toHaveBeenCalledTimes(initialReads + 1);
    expect(chat.snapshot.conversation_history_revision).toBe(2);
    expect(chat.write).toHaveBeenCalledExactlyOnceWith({
      conversation_id: "conversation-a", conversation_generation: 1, message_id: 101, new_content: "Saved message",
    });
    expect(chat.editor()).toBeNull();
    expect(chat.row().querySelector(".whitespace-pre-wrap")?.textContent).toBe("Saved message");
    expect(chat.row().querySelector('[role="alert"]')).toBeNull();
    expect(chat.action("edit")).toBeNull();
    expect(chat.action("retry_refresh")).toBeNull();

    await act(async () => history.reject(readError));

    expect(chat.row().querySelector('[role="alert"]')?.textContent).toBe(notice);
    expect(chat.action("save")).toBeNull();
    expect(chat.action("edit")).toBeNull();

    const readCount = chat.read.mock.calls.length;
    chat.read.mockRejectedValueOnce(readError);
    await act(async () => chat.action("retry_refresh")!.click());
    expect(chat.row().textContent).toContain(notice);
    expect(chat.read).toHaveBeenCalledTimes(readCount + 1);

    await act(async () => chat.action("retry_refresh")!.click());
    expect(chat.read).toHaveBeenCalledTimes(readCount + 2);
    expect(chat.write).toHaveBeenCalledTimes(1);
    expect(chat.snapshot.conversation_history_revision).toBe(2);
    expect(chat.action("retry_refresh")).toBeNull();
    await act(async () => chat.action("edit")!.click());
    expect(chat.editor()?.value).toBe("Saved message");
  } finally {
    await act(async () => {
      editing.resolve();
      history.resolve(structuredClone(chat.snapshot));
    });
    await chat.unmount();
  }
});

it("shows the write error and allows a save retry while history is pending", async () => {
  const chat = await mountChat();
  const editing = deferred<EditConversationMessageResponse>();
  const history = deferred<ChatViewSnapshot>();
  const retryHistory = deferred<ChatViewSnapshot>();
  const staleHistory = structuredClone(chat.snapshot);
  try {
    await act(async () => chat.action("edit")!.click());
    await chat.change("Draft to keep");
    const writeError = { code: "Database", message: "Write failed" };
    const initialReads = chat.read.mock.calls.length;
    chat.write.mockReturnValueOnce(editing.promise);
    chat.read.mockReturnValueOnce(history.promise);
    await act(async () => chat.action("save")!.click());
    expect(chat.action("save")?.disabled).toBe(true);
    expect(chat.read).toHaveBeenCalledTimes(initialReads);

    await act(async () => editing.reject(writeError));

    expect(chat.read).toHaveBeenCalledTimes(initialReads + 1);
    expect(chat.container.querySelector('[role="alert"]')?.textContent).toBe(writeError.message);
    expect(chat.snapshot.messages[0].content).toBe("Original message");
    expect(chat.snapshot.conversation_history_revision).toBe(1);
    expect(chat.editor()?.value).toBe("Draft to keep");
    expect(chat.editor()?.readOnly).toBe(false);
    expect(chat.action("save")?.disabled).toBe(false);
    expect(chat.action("cancel")?.disabled).toBe(false);
    expect(chat.action("retry_refresh")).toBeNull();

    chat.read.mockReturnValueOnce(retryHistory.promise);
    await act(async () => chat.action("save")!.click());
    expect(chat.read).toHaveBeenCalledTimes(initialReads + 2);
    expect(chat.write).toHaveBeenCalledTimes(2);
    expect(chat.write.mock.calls[1]).toEqual(chat.write.mock.calls[0]);
    expect(chat.snapshot.conversation_history_revision).toBe(2);
    expect(chat.editor()).toBeNull();
    expect(chat.row().querySelector(".whitespace-pre-wrap")?.textContent).toBe("Draft to keep");

    await act(async () => history.resolve(staleHistory));
    expect(chat.row().querySelector(".whitespace-pre-wrap")?.textContent).toBe("Draft to keep");
    expect(chat.container.querySelector('[role="alert"]')).toBeNull();

    await act(async () => retryHistory.resolve(structuredClone(chat.snapshot)));
    expect(chatApplication.getSnapshot().conversation_history_revision).toBe(2);
  } finally {
    await act(async () => {
      history.resolve(staleHistory);
      retryHistory.resolve(structuredClone(chat.snapshot));
    });
    await chat.unmount();
  }
});

it("does not show the previous message's edit outcome after switching conversations", async () => {
  const chat = await mountChat();
  const editing = deferred<EditConversationMessageResponse>();
  try {
    await act(async () => chat.action("edit")!.click());
    await chat.change("Saved in conversation A");
    chat.write.mockReturnValueOnce(editing.promise);
    await act(async () => chat.action("save")!.click());

    // Simulate selection changing from another window while the edit response is in transit.
    chat.snapshot.conversation_id = "conversation-b";
    chat.snapshot.generation++;
    chat.snapshot.messages[0].content = "Message in conversation B";
    await act(async () => { await chatApplication.refresh(); });
    chat.read.mockRejectedValueOnce(readError);
    await act(async () => editing.resolve({ message_id: 101, updated_content: "Saved in conversation A" }));

    expect(chat.row().querySelector(".whitespace-pre-wrap")?.textContent).toBe("Message in conversation B");
    expect(chat.container.querySelector('[role="alert"]')).toBeNull();
    expect(chat.action("retry_refresh")).toBeNull();
    expect(chat.action("edit")).not.toBeNull();
  } finally {
    editing.resolve({ message_id: 101, updated_content: "Saved in conversation A" });
    await chat.unmount();
  }
});

it.each([
  { order: "completion before the saved row", hasSavedReply: false },
  { order: "completion after the saved row", hasSavedReply: true },
])("enables edit after history becomes authoritative for $order", async ({ hasSavedReply }) => {
  const requestId = "completed-reply";
  const savedReply = {
    id: 102, role: "assistant", content: "Reply from history", created_at: "2026-11-18T00:00:00Z",
    metadata: JSON.stringify({ turn_id: `turn_${requestId}` }),
  };
  const prompt = { id: 101, role: "user", content: "Question", created_at: savedReply.created_at, metadata: null };
  const chat = await mountChat({ messages: hasSavedReply ? [prompt, savedReply] : [prompt] }, savedReply.id);
  const completionHistory = deferred<ChatViewSnapshot>();
  const editHistory = deferred<ChatViewSnapshot>();
  const bubbleText = () => chat.row().querySelector(".whitespace-pre-wrap")?.textContent;

  try {
    if (hasSavedReply) {
      expect(bubbleText()).toBe(savedReply.content);
      expect(chat.action("edit")).not.toBeNull();
    } else {
      expect(chat.row()).toBeNull();
    }

    const initialReads = chat.read.mock.calls.length;
    const revision = chat.snapshot.revision + 1;
    const completed = {
      ...turn(requestId, revision, "completed"),
      completion_revision: revision, user_message_id: prompt.id, assistant_message_id: savedReply.id,
      conversation_generation: chat.snapshot.generation, text: "Reply from completion",
    };
    chat.read.mockReturnValueOnce(completionHistory.promise);
    await act(async () => chat.complete(completed));

    expect(chat.read).toHaveBeenCalledTimes(initialReads + 1);
    expect(bubbleText()).toBe(completed.text);
    expect(chat.action("edit")).toBeNull();
    expect(chat.editor()).toBeNull();
    expect(chat.write).not.toHaveBeenCalled();

    chat.snapshot.revision = revision;
    chat.snapshot.turns = [completed];
    chat.snapshot.messages = [prompt, savedReply];
    if (!hasSavedReply) {
      chat.snapshot.history_revision++;
      chat.snapshot.conversation_history_revision = chat.snapshot.history_revision;
    }
    await act(async () => completionHistory.resolve(structuredClone(chat.snapshot)));
    expect(bubbleText()).toBe("Reply from history");

    await act(async () => chat.action("edit")!.click());
    expect(chat.editor()?.value).toBe("Reply from history");
    await chat.change("Edited reply");
    chat.read.mockReturnValueOnce(editHistory.promise);
    await act(async () => chat.action("save")!.click());

    expect(chat.read).toHaveBeenCalledTimes(initialReads + 2);
    expect(chat.write).toHaveBeenCalledExactlyOnceWith({
      conversation_id: "conversation-a", conversation_generation: 1, message_id: savedReply.id, new_content: "Edited reply",
    });
    expect(chat.editor()).toBeNull();
    expect(bubbleText()).toBe("Edited reply");
    expect(chat.action("retry_refresh")).toBeNull();

    await act(async () => editHistory.reject(readError));
    expect(bubbleText()).toBe("Edited reply");
    expect(chat.row().querySelector('[role="alert"]')?.textContent).toBe(notice);

    const callsBeforeRetry = vi.mocked(invoke).mock.calls.length;
    await act(async () => chat.action("retry_refresh")!.click());
    expect(vi.mocked(invoke).mock.calls.slice(callsBeforeRetry).map(([command]) => command)).toEqual(["get_chat_view"]);
    expect(chat.read).toHaveBeenCalledTimes(initialReads + 3);
    expect(chat.write).toHaveBeenCalledTimes(1);
    expect(bubbleText()).toBe("Edited reply");
    expect(chat.row().querySelector('[role="alert"]')).toBeNull();
    expect(chat.action("edit")).not.toBeNull();
  } finally {
    await act(async () => {
      completionHistory.resolve(structuredClone(chat.snapshot));
      editHistory.resolve(structuredClone(chat.snapshot));
    });
    await chat.unmount();
  }
});
