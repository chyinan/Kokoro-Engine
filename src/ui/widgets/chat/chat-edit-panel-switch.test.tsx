// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { listen, type EventCallback } from "@tauri-apps/api/event";
import ChatPanel from "../ChatPanel";
import { chatViewFixture as chat } from "../../../lib/chat/view-test-fixture";
import { deferred, turn } from "../../../lib/chat/__tests__/harness";
import type { StreamChatResponse } from "../../../lib/kokoro-bridge";

vi.mock("../../../lib/chat/application", async () => ({ chatApplication: (await import("../../../lib/chat/view-test-fixture")).chatViewFixture }));
vi.mock("@tauri-apps/api/core", async original => ({ ...await original<typeof import("@tauri-apps/api/core")>(), invoke: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("shows a transient backend warning without replacing the completed text reply", async () => {
    chat.reset();
    chat.set({ messages: [{ id: 1, role: "assistant", content: "Completed answer", created_at: "now" }] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const unlisten = vi.fn();
    let onWarning: EventCallback<unknown> | undefined;
    const mockListen = vi.mocked(listen).mockImplementation(async (event, handler) => {
        if (event === "chat-warning") {
            onWarning = handler;
            return unlisten;
        }
        return () => {};
    });
    try {
        await act(async () => root.render(<ChatPanel />));
        expect(onWarning).toBeDefined();
        await act(async () => onWarning!({ event: "chat-warning", id: 1, payload: "Speech provider unavailable" }));
        expect(container.querySelector('[role="alert"]')?.textContent).toBe("Speech provider unavailable");
        expect(container.textContent).toContain("Completed answer");
    } finally {
        await act(async () => root.unmount());
        mockListen.mockImplementation(async () => () => {});
        container.remove();
    }
    expect(unlisten).toHaveBeenCalledOnce();
});

it("restores a speech failure on its saved reply after reopening and never recreates a removed reply", async () => {
    chat.reset();
    const message = { id: 2, role: "assistant", content: "Completed answer", created_at: "now" };
    const completed = {
        ...turn("speech", 2, "completed"), character_id: "char-1", conversation_id: "conv-1",
        assistant_message_id: message.id, completion_revision: 2,
    };
    chat.set({ revision: 2, messages: [message], turns: [completed] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const warning = () => container.querySelector('[role="alert"]');
    try {
        await act(async () => root.render(<ChatPanel />));
        expect(warning()).toBeNull();
        await act(async () => root.render(null));
        expect(container.textContent).toBe("");

        const failedSpeech = { ...completed, revision: 3, speech_error: "Speech provider unavailable" };
        chat.set({ turns: [failedSpeech] });
        await act(async () => root.render(<ChatPanel />));
        expect(warning()?.textContent).toBe("chat.speech_failed");
        expect(container.textContent).toContain("Completed answer");

        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2, messages: [{ ...message, id: 3, content: "Other answer" }] }));
        expect(warning()).toBeNull();
        expect(container.textContent).toContain("Other answer");

        await act(async () => chat.set({ conversation_id: "conv-1", generation: 3, messages: [] }));
        expect(warning()).toBeNull();
        expect(container.textContent).not.toContain("answer");
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});

it("shows the legacy Mod failure once across the journal and history, scoped to its conversation and character", async () => {
    chat.reset();
    const prompt = { id: 1, role: "user", content: "Mod prompt", created_at: "now" };
    const failed = {
        ...turn("mod_connection", 3, "error"), character_id: "char-1", conversation_id: "conv-1",
        user_message_id: prompt.id, error: "Model connection failed", completion_revision: 3,
    };
    const diagnostic = (id: number, event: object) => ({
        id, role: "system", content: failed.error, created_at: "now", metadata: JSON.stringify({ type: "failure_event", event }),
    });
    const saved = diagnostic(2, { turn_id: failed.turn_id, character_id: failed.character_id, conversation_id: failed.conversation_id });
    chat.set({ revision: 2, messages: [prompt] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const reasons = () => [...container.querySelectorAll('[role="alert"]')].map(node => node.textContent);
    try {
        await act(async () => root.render(<ChatPanel />));
        expect(reasons()).toEqual([]);
        await act(async () => chat.set({ turns: [failed] }));
        expect(reasons()).toEqual([failed.error]);
        expect(container.querySelector('[title="chat.actions.retry"]')).toBeNull();

        await act(async () => chat.set({ revision: 3, messages: [prompt, saved] }));
        expect(reasons()).toEqual([failed.error]);
        expect(container.textContent?.match(/Model connection failed/g)).toHaveLength(1);
        const diagnosticBubble = container.querySelector('[role="alert"]')!.parentElement!;
        const retry = diagnosticBubble.querySelector<HTMLButtonElement>('[title="chat.actions.retry"]');
        expect(retry).not.toBeNull();
        expect(diagnosticBubble.querySelector('[title="chat.actions.edit"], [title="chat.actions.continue_from"]')).toBeNull();
        await act(async () => retry!.click());
        expect(chat.continueFrom).toHaveBeenCalledExactlyOnceWith({
            conversationId: "conv-1", generation: 1, messageId: saved.id, historyRevision: 0,
        }, true, expect.anything());

        // Neither a delayed turn snapshot nor a diagnostic filed under the
        // wrong selection should appear in another conversation or character.
        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2 }));
        expect(reasons()).toEqual([]);
        await act(async () => chat.set({ character_id: "char-2", conversation_id: "conv-1", generation: 3 }));
        expect(reasons()).toEqual([]);

        await act(async () => chat.set({ character_id: "char-1", generation: 4, messages: [] }));
        expect(reasons()).toEqual([]);
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});

it.each([false, true])("announces a composer failure once when history arrives first: %s", async historyFirst => {
    chat.reset();
    const sending = deferred<StreamChatResponse>();
    chat.send.mockReturnValueOnce(sending.promise);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const prompt = { id: 1, role: "user", content: "Prompt that failed", created_at: "now" };
    const alertNodes = () => [...container.querySelectorAll('[role="alert"]')];
    try {
        await act(async () => root.render(<ChatPanel />));
        const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
        await act(async () => {
            Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, prompt.content);
            input.dispatchEvent(new Event("input", { bubbles: true }));
        });
        await act(async () => { input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
        const requestId = chat.send.mock.calls[0]?.[0].client_request_id;
        expect(requestId).toBeDefined();
        const failed = { ...turn(requestId!, 3, "error"), character_id: "char-1", conversation_id: "conv-1", error: "Wrapped provider error" };
        const diagnostic = {
            id: 2, role: "system", content: "Saved provider error", created_at: "now",
            metadata: JSON.stringify({ type: "failure_event", user_message_id: prompt.id,
                event: { character_id: failed.character_id, conversation_id: failed.conversation_id, turn_id: failed.turn_id } }),
        };
        const save = () => chat.set({ revision: 3, messages: [prompt, diagnostic], turns: [failed] });
        const reject = () => sending.reject(new Error(failed.error));
        await act(async () => { (historyFirst ? save : reject)(); });
        const [firstAlert] = alertNodes();
        expect(firstAlert?.textContent).toBe(historyFirst ? diagnostic.content : failed.error);
        await act(async () => { (historyFirst ? reject : save)(); });
        expect(alertNodes()).toEqual([firstAlert]);
        expect(container.textContent).toContain(diagnostic.content);
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});

it.each(["other conversation", "same conversation reselected"])("discards a late composer rejection after switching to %s", async selection => {
    chat.reset();
    const sending = deferred<StreamChatResponse>();
    chat.send.mockReturnValueOnce(sending.promise);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
        await act(async () => root.render(<ChatPanel />));
        const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
        await act(async () => {
            Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Old prompt");
            input.dispatchEvent(new Event("input", { bubbles: true }));
        });
        await act(async () => { input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
        expect(chat.send).toHaveBeenCalledOnce();
        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2 }));
        if (selection === "same conversation reselected") {
            await act(async () => chat.set({ conversation_id: "conv-1", generation: 3 }));
        }
        await act(async () => { sending.reject(new Error("Old conversation failed")); });
        expect(container.querySelector('[role="alert"]')).toBeNull();
        expect(container.textContent).not.toContain("Old conversation failed");
    } finally {
        await act(async () => root.unmount());
        localStorage.removeItem("kokoro_chat_submission_char-1");
        sessionStorage.removeItem("kokoro_chat_submission_char-1");
        container.remove();
    }
});

it("dismisses the old editor when another conversation replaces a message at the same position", async () => {
    chat.reset();
    const first = { id: 101, role: "user", content: "Message A", created_at: "now" };
    const second = { ...first, id: 201, content: "Message B" };
    chat.set({ messages: [first] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const button = (action: string) => container.querySelector<HTMLButtonElement>(`button[title="chat.actions.${action}"]`);
    try {
        await act(async () => root.render(<ChatPanel />));
        await act(async () => button("edit")!.click());
        expect(button("save")).not.toBeNull();
        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2, messages: [second] }));

        expect(button("save")).toBeNull();
        expect(container.textContent).toContain("Message B");
        expect(chat.edit).not.toHaveBeenCalled();

        await act(async () => button("edit")!.click());
        await act(async () => button("save")!.click());
        expect(chat.edit).toHaveBeenCalledExactlyOnceWith({ conversationId: "conv-2", generation: 2, messageId: 201 }, "Message B");
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});

it("dismisses an assistant editor when the same row is reloaded under a new generation", async () => {
    chat.reset();
    chat.set({ messages: [{ id: 101, role: "assistant", content: "Original answer", created_at: "now", metadata: '{"turn_id":"turn_a"}' }] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const button = (action: string) => container.querySelector<HTMLButtonElement>(`button[title="chat.actions.${action}"]`);
    try {
        await act(async () => root.render(<ChatPanel />));
        await act(async () => button("edit")!.click());
        expect(button("save")).not.toBeNull();

        await act(async () => chat.set({ generation: 2 }));
        expect(button("save")).toBeNull();
        await act(async () => button("edit")!.click());
        await act(async () => button("save")!.click());
        expect(chat.edit).toHaveBeenCalledExactlyOnceWith({ conversationId: "conv-1", generation: 2, messageId: 101 }, "Original answer");
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});

it.each(["native", "text"])("keeps %s tool details open through the streaming draft and saved history", async mode => {
    chat.reset();
    const snapshot = {
        ...turn("search", 2), character_id: "char-1", conversation_id: "conv-1",
        tools: [{ turn_id: "turn_search", tool: "search", result: { message: "found result" }, tool_trace_id: "0:0" }],
    };
    const user = { id: 1, role: "user", content: "question", created_at: "now" };
    chat.set({ revision: 2, busy: true, messages: [user], turns: [snapshot] });
    const completed = { ...snapshot, revision: 3, status: "completed" as const, assistant_message_id: 3 };
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const details = () => [...container.querySelectorAll("button")].find(button => button.textContent?.includes("chat.tools.header"));
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
        await act(async () => root.render(<ChatPanel />));
        const toggle = details();
        expect(toggle).toBeDefined();
        await act(async () => toggle!.click());
        expect(container.textContent).toContain("found result");

        // SQLite can already have the first round's draft while the journal is still streaming.
        await act(async () => chat.set({ messages: [user, {
            id: 3, role: "assistant", content: "first-round draft", created_at: "now",
            metadata: JSON.stringify({ turn_id: snapshot.turn_id, streaming_draft: true }),
        }] }));
        expect(container.textContent).not.toContain("first-round draft");
        expect(details()).toBe(toggle);
        expect(container.textContent).toContain("found result");

        // A terminal event updates the turn while the cached history is unchanged.
        await act(async () => chat.set({ busy: false, turns: [completed] }));
        expect(details()).toBe(toggle);
        expect(container.textContent).toContain("found result");
        const continueFrom = container.querySelector<HTMLButtonElement>('button[title="chat.actions.continue_from"]');
        await act(async () => continueFrom!.click());
        expect(chat.continueFrom).toHaveBeenCalledExactlyOnceWith({ conversationId: "conv-1", generation: 1, messageId: 1, historyRevision: 0 });

        const savedToolRows = mode === "native" ? [{
            id: 2, role: "tool", content: "saved result", created_at: "now",
            metadata: JSON.stringify({ turn_id: snapshot.turn_id, tool_name: "search", tool_trace_id: "0:0" }),
        }] : [];
        await act(async () => chat.set({
            revision: 3,
            messages: [user, ...savedToolRows, {
                id: 3, role: "assistant", content: "answer", created_at: "now",
                metadata: JSON.stringify({ turn_id: snapshot.turn_id }),
            }],
        }));
        expect(details()).toBe(toggle);
        expect(container.textContent).toContain(mode === "native" ? "saved result" : "found result");
        if (mode === "native") expect(container.textContent).not.toContain("found result");
    } finally {
        await act(async () => root.unmount());
        scrollTo.mockRestore();
        container.remove();
    }
});

it("keeps a text-mode tool-only reply expanded after history catches up", async () => {
    chat.reset();
    const user = { id: 1, role: "user", content: "question", created_at: "now" };
    const streaming = {
        ...turn("tool-only", 2), character_id: "char-1", conversation_id: "conv-1", text: "",
        tools: [{ turn_id: "turn_tool-only", tool: "lookup", result: { message: "Tool-only result" }, tool_trace_id: "0:0" }],
    };
    chat.set({ revision: 2, busy: true, messages: [user], turns: [streaming] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const details = () => [...container.querySelectorAll("button")].filter(button => button.textContent?.includes("chat.tools.header"));
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
        await act(async () => root.render(<ChatPanel />));
        expect(details()).toHaveLength(1);
        const [toggle] = details();
        await act(async () => toggle.click());
        expect(container.textContent).toContain("Tool-only result");

        const completed = { ...streaming, revision: 3, status: "completed" as const };
        await act(async () => chat.set({ busy: false, turns: [completed] }));
        expect(details()[0]).toBe(toggle);
        expect(container.textContent).toContain("Tool-only result");

        // The saved history has neither an assistant row nor a tool row for this turn.
        await act(async () => chat.set({ revision: 3, messages: [{ ...user }] }));
        expect(details()).toHaveLength(1);
        expect(details()[0]).toBe(toggle);
        expect(container.textContent).toContain("Tool-only result");

        const next = { id: 2, role: "user", content: "Next question", created_at: "now" };
        await act(async () => chat.set({ messages: [user, next] }));
        expect(details()).toHaveLength(1);
        expect(details()[0]).toBe(toggle);
        expect(container.textContent).toMatch(/Tool-only result[\s\S]*Next question/);
    } finally {
        await act(async () => root.unmount());
        scrollTo.mockRestore();
        container.remove();
    }
});

it("keeps approved tool details expanded when the first hidden turn creates its conversation", async () => {
    chat.reset();
    const request = {
        turn_id: "turn_hidden", tool: "lookup", tool_trace_id: "0:0", approval_request_id: "approval-1",
        approval_status: "requested" as const, deny_kind: "pending_approval" as const, error: "Waiting for approval",
    };
    const pending = {
        ...turn("hidden", 2), character_id: "char-1", conversation_id: null, hidden: true,
        user_message_id: null, text: "", tools: [request],
    };
    chat.set({ conversation_id: null, revision: 2, busy: true, turns: [pending] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const details = () => [...container.querySelectorAll("button")].filter(button => button.textContent?.includes("chat.tools.header"));
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
        await act(async () => root.render(<ChatPanel />));
        const [toggle] = details();
        await act(async () => toggle.click());
        expect(container.textContent).toContain("Waiting for approval");

        const approved = { ...request, approval_status: "approved" as const, error: undefined, deny_kind: undefined, result: { message: "Approved tool result" } };
        const resolved = { ...pending, revision: 3, tools: [request, approved] };
        await act(async () => chat.set({ turns: [resolved] }));
        expect(container.textContent).toContain("Approved tool result");

        const savedTool = {
            id: 7, role: "tool", content: "Approved tool result", created_at: "now",
            metadata: JSON.stringify({ turn_id: pending.turn_id, tool_name: "lookup", tool_trace_id: "0:0" }),
        };
        // SQLite publishes the new selection while the streaming turn is still unbound in the journal.
        await act(async () => chat.set({
            conversation_id: "new-conversation", generation: 2, revision: 3, messages: [savedTool],
        }));
        expect(details()).toHaveLength(1);
        expect(details()[0]).toBe(toggle);
        expect(container.textContent).toContain("Approved tool result");

        const completed = { ...resolved, conversation_id: "new-conversation", status: "completed" as const, revision: 4, text: "Finished", assistant_message_id: 8 };
        await act(async () => chat.set({ turns: [completed], busy: false }));
        await act(async () => chat.set({ revision: 4, messages: [savedTool, {
            id: 8, role: "assistant", content: "Finished", created_at: "now",
            metadata: JSON.stringify({ turn_id: pending.turn_id }),
        }] }));
        expect(details()).toHaveLength(1);
        expect(details()[0]).toBe(toggle);
        expect(container.textContent).toContain("Approved tool result");
    } finally {
        await act(async () => root.unmount());
        scrollTo.mockRestore();
        container.remove();
    }
});
