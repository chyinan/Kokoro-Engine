// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { chatApplication, emptyChatView } from "@/lib/chat/application";
import { deferred, turn } from "@/lib/chat/__tests__/harness";
import type { ChatRequest, ChatTurnSnapshot, ChatViewSnapshot } from "@/lib/kokoro-bridge";
import ChatPanel from "../ChatPanel";

vi.mock("@tauri-apps/api/core", async original => ({ ...await original<typeof import("@tauri-apps/api/core")>(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => {
  vi.mocked(invoke).mockReset();
  localStorage.clear();
  sessionStorage.clear();
});

const commandFailure = { code: "Chat", message: "Model request failed" };
const savedFailure = "Saved model error details";
let revision = 0;

function setup(send: (request: ChatRequest) => Promise<unknown>, conversationId: string | null = "conversation-a") {
  const initial = {
    ...emptyChatView, revision: ++revision, character_id: "character-a", conversation_id: conversationId,
    generation: revision, conversation_history_revision: conversationId === null ? null : 0,
  };
  let view: ChatViewSnapshot = initial;
  let read = () => Promise.resolve(view);
  const host = vi.mocked(invoke).mockImplementation(async (command, args) => {
    switch (command) {
      case "get_chat_view": return read();
      case "stream_chat": return send((args as { request: ChatRequest }).request);
      case "recover_chat_draft_submission": return "unsent";
    }
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  return {
    mount: () => act(async () => root.render(<ChatPanel />)),
    async close() {
      await act(async () => root.unmount());
      container.remove();
    },
    submit: async () => {
      const input = container.querySelector("textarea")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Question");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => { input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
      const call = host.mock.calls.find(([command]) => command === "stream_chat");
      const request = (call?.[1] as { request: ChatRequest }).request;
      expect(request.client_request_id).toBeDefined();
      return request.client_request_id!;
    },
    holdRead: (pending: Promise<ChatViewSnapshot>) => { read = () => pending; },
    show: (next: ChatViewSnapshot) => {
      view = next;
      read = () => Promise.resolve(view);
    },
    initial,
    container,
    historyReads: () => host.mock.calls.filter(([command]) => command === "get_chat_view").length,
    sendButton: () => container.querySelector<HTMLButtonElement>('button[aria-label="Send message"]')!,
    alertNodes: () => [...container.querySelectorAll('[role="alert"]')],
    alerts: () => [...container.querySelectorAll('[role="alert"]')].map(node => node.textContent),
  };
}

function failure(requestId: string, id: number, status: ChatTurnSnapshot["status"] = "error") {
  const failed = { ...turn(requestId, ++revision, status), user_message_id: id - 1, error: savedFailure };
  return {
    failed,
    row: {
      id, role: "system", content: savedFailure, created_at: "now",
      metadata: JSON.stringify({ type: "failure_event", user_message_id: id - 1,
        event: { character_id: failed.character_id, conversation_id: failed.conversation_id, turn_id: failed.turn_id } }),
    },
  };
}

it.each([false, true])("shows a direct send failure during a pending history read (intervening stale view: %s)", async staleFirst => {
  const pending = deferred<ChatViewSnapshot>();
  const h = setup(async () => {
    h.holdRead(pending.promise);
    throw commandFailure;
  });
  try {
    await h.mount();
    const readsBeforeSend = h.historyReads();
    const requestId = await h.submit();
    expect(h.historyReads()).toBeGreaterThan(readsBeforeSend);
    expect(h.sendButton().disabled).toBe(false);
    expect(h.alerts()).toEqual([commandFailure.message]);

    const matching = failure(requestId, 2);
    if (staleFirst) {
      await act(async () => { pending.resolve(h.initial); });
      expect(h.alerts()).toEqual([commandFailure.message]);
    }
    const latest = {
      ...h.initial, revision, history_revision: 2, messages: [matching.row], turns: [matching.failed],
    };
    await act(async () => {
      h.show(latest);
      if (staleFirst) await chatApplication.refresh();
      else pending.resolve(latest);
    });
    expect(h.alerts()).toEqual([]);
    expect(h.container.textContent).not.toContain(commandFailure.message);
    expect(h.container.textContent?.split(savedFailure)).toHaveLength(2);
  } finally {
    pending.resolve(h.initial);
    await h.close();
  }
});

it("announces an unrelated turn independently and keeps the current request's toast visible", async () => {
  const h = setup(async () => { throw commandFailure; });
  try {
    await h.mount();
    await h.submit();
    const separate = failure("separate-request", 4);
    const inOtherConversation = failure("wrong-conversation", 6);
    const elsewhere = JSON.parse(inOtherConversation.row.metadata);
    elsewhere.event.conversation_id = "conversation-b";
    await act(async () => {
      h.show({ ...h.initial, revision, history_revision: 2,
        messages: [
          { ...separate.row, content: commandFailure.message },
          { ...inOtherConversation.row, content: "Other conversation error", metadata: JSON.stringify(elsewhere) },
        ],
        turns: [separate.failed, { ...inOtherConversation.failed, conversation_id: "conversation-b" }],
      });
      await chatApplication.refresh();
    });
    expect(h.alerts()).toEqual([commandFailure.message, commandFailure.message]);
    expect(h.container.textContent).not.toContain("Other conversation error");
  } finally {
    await h.close();
  }
});

it.each([true, false])("scopes the first send error when an empty selection opens a conversation (owns new conversation: %s)", async ownsConversation => {
  const pending = deferred<ChatViewSnapshot>();
  const h = setup(async () => {
    h.holdRead(pending.promise);
    throw commandFailure;
  }, null);
  try {
    await h.mount();
    const requestId = await h.submit();
    const [alert] = h.alertNodes();
    expect(alert?.textContent).toBe(commandFailure.message);

    const generation = h.initial.generation + 1;
    const failed = {
      ...turn(ownsConversation ? requestId : "another-request", ++revision, "error"),
      conversation_generation: generation, error: commandFailure.message,
    };
    const next = {
      ...h.initial, revision, generation, conversation_id: failed.conversation_id,
      history_revision: 2, conversation_history_revision: 1, turns: [failed],
      messages: [{ id: 1, role: "user", content: "Question", created_at: "now", metadata: null }],
    };
    await act(async () => {
      h.show(next);
      pending.resolve(next);
    });
    const alerts = h.alertNodes();
    expect(alerts).toHaveLength(ownsConversation ? 1 : 0);
    if (ownsConversation) expect(alerts[0]).toBe(alert);
  } finally {
    pending.resolve(h.initial);
    await h.close();
  }
});

it("keeps an already visible failure as the only error when its command rejects", async () => {
  const sending = deferred<unknown>();
  const h = setup(() => sending.promise);
  try {
    await h.mount();
    const requestId = await h.submit();
    // The diagnostic write can appear in history before the journal observes the command error.
    const matching = failure(requestId, 2, "streaming");
    await act(async () => {
      h.show({ ...h.initial, revision, history_revision: 2, messages: [matching.row], turns: [matching.failed] });
      await chatApplication.refresh();
    });
    expect(h.alerts()).toEqual([savedFailure]);
    await act(async () => { sending.reject(commandFailure); });
    expect(h.alerts()).toEqual([savedFailure]);
    expect(h.container.textContent).not.toContain(commandFailure.message);
  } finally {
    sending.resolve(undefined);
    await h.close();
  }
});
