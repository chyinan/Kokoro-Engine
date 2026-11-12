// @vitest-environment node
// Former Panel fallback/watchdog risks now live at the application boundary.
import { afterEach, describe, expect, it, vi } from "vitest";
import { harness, turn, deferred } from "../../../lib/chat/__tests__/harness";
import { projectMessages } from "../../../lib/chat/projection";
import type { ChatViewSnapshot } from "../../../lib/kokoro-bridge";
afterEach(() => vi.useRealTimers());

describe("application recovery when lifecycle notifications are lost", () => {
  it.each(["completed", "cancelled", "error"] as const)("recovers %s without start/finish and releases busy", async status => {
    vi.useFakeTimers(); const h = harness();
    h.set({ revision: 1, busy: true, turns: [turn("a", 1, "accepted")] });
    const stop = h.app.subscribe(() => {}); await vi.advanceTimersByTimeAsync(0);
    h.set({ revision: 2, busy: false, turns: [turn("a", 2, status)], messages: status === "completed" ? [{ id: 2, role: "assistant", content: "saved answer", created_at: "now" }] : [] });
    await vi.advanceTimersByTimeAsync(800);
    expect(h.app.getSnapshot().busy).toBe(false);
    expect(projectMessages(h.app.getSnapshot())).toHaveLength(status === "completed" ? 1 : 0);
    expect(h.io.send).not.toHaveBeenCalled(); stop();
  });
  it("recovers persisted user identity before chat-turn-start without an optimistic duplicate", async () => {
    const h = harness(); h.set({ revision: 2, busy: true, turns: [{ ...turn("preparing", 2, "accepted"), text: "" }], messages: [{ id: 1, role: "user", content: "question", created_at: "now" }] });
    await h.app.refresh(); expect(projectMessages(h.app.getSnapshot())).toHaveLength(1);
  });
  it("keeps context and tool evidence when the assistant shares their turn id", async () => {
    const h = harness(); h.set({ revision: 5, turns: [turn("a", 5)], messages: [
      { id: 10, role: "context", content: "camera evidence", metadata: '{"turn_id":"turn_a"}', created_at: "now" },
      { id: 11, role: "assistant", content: "draft", metadata: '{"turn_id":"turn_a"}', created_at: "now" },
      { id: 12, role: "tool", content: "clock result", metadata: '{"turn_id":"turn_a","tool_name":"get_time"}', created_at: "now" },
    ] });
    await h.app.refresh(); const messages = projectMessages(h.app.getSnapshot());
    expect(messages.find(message => message.role === "context")?.text).toBe("camera evidence");
    expect(messages.filter(message => message.role === "kokoro")).toHaveLength(1);
    expect(messages.find(message => message.role === "kokoro")).toMatchObject({ text: "answer", tools: [{ text: "clock result" }] });
  });
  it("coalesces overlapping reads and does not query the history for every text delta", async () => {
    const h = harness(); h.set({ revision: 1, turns: [turn("a", 1)], busy: true });
    const stop = h.app.subscribe(() => {}); await h.app.refresh(); vi.mocked(h.io.read).mockClear();
    for (let revision = 2; revision < 20; revision++) h.emit({ ...turn("a", revision), text: `delta ${revision}` });
    expect(h.io.read).not.toHaveBeenCalled();
    const read = deferred<ChatViewSnapshot>(); vi.mocked(h.io.read).mockReturnValueOnce(read.promise);
    const a = h.app.refresh(), b = h.app.refresh(); expect(a).toBe(b); expect(h.io.read).toHaveBeenCalledOnce();
    read.resolve({ ...h.app.getSnapshot(), revision: 20 }); await a; stop();
  });
  it("a read from the detached subscriber cannot overwrite its reconnected selection", async () => {
    const h = harness(); const stale = deferred<ChatViewSnapshot>();
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    vi.mocked(h.io.read).mockReturnValueOnce(stale.promise); const old = h.app.refresh(); stop();
    h.set({ character_id: "character-b", conversation_id: "conversation-b", generation: 9 });
    const stopNew = h.app.subscribe(() => {}); await h.app.refresh();
    stale.resolve({ ...h.app.getSnapshot(), character_id: "character-a", conversation_id: "conversation-a" }); await old;
    expect(h.app.getSnapshot().character_id).toBe("character-b"); stopNew();
  });
  it("late finish after cancellation cannot restore partial text removed from history", async () => {
    const h = harness(); h.set({ revision: 3, busy: false, turns: [turn("a", 3, "cancelled")], messages: [] });
    const stop = h.app.subscribe(() => {}); await h.app.refresh(); h.emit(turn("a", 2, "completed"));
    expect(projectMessages(h.app.getSnapshot())).toEqual([]); expect(h.app.getSnapshot().busy).toBe(false); stop();
  });
});