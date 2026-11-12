// @vitest-environment node
// History ownership moved from Panel closures to captured application commands.
import { afterEach, describe, expect, it, vi } from "vitest";
import * as bridge from "../../../lib/kokoro-bridge";
import { harness, deferred, turn } from "../../../lib/chat/__tests__/harness";
import { projectMessages } from "../../../lib/chat/projection";
import type { ChatViewSnapshot } from "../../../lib/kokoro-bridge";
const messages = [
  { id: 101, role: "user", content: "original A", created_at: "now" },
  { id: 102, role: "assistant", content: "answer A", created_at: "now" },
  { id: 103, role: "user", content: "later A", created_at: "now" },
];
const editTarget = { conversationId: "conversation-a", generation: 3, messageId: 101 };
afterEach(() => vi.restoreAllMocks());

describe("application history command session ownership", () => {
  it("cancels a true character switch by request id, but not a same-character runtime update", async () => {
    const h = harness();
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
    const running = h.app.edit(editTarget, "edited A");
    expect(invoke).toHaveBeenCalledWith({ conversation_id: "conversation-a", message_id: 101, new_content: "edited A" });
    h.set({ conversation_id: "conversation-b", generation: 4, messages: [{ id: 201, role: "user", content: "original B", created_at: "now" }] });
    await h.app.refresh(); edit.resolve({ message_id: 101, updated_content: "edited A" }); await running;
    expect(projectMessages(h.app.getSnapshot())).toMatchObject([{ id: 201, text: "original B" }]);
  });
  it("drops a stale save after switching away and after returning to the same conversation", async () => {
    const h = harness(); h.set({ messages }); await h.app.refresh();
    const invoke = vi.spyOn(bridge, "editConversationMessage").mockResolvedValue({ message_id: 101, updated_content: "edited A" });
    h.set({ conversation_id: "conversation-b", generation: 4, messages: [{ ...messages[0], id: 201, content: "original B" }] });
    await h.app.refresh();
    await h.app.edit(editTarget, "edited A");
    h.set({ conversation_id: "conversation-a", generation: 5, messages });
    await h.app.refresh();
    await h.app.edit(editTarget, "edited A");
    expect(invoke).not.toHaveBeenCalled();
  });
  it("reads edited history even while a pre-edit poll is still in flight", async () => {
    const h = harness(); h.set({ messages });
    const before = await h.app.refresh();
    const stale = deferred<ChatViewSnapshot>();
    vi.mocked(h.io.read).mockReturnValueOnce(stale.promise);
    const poll = h.app.refresh();
    vi.spyOn(bridge, "editConversationMessage").mockImplementation(async () => {
      h.set({ messages: [{ ...messages[0], content: "edited A" }, ...messages.slice(1)] });
      return { message_id: 101, updated_content: "edited A" };
    });
    const editing = h.app.edit(editTarget, "edited A");
    await vi.waitFor(() => expect(h.io.read).toHaveBeenCalledTimes(3));
    await editing;
    stale.resolve(before); await poll;
    expect(projectMessages(h.app.getSnapshot())[0]?.text).toBe("edited A");
  });
  it("restores the new selection before returning when a pre-switch poll is still in flight", async () => {
    const h = harness(); h.set({ messages });
    const before = await h.app.refresh();
    const loadStarted = deferred<void>();
    const load = deferred<bridge.LoadedConversation>();
    vi.spyOn(bridge, "loadConversation").mockImplementation(() => { loadStarted.resolve(); return load.promise; });
    const switching = h.app.selectConversation("conversation-b");
    await loadStarted.promise;
    const stale = deferred<ChatViewSnapshot>();
    vi.mocked(h.io.read).mockReturnValueOnce(stale.promise);
    const poll = h.app.refresh();
    h.set({ conversation_id: "conversation-b", generation: 4, messages: [] });
    load.resolve({ topic: "", pinned_state: "", messages: [] });
    await vi.waitFor(() => expect(h.io.read).toHaveBeenCalledTimes(4));
    await switching;
    stale.resolve(before); await poll;
    expect(h.app.getSnapshot()).toMatchObject({ conversation_id: "conversation-b", generation: 4, messages: [], busy: false });
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
  it("regenerates from a restored turn with the selected image and without the original camera frame", async () => {
    const h = harness();
    const images = ["http://127.0.0.1:1234/vision/test.png"];
    const cameraFrame = "data:image/jpeg;base64,Y2FtZXJh";
    await h.app.refresh();
    const completion = h.pending();
    const sending = h.app.send({ message: "original A", images, camera_frame: cameraFrame, client_request_id: "image-turn" });
    expect(projectMessages(h.app.getSnapshot())).toEqual([
      { role: "user", text: "original A", images, clientRequestId: "image-turn" },
    ]);
    h.set({ revision: 2, messages: messages.slice(0, 2), turns: [{ ...turn("image-turn", 2, "completed"), user_message_id: 101, images }] });
    completion.resolve({ status: "completed", conversation_id: "conversation-a", text: "answer A" });
    await sending;
    const reopened = h.newWindow(); await reopened.refresh();
    const deletion = deferred<void>(); const remove = vi.spyOn(bridge, "deleteLastMessages").mockReturnValue(deletion.promise);
    const running = reopened.continueFrom(1, true, { allow_image_gen: true });
    expect(remove).toHaveBeenCalledWith(1, "conversation-a", 102);
    expect(h.io.send).toHaveBeenCalledOnce();
    h.set({ messages: messages.slice(0, 1) }); deletion.resolve(); await running;
    expect(h.io.send).toHaveBeenNthCalledWith(2, expect.objectContaining({ message: "original A", images, regenerate: true, conversation_id: "conversation-a", character_id: "character-a", conversation_generation: 3, allow_image_gen: true }));
    expect(vi.mocked(h.io.send).mock.lastCall?.[0]).not.toHaveProperty("camera_frame");
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
