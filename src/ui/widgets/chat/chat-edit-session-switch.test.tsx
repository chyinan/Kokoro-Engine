// @vitest-environment node
// History ownership moved from Panel closures to captured application commands.
import { afterEach, describe, expect, it, vi } from "vitest";
import * as bridge from "../../../lib/kokoro-bridge";
import { harness, deferred } from "../../../lib/chat/__tests__/harness";
import { projectMessages } from "../../../lib/chat/projection";
const messages = [
  { id: 101, role: "user", content: "original A", created_at: "now" },
  { id: 102, role: "assistant", content: "answer A", created_at: "now" },
  { id: 103, role: "user", content: "later A", created_at: "now" },
];
afterEach(() => vi.restoreAllMocks());

describe("application history command session ownership", () => {
  it("cancels a true character switch by request id, but not a same-character runtime update", async () => {
    const h = harness(); const { turn } = await import("../../../lib/chat/__tests__/harness");
    h.set({ revision: 2, busy: true, turns: [turn("active", 2)] });
    await h.app.prepareCharacterSwitch("character-a"); expect(h.io.cancel).not.toHaveBeenCalled();
    await h.app.prepareCharacterSwitch("character-b"); expect(h.io.cancel).toHaveBeenCalledWith("active", "character_switched");
  });
  it("a delayed delete response cannot clear a newly selected conversation", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    const deletion = deferred<void>(); const remove = vi.spyOn(bridge, "deleteConversation").mockReturnValue(deletion.promise);
    const clear = vi.spyOn(bridge, "clearHistory").mockResolvedValue(undefined);
    const running = h.app.deleteConversation("conversation-a");
    h.set({ conversation_id: "conversation-b", generation: 4, messages: [{ id: 201, role: "user", content: "B", created_at: "now" }] });
    deletion.resolve(); await running;
    expect(remove).toHaveBeenCalledWith("conversation-a"); expect(clear).not.toHaveBeenCalled();
    expect(h.app.getSnapshot().conversation_id).toBe("conversation-b");
    expect(projectMessages(h.app.getSnapshot())).toMatchObject([{ text: "B" }]);
  });
  it("a delayed edit remains scoped to its original message and never overwrites another selection", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    const edit = deferred<bridge.EditConversationMessageResponse>();
    const invoke = vi.spyOn(bridge, "editConversationMessage").mockReturnValue(edit.promise);
    const running = h.app.edit(0, "edited A");
    expect(invoke).toHaveBeenCalledWith({ conversation_id: "conversation-a", message_id: 101, new_content: "edited A" });
    h.set({ conversation_id: "conversation-b", generation: 4, messages: [{ id: 201, role: "user", content: "original B", created_at: "now" }] });
    await h.app.refresh(); edit.resolve({ message_id: 101, updated_content: "edited A" }); await running;
    expect(projectMessages(h.app.getSnapshot())).toMatchObject([{ id: 201, text: "original B" }]);
  });
  it("clearing restores the authoritative empty selection before a new send", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    vi.spyOn(bridge, "clearHistory").mockImplementation(async () => { h.set({ conversation_id: null, generation: 4, messages: [] }); });
    await h.app.clear(); expect(projectMessages(h.app.getSnapshot())).toEqual([]);
    await h.app.send({ message: "new", conversation_id: h.app.getSnapshot().conversation_id ?? undefined });
    expect(h.io.send).toHaveBeenCalledWith(expect.objectContaining({ conversation_id: undefined }));
  });
  it("a failed clear preserves visible history and releases mutation busy state", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    vi.spyOn(bridge, "clearHistory").mockRejectedValue(new Error("database locked"));
    await expect(h.app.clear()).rejects.toThrow("database locked");
    expect(projectMessages(h.app.getSnapshot())).toHaveLength(3); expect(h.app.getSnapshot().busy).toBe(false);
  });
  it("blocks send and a second history mutation while continuation deletion is pending", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    const deletion = deferred<void>(); vi.spyOn(bridge, "deleteLastMessages").mockReturnValue(deletion.promise);
    const running = h.app.continueFrom(1);
    expect(h.app.getSnapshot().busy).toBe(true);
    await expect(h.app.send({ message: "overlap" })).rejects.toThrow("history_mutation_busy");
    await expect(h.app.continueFrom(1, true)).rejects.toThrow("history_mutation_busy");
    expect(h.io.send).not.toHaveBeenCalled();
    deletion.resolve(); await running; expect(h.app.getSnapshot().busy).toBe(false);
  });
  it("regenerates only after guarded deletion, preserving captured generation and image intent", async () => {
    const h = harness(); h.set({ messages: messages.slice(0, 2) }); await h.app.refresh();
    const deletion = deferred<void>(); const remove = vi.spyOn(bridge, "deleteLastMessages").mockReturnValue(deletion.promise);
    const running = h.app.continueFrom(1, true, { allow_image_gen: true });
    expect(remove).toHaveBeenCalledWith(1, "conversation-a", 102);
    expect(h.io.send).not.toHaveBeenCalled();
    h.set({ messages: messages.slice(0, 1) }); deletion.resolve(); await running;
    expect(h.io.send).toHaveBeenCalledWith(expect.objectContaining({ message: "original A", regenerate: true, conversation_id: "conversation-a", character_id: "character-a", conversation_generation: 3, allow_image_gen: true }));
  });
  it.each(["conversation", "character", "generation"])("does not regenerate after %s changes while deleting", async changed => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    const deletion = deferred<void>(); vi.spyOn(bridge, "deleteLastMessages").mockReturnValue(deletion.promise);
    const running = h.app.continueFrom(1, true);
    h.set(changed === "conversation" ? { conversation_id: "conversation-b" } : changed === "character" ? { character_id: "character-b" } : { generation: 5 });
    deletion.resolve(); await running; expect(h.io.send).not.toHaveBeenCalled();
  });
  it("a rejected tail guard never sends regeneration or changes projected history", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    vi.spyOn(bridge, "deleteLastMessages").mockRejectedValue(new Error("tail changed"));
    await expect(h.app.continueFrom(1, true)).rejects.toThrow("tail changed");
    expect(h.io.send).not.toHaveBeenCalled(); expect(projectMessages(h.app.getSnapshot())).toHaveLength(3);
  });
});
