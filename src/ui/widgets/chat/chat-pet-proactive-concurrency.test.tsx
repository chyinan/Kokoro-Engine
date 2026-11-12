// @vitest-environment node
// Old pet/proactive/interaction handshake rejection and watchdog cases are now
// covered by backend admission/cancellation plus independent window projections.
import { afterEach, describe, expect, it, vi } from "vitest";
import { harness, turn } from "../../../lib/chat/__tests__/harness";
import { projectMessages } from "../../../lib/chat/projection";
afterEach(() => vi.useRealTimers());

describe("main/pet application concurrency and recovery", () => {
  it("independent windows submit directly and surface the backend busy decision", async () => {
    const h = harness(), pet = h.newWindow(); const first = h.pending();
    vi.mocked(h.io.send).mockRejectedValueOnce(new Error("chat_turn_busy"));
    const mainResult = h.app.send({ message: "main", client_request_id: "main" });
    await expect(pet.send({ message: "pet", client_request_id: "pet" })).rejects.toThrow("chat_turn_busy");
    expect(h.io.send).toHaveBeenCalledTimes(2); // two IPC intents; one Rust executor (native test)
    first.resolve({ status: "completed", conversation_id: "conversation-a" }); await mainResult;
    await expect(pet.send({ message: "retry", client_request_id: "retry" })).resolves.toMatchObject({ status: "completed" });
  });
  it("a late command resolution does not clear another window's newer active turn", async () => {
    const h = harness(); const pending = h.pending();
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    const old = h.app.send({ message: "old", client_request_id: "old" });
    h.set({ revision: 8, busy: true, turns: [turn("old", 6, "completed"), turn("pet", 8)] });
    h.emit(turn("pet", 8)); pending.resolve({ status: "completed", conversation_id: "conversation-a" }); await old; await h.app.refresh();
    expect(h.app.getSnapshot().busy).toBe(true);
    expect(h.app.getSnapshot().turns.find(t => t.client_request_id === "pet")?.status).toBe("streaming"); stop();
  });
  it.each(["accepted", "streaming"] as const)("does not let view timers abandon a legitimate slow %s request", async status => {
    vi.useFakeTimers(); const h = harness();
    h.set({ revision: 1, busy: true, turns: [turn("slow", 1, status)] });
    const stop = h.app.subscribe(() => {}); await vi.advanceTimersByTimeAsync(65_000);
    expect(h.app.getSnapshot().busy).toBe(true); expect(h.io.cancel).not.toHaveBeenCalled(); stop();
  });
  it.each(["pending", "accepted", "streaming"] as const)("can stop %s by logical id without waiting for a turn id", async status => {
    const h = harness(); h.set({ revision: 1, busy: true, turns: [{ ...turn("cancel", 1, status), turn_id: status === "streaming" ? "turn_cancel" : null }] });
    await h.app.refresh(); await h.app.cancel();
    expect(h.io.cancel).toHaveBeenCalledWith("cancel", "stopped_by_user");
  });
  it("cancellation before any acknowledgement remains recoverable and retryable", async () => {
    const h = harness(); const pending = h.pending();
    const send = h.app.send({ message: "stop early", client_request_id: "cancel" });
    await h.app.cancel("cancel"); expect(h.io.cancel).toHaveBeenCalledWith("cancel", "stopped_by_user");
    h.set({ revision: 2, busy: false, turns: [turn("cancel", 2, "cancelled")] });
    pending.resolve({ status: "cancelled", conversation_id: "" }); await send; await h.app.refresh();
    expect(h.app.getSnapshot().busy).toBe(false);
    await expect(h.app.send({ message: "retry" })).resolves.toMatchObject({ status: "completed" });
  });
  it("an unrelated rejected request cannot clear the active pet request", async () => {
    const h = harness(); h.set({ revision: 2, busy: true, turns: [turn("pet", 2)] });
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    h.set({ revision: 3, busy: true, turns: [turn("pet", 2), turn("other", 3, "rejected")] });
    h.emit(turn("other", 3, "rejected")); await h.app.refresh();
    expect(h.app.getSnapshot().busy).toBe(true); stop();
  });
  it("repeated mounts in two windows only restore projection and never repeat side effects", async () => {
    const h = harness(), pet = h.newWindow();
    h.set({ revision: 2, busy: true, turns: [turn("shared", 2)] });
    for (let cycle = 0; cycle < 4; cycle++) {
      const mainStop = h.app.subscribe(() => {}), petStop = pet.subscribe(() => {});
      await Promise.all([h.app.refresh(), pet.refresh()]);
      expect(projectMessages(h.app.getSnapshot())).toEqual(projectMessages(pet.getSnapshot()));
      mainStop(); petStop();
    }
    expect(h.io.send).not.toHaveBeenCalled(); expect(h.io.cancel).not.toHaveBeenCalled();
  });
  it("same logical delivery in a window shares its command result", async () => {
    const h = harness(); const pending = h.pending(); const request = { message: "same", client_request_id: "same" };
    const one = h.app.send(request), two = h.app.send({ ...request });
    expect(one).toBe(two); expect(h.io.send).toHaveBeenCalledOnce();
    pending.resolve({ status: "completed", conversation_id: "conversation-a" }); await one;
  });
});