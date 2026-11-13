// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { ChatApplication, emptyChatView, type ChatTransport } from "./application";
import type { ChatTurnSnapshot, ChatViewSnapshot, ChatViewVersion, StreamChatResponse } from "../kokoro-bridge";
import { projectMessages } from "./projection";
import { canSubmitApproval } from "./turn-state";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function turn(id: string, revision: number, status: ChatTurnSnapshot["status"] = "streaming"): ChatTurnSnapshot {
  return { revision, client_request_id: id, turn_id: `turn_${id}`, character_id: "character-a", conversation_id: "conversation-a", user_message_id: 1, assistant_message_id: 2, message: "question", images: [], hidden: false, status, text: "answer", translation: null, tools: [], error: null };
}
function harness() {
  let view: ChatViewSnapshot = { ...emptyChatView, character_id: "character-a", conversation_id: "conversation-a", generation: 1 };
  let callback: (turn: ChatTurnSnapshot) => void = () => {};
  const io: ChatTransport = {
    read: vi.fn(async (known?: ChatViewVersion) => {
      const { revision, history_revision, generation, busy } = view;
      if (known?.revision === revision && known.history_revision === history_revision && known.generation === generation && known.busy === busy) return null;
      return structuredClone(view);
    }),
    listen: vi.fn(async listener => { callback = listener; return vi.fn(); }),
    send: vi.fn(async request => ({ status: "completed", conversation_id: "conversation-a", client_request_id: request.client_request_id, text: "answer" })),
    cancel: vi.fn(async () => {}),
  };
  const app = new ChatApplication(io);
  return { app, io, emit: (snapshot: ChatTurnSnapshot) => callback(snapshot), set: (next: Partial<ChatViewSnapshot>) => { view = { ...view, ...next }; } };
}
afterEach(() => vi.useRealTimers());

describe("UI-independent chat application", () => {
  it("submits and obtains the authoritative result without any subscription or React component", async () => {
    const { app, io } = harness();
    const result = await app.send({ message: "question", client_request_id: "pet_1" });
    expect(result.text).toBe("answer");
    expect(io.listen).not.toHaveBeenCalled();
    expect(io.send).toHaveBeenCalledOnce();
  });
  it("an event before command resolution does not append the persisted assistant twice", async () => {
    const h = harness();
    const response = deferred<StreamChatResponse>();
    vi.mocked(h.io.send).mockReturnValue(response.promise);
    const stop = h.app.subscribe(() => {});
    await h.app.refresh();
    const result = h.app.send({ message: "question", client_request_id: "a" });
    h.set({ revision: 4, busy: false, turns: [turn("a", 4, "completed")], messages: [
      { id: 1, role: "user", content: "question", created_at: "now" },
      { id: 2, role: "assistant", content: "answer", created_at: "now" },
    ] });
    h.emit(turn("a", 4, "completed"));
    response.resolve({ status: "completed", conversation_id: "conversation-a", text: "answer" });
    await result; await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot()).filter(message => message.role === "kokoro")).toHaveLength(1);
    expect(h.app.getSnapshot().busy).toBe(false);
    stop();
  });
  it("recovers lost finish for a passive subscriber by reading, without a command promise", async () => {
    vi.useFakeTimers();
    const h = harness();
    h.set({ revision: 2, busy: true, turns: [turn("other_window", 2)] });
    const stop = h.app.subscribe(() => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(h.app.getSnapshot().busy).toBe(true);
    h.set({ revision: 3, busy: false, turns: [turn("other_window", 3, "completed")] });
    await vi.advanceTimersByTimeAsync(800);
    expect(h.app.getSnapshot().busy).toBe(false);
    expect(h.io.send).not.toHaveBeenCalled();
    stop();
  });
  it("keeps the same view through idle polls and reloads history after an edit with no journal event", async () => {
    vi.useFakeTimers();
    const h = harness();
    const message = { id: 1, role: "user", content: "original", created_at: "now" };
    h.set({ revision: 2, history_revision: 4, messages: [message] });
    const publish = vi.fn();
    const stop = h.app.subscribe(publish);
    try {
      await vi.advanceTimersByTimeAsync(0);
      const restored = h.app.getSnapshot();
      await vi.advanceTimersByTimeAsync(2250);
      expect(h.io.read).toHaveBeenCalledTimes(4);
      expect(h.io.read).toHaveBeenLastCalledWith({ revision: 2, history_revision: 4, generation: 1, busy: false });
      expect(h.app.getSnapshot()).toBe(restored);
      expect(publish).toHaveBeenCalledOnce();

      h.set({ history_revision: 5, messages: [{ ...message, content: "edited" }] });
      await vi.advanceTimersByTimeAsync(750);
      expect(h.app.getSnapshot().messages[0]?.content).toBe("edited");
      expect(publish).toHaveBeenCalledTimes(2);
    } finally { stop(); }
  });
  it("an explicit refresh does not wait on or accept a stale conditional poll", async () => {
    vi.useFakeTimers();
    const h = harness(); const stop = h.app.subscribe(() => {});
    try {
      await vi.advanceTimersByTimeAsync(0);
      const before = h.app.getSnapshot();
      const poll = deferred<ChatViewSnapshot | null>();
      vi.mocked(h.io.read).mockReturnValueOnce(poll.promise);
      await vi.advanceTimersByTimeAsync(750);
      h.set({ conversation_id: "conversation-b", generation: 2 });
      expect(await h.app.refresh()).toMatchObject({ conversation_id: "conversation-b", generation: 2 });
      poll.resolve(before);
      await vi.advanceTimersByTimeAsync(0);
      expect(h.app.getSnapshot().conversation_id).toBe("conversation-b");
    } finally { stop(); }
  });
  it("ignores duplicate terminal and late active events, including old requests after a newer request starts", async () => {
    const h = harness();
    h.set({ revision: 10, busy: true, turns: [turn("old", 6, "completed"), turn("new", 10)] });
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    h.emit(turn("old", 6, "completed")); h.emit(turn("old", 2));
    expect(h.app.getSnapshot().turns.find(t => t.client_request_id === "old")?.status).toBe("completed");
    expect(h.app.getSnapshot().turns.find(t => t.client_request_id === "new")?.status).toBe("streaming");
    expect(h.io.send).not.toHaveBeenCalled(); stop();
  });
  it("reconnect restores complete streaming text without replay or cancellation", async () => {
    const h = harness(); const stop = h.app.subscribe(() => {}); await h.app.refresh(); stop();
    h.set({ revision: 9, busy: true, turns: [{ ...turn("pet_a", 9), text: "already emitted while disconnected" }] });
    const stopAgain = h.app.subscribe(() => {}); await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot()).slice(-1)[0]?.text).toBe("already emitted while disconnected");
    expect(h.io.cancel).not.toHaveBeenCalled(); stopAgain();
  });
  it("keeps the attachment preview when an optimistic message is bound to a saved row", async () => {
    const h = harness(); const images = ["http://127.0.0.1:1234/vision/test.png"];
    h.set({ revision: 1, turns: [{ ...turn("image", 1), user_message_id: null, images }] });
    await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot())[0]).toMatchObject({ role: "user", images });
    h.set({ revision: 2, turns: [{ ...turn("image", 2), images }], messages: [{ id: 1, role: "user", content: "saved question", created_at: "now", metadata: JSON.stringify({ client_request_id: "image" }) }] });
    await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot()).filter(message => message.role === "user")).toEqual([
      { id: 1, role: "user", text: "saved question", images },
    ]);
  });
  it("shows each tool call once while its live result and saved row overlap", () => {
    const active = turn("a", 2);
    const result = { turn_id: "turn_a", tool: "lookup", result: { message: "found" } };
    active.tools = ["0:0", "1:0"].map(tool_trace_id => ({ ...result, tool_trace_id }));
    const rows = active.tools.map((event, index) => ({
      id: index + 10, role: "tool", content: "found", created_at: "now",
      metadata: JSON.stringify({
        type: "tool_result", turn_id: "turn_a", tool_name: "lookup",
        tool_call_id: "provider-call", tool_trace_id: event.tool_trace_id,
      }),
    }));
    const view = { ...emptyChatView, character_id: "character-a", conversation_id: "conversation-a" };
    const tools = (messages: ChatViewSnapshot["messages"], current = active) =>
      projectMessages({ ...view, messages, turns: [current] }).find(message => message.role === "kokoro")?.tools;
    const expected = ["0:0", "1:0"].map(toolTraceId => ({ toolTraceId, tool: "lookup", text: "found" }));

    expect(tools(rows.slice(0, 1))).toMatchObject(expected);
    expect(tools(rows)).toMatchObject(expected);
    expect(tools([])).toMatchObject(expected);
    expect(tools(rows, { ...active, status: "completed" })).toMatchObject(expected);
    // A completed mixed turn keeps the text-mode result and uses the saved native result.
    expect(tools([{ ...rows[0], content: "saved result" }], { ...active, revision: 0, status: "completed" })).toMatchObject([
      { ...expected[0], text: "saved result" }, expected[1],
    ]);
  });
  it("restores an actionable tool approval from a snapshot before the result is saved", async () => {
    const h = harness();
    const requested = { turn_id: "turn_a", tool: "test_tool", tool_trace_id: "0:0", approval_request_id: "approval_1", approval_status: "requested" as const, deny_kind: "pending_approval" as const, error: "Waiting for approval" };
    const waiting = { ...turn("a", 1), text: "", tools: [requested] };
    h.set({ revision: 1, busy: true, turns: [waiting] });
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    const tools = () => projectMessages(h.app.getSnapshot()).find(message => message.role === "kokoro")?.tools ?? [];
    expect(tools().filter(canSubmitApproval)).toMatchObject([{ approvalRequestId: "approval_1" }]);
    const resolved = { ...waiting, revision: 2, tools: [requested, { ...requested, approval_status: "approved" as const, result: { message: "done" } }] };
    h.emit(resolved);
    expect(tools()).toMatchObject([{ approvalStatus: "approved", text: "done" }]);
    h.set({ revision: 2, turns: [resolved], messages: [{
      id: 3, role: "tool", content: "done", created_at: "now",
      metadata: JSON.stringify({ type: "tool_result", turn_id: "turn_a", tool_name: "test_tool", tool_trace_id: "0:0" }),
    }] });
    await h.app.refresh();
    expect(tools()).toMatchObject([{ approvalStatus: "approved", text: "done" }]);
    expect(tools().some(canSubmitApproval)).toBe(false);
    stop();
  });
  it("subscribes before snapshot and rejects a read overtaken by an event", async () => {
    const h = harness(); const first = deferred<ChatViewSnapshot>();
    vi.mocked(h.io.read).mockReturnValueOnce(first.promise);
    const stop = h.app.subscribe(() => {}); await Promise.resolve();
    expect(h.io.listen).toHaveBeenCalledBefore(vi.mocked(h.io.read));
    h.set({ revision: 5, turns: [turn("a", 5, "completed")], messages: [{ id: 2, role: "assistant", content: "committed", created_at: "now" }] });
    h.emit(turn("a", 5, "completed"));
    first.resolve({ ...emptyChatView, revision: 1, busy: true, turns: [turn("a", 1)] });
    await h.app.refresh();
    expect(h.app.getSnapshot().revision).toBe(5);
    expect(projectMessages(h.app.getSnapshot()).slice(-1)[0]?.text).toBe("committed"); stop();
  });
  it("publishes history and the newest streaming text without restarting a slow read for deltas", async () => {
    const h = harness();
    const streaming = turn("live", 2);
    h.set({ revision: 2, busy: true, turns: [streaming] });
    const stop = h.app.subscribe(() => {});
    try {
      const cached = await h.app.refresh();
      vi.mocked(h.io.read).mockClear();
      const history = deferred<ChatViewSnapshot>();
      vi.mocked(h.io.read).mockReturnValueOnce(history.promise);
      const refresh = h.app.refresh();
      h.emit({ ...streaming, revision: 3, text: "new text" });
      h.emit({ ...streaming, revision: 4, text: "newest text" });
      history.resolve({ ...cached, history_revision: 1, messages: [
        { id: 1, role: "user", content: "saved question", created_at: "now" },
      ] });

      const view = await refresh;
      expect(h.io.read).toHaveBeenCalledOnce();
      expect(view).toMatchObject({ revision: 2, history_revision: 1, busy: true });
      expect(projectMessages(view).map(message => message.text)).toEqual(["saved question", "newest text"]);
    } finally {
      stop();
    }
  });
  it("coalesces duplicate logical delivery but forwards conflicting payload to authority", async () => {
    const h = harness(); const pending = deferred<StreamChatResponse>();
    vi.mocked(h.io.send).mockReturnValueOnce(pending.promise).mockRejectedValueOnce(new Error("chat_request_conflict"));
    const request = { message: "one", client_request_id: "same" };
    const a = h.app.send(request); const b = h.app.send({ ...request });
    expect(a).toBe(b);
    await expect(h.app.send({ ...request, message: "different" })).rejects.toThrow("conflict");
    await h.app.cancel();
    expect(h.io.cancel).toHaveBeenCalledWith("same", "stopped_by_user");
    pending.resolve({ status: "cancelled", conversation_id: "conversation-a" }); await a;
    expect(h.io.send).toHaveBeenCalledTimes(2);
  });
  it("cancels immediately by request id before a turn id or event exists", async () => {
    const h = harness(); const pending = deferred<StreamChatResponse>();
    vi.mocked(h.io.send).mockReturnValue(pending.promise);
    const result = h.app.send({ message: "one", client_request_id: "before_ack" });
    await h.app.cancel();
    expect(h.io.cancel).toHaveBeenCalledWith("before_ack", "stopped_by_user");
    pending.resolve({ status: "cancelled", conversation_id: "" }); await result;
    expect(h.app.getSnapshot().busy).toBe(false);
  });
  it.each(["denied IPC", "connection lost", "serialization failed"])("releases provisional state after %s before journal registration", async error => {
    const h = harness(); vi.mocked(h.io.send).mockRejectedValueOnce(new Error(error));
    await expect(h.app.send({ message: "one" })).rejects.toThrow(error);
    await h.app.refresh();
    expect(h.app.getSnapshot().busy).toBe(false);
    expect(h.app.getSnapshot().turns).toHaveLength(0);
    await expect(h.app.send({ message: "retry" })).resolves.toMatchObject({ status: "completed" });
  });
  it.each(["completed", "cancelled", "error"] as const)("never resurrects deleted history from a late %s snapshot", async status => {
    const deleted = { ...turn("deleted", 30, status), assistant_message_id: null, text: "", tools: [{ turn_id: "turn_deleted", tool: "lookup", tool_trace_id: "0:0", result: { message: "found" } }] };
    const h = harness(); h.set({ revision: 30, turns: [deleted], messages: [] });
    await h.app.refresh(); expect(projectMessages(h.app.getSnapshot())).toEqual([]);
  });
  it("does not restore tool-only results once another turn has reused the saved prompt", () => {
    const old = { ...turn("old", 1, "completed"), assistant_message_id: null, text: "", tools: [{ turn_id: "turn_old", tool: "lookup", tool_trace_id: "0:0", result: { message: "old result" } }] };
    const newer = turn("new", 2, "completed");
    const user = { id: 1, role: "user", content: "question", created_at: "now" };
    const view = { ...emptyChatView, character_id: old.character_id!, conversation_id: old.conversation_id, revision: 2, messages: [user] };
    expect(projectMessages({ ...view, turns: [old] }).map(message => message.role)).toEqual(["user", "kokoro"]);
    expect(projectMessages({ ...view, turns: [newer, old] }).map(message => message.role)).toEqual(["user"]);
  });
  it("removes an old active cache entry if authoritative retention has expired", async () => {
    const h = harness(); h.set({ revision: 1, turns: [turn("old", 1)], busy: true });
    await h.app.refresh(); h.set({ revision: 999, turns: [turn("recent", 999, "completed")], busy: false });
    await h.app.refresh(); expect(h.app.getSnapshot().turns.some(t => t.client_request_id === "old")).toBe(false);
  });
  it("drops retained images after import invalidates the journal even when message IDs are reused", async () => {
    const h = harness();
    const old = { ...turn("device-a", 8, "completed"), images: ["device-a-image"] };
    h.set({ revision: 8, turns: [old], messages: [{ id: 1, role: "user", content: "device A", created_at: "now", metadata: JSON.stringify({ client_request_id: "device-a" }) }] });
    const stop = h.app.subscribe(() => {});
    try {
      await h.app.refresh();
      expect(projectMessages(h.app.getSnapshot())[0].images).toEqual(["device-a-image"]);
      h.set({ revision: 9, history_revision: 2, turns: [], messages: [{
        id: 1, role: "user", content: "device B", created_at: "now", metadata: JSON.stringify({ client_request_id: "device-b" }),
      }] });
      await h.app.refresh();
      h.emit(old); // A queued pre-import event cannot restore the retired attachment.
      expect(projectMessages(h.app.getSnapshot())).toEqual([{ id: 1, role: "user", text: "device B" }]);
    } finally { stop(); }
  });
  it.each(["resolve", "reject"] as const)("retires imported history before the sender's delayed IPC %s, without reviving its images", async outcome => {
    const h = harness();
    const response = deferred<StreamChatResponse>();
    vi.mocked(h.io.send).mockReturnValueOnce(response.promise);
    const stop = h.app.subscribe(() => {});
    await h.app.refresh();
    const sent = h.app.send({ message: "device A", client_request_id: "device-a", images: ["device-a-image"] });
    const old = { ...turn("device-a", 8, "completed"), images: ["device-a-image"] };
    try {
      h.set({ revision: 8, turns: [old], messages: [{ id: 1, role: "user", content: "device A", created_at: "now", metadata: JSON.stringify({ client_request_id: "device-a" }) }] });
      await h.app.refresh();
      expect(projectMessages(h.app.getSnapshot())[0].images).toEqual(["device-a-image"]);
      // The worker has released its database gate. Import and the new snapshot
      // can finish before the sender's IPC response reaches JavaScript.
      h.set({ revision: 9, history_revision: 2, turns: [], messages: [{ id: 1, role: "user", content: "device B", created_at: "now", metadata: JSON.stringify({ client_request_id: "device-b" }) }] });
      await h.app.refresh();
      expect(h.app.getSnapshot().turns).toEqual([]);
      const expected = [{ id: 1, role: "user", text: "device B" }];
      expect(projectMessages(h.app.getSnapshot())).toEqual(expected);
      h.emit({ ...old, status: "streaming", revision: 7 });
      h.emit(old);
      expect(projectMessages(h.app.getSnapshot())).toEqual(expected);
      if (outcome === "resolve") {
        response.resolve({ status: "completed", conversation_id: "conversation-a" });
        await sent;
      } else {
        response.reject(new Error("chat_history_replaced"));
        await expect(sent).rejects.toThrow("chat_history_replaced");
      }
      await h.app.refresh();
      h.emit(old);
      expect(h.app.getSnapshot().turns).toEqual([]);
      expect(projectMessages(h.app.getSnapshot())).toEqual(expected);
      expect(h.app.getSnapshot().busy).toBe(false);
      // A new request attached to the imported row still restores normally.
      h.set({ revision: 10, turns: [{ ...turn("device-b", 10, "completed"), images: ["device-b-image"] }] });
      await h.app.refresh();
      expect(projectMessages(h.app.getSnapshot())).toEqual([{ ...expected[0], images: ["device-b-image"] }]);
    } finally {
      response.resolve({ status: "completed", conversation_id: "conversation-a" });
      await sent.catch(() => {});
      stop();
    }
  });
  it("preserves a truly unregistered optimistic request across a newer empty journal snapshot", async () => {
    const h = harness(); const response = deferred<StreamChatResponse>();
    vi.mocked(h.io.send).mockReturnValueOnce(response.promise);
    await h.app.refresh();
    const sent = h.app.send({ message: "waiting for admission", client_request_id: "unregistered", images: ["draft-image"] });
    try {
      h.set({ revision: 9, turns: [] });
      await h.app.refresh();
      expect(h.app.getSnapshot().turns).toMatchObject([{ revision: 0, client_request_id: "unregistered", status: "pending" }]);
      expect(projectMessages(h.app.getSnapshot())).toEqual([{ role: "user", text: "waiting for admission", images: ["draft-image"], clientRequestId: "unregistered" }]);
    } finally {
      response.resolve({ status: "completed", conversation_id: "conversation-a" });
      await sent;
    }
  });
  it("preserves an event newer than an empty journal snapshot", async () => {
    const h = harness();
    const streaming = turn("newer", 8);
    h.set({ revision: 8, turns: [streaming], busy: true });
    const stop = h.app.subscribe(() => {});
    try {
      const cached = await h.app.refresh();
      const read = deferred<ChatViewSnapshot>();
      vi.mocked(h.io.read).mockReturnValueOnce(read.promise);
      const refresh = h.app.refresh();
      h.emit({ ...streaming, revision: 10, text: "newer event" });
      read.resolve({ ...cached, revision: 9, turns: [] });
      await refresh;
      expect(h.app.getSnapshot().turns).toMatchObject([{ revision: 10, text: "newer event" }]);
    } finally { stop(); }
  });
  it.each([undefined, "invalid json", "null", "{}", '{"client_request_id":1}', '{"client_request_id":"device-b"}'])("does not attach journal images without the saved request identity: %s", metadata => {
    const old = { ...turn("device-a", 8, "completed"), images: ["device-a-image"] };
    const view = { ...emptyChatView, revision: 8, character_id: "character-a", conversation_id: "conversation-a", turns: [old], messages: [{ id: 1, role: "user", content: "saved prompt", created_at: "now", metadata }] };
    expect(projectMessages(view)).toEqual([{ id: 1, role: "user", text: "saved prompt" }]);
  });
  it("restores images only from their saved request, including while a retry reuses that prompt", () => {
    const original = { ...turn("original", 8, "completed"), images: ["original-image"] };
    const retry = { ...turn("retry", 9, "completed"), reusing_saved_prompt: true, images: ["unrelated-image"] };
    const view = { ...emptyChatView, revision: 9, character_id: "character-a", conversation_id: "conversation-a", turns: [original, retry], messages: [{ id: 1, role: "user", content: "saved prompt", created_at: "now", metadata: JSON.stringify({ client_request_id: "original" }) }] };
    expect(projectMessages(view)).toEqual([{ id: 1, role: "user", text: "saved prompt", images: ["original-image"] }]);
  });
  it("does not project replies belonging to another character or conversation", async () => {
    const h = harness(); h.set({ revision: 2, turns: [ { ...turn("old-character", 1), character_id: "other" }, { ...turn("old-conversation", 2), conversation_id: "other" } ] });
    await h.app.refresh(); expect(projectMessages(h.app.getSnapshot())).toEqual([]);
  });
  it("two independent window projections neither execute requests nor synthesize speech on recovery", async () => {
    const a = harness(), b = harness();
    const view = { revision: 5, turns: [turn("shared", 5, "completed")], messages: [{ id: 2, role: "assistant", content: "once", created_at: "now" }] };
    a.set(view); b.set(view); await a.app.refresh(); await b.app.refresh();
    expect(projectMessages(a.app.getSnapshot())).toEqual(projectMessages(b.app.getSnapshot()));
    expect(a.io.send).not.toHaveBeenCalled(); expect(b.io.send).not.toHaveBeenCalled();
  });
});
