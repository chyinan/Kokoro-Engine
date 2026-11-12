// @vitest-environment node
import { describe, expect, it, vi } from "vitest";
import { OnboardingChat } from "./onboarding-chat";
import { deferred } from "../../lib/chat/__tests__/harness";
import type { ChatRequest, StreamChatResponse } from "../../lib/kokoro-bridge";
function fixture() {
  const response = deferred<StreamChatResponse>();
  const chat = { send: vi.fn((_request: ChatRequest) => response.promise), cancel: vi.fn(async () => {}) };
  return { response, chat, onboarding: new OnboardingChat(chat) };
}
describe("onboarding first reply owner", () => {
  it("uses canonical final text without any mounted view or finish event", async () => {
    const f = fixture(); const reply = f.onboarding.start("hello", "character-a");
    expect(f.chat.send).toHaveBeenCalledWith(expect.objectContaining({ character_id: "character-a", client_request_id: expect.stringMatching(/^onboarding_/) }));
    f.response.resolve({ status: "completed", text: "final answer", conversation_id: "conversation-a" });
    await expect(reply).resolves.toBe("final answer");
  });
  it("dismisses before turn id exists and ignores late success", async () => {
    const f = fixture(); const reply = f.onboarding.start("hello");
    const id = f.chat.send.mock.calls[0][0].client_request_id;
    f.onboarding.dismiss(); await expect(reply).resolves.toBe("");
    expect(f.chat.cancel).toHaveBeenCalledWith(id, "onboarding_dismissed");
    f.response.resolve({ status: "completed", text: "late", conversation_id: "conversation-a" });
    await Promise.resolve(); await expect(reply).resolves.toBe("");
  });
  it("a dismissed old completion cannot settle a newer onboarding attempt", async () => {
    const f = fixture(), next = deferred<StreamChatResponse>();
    const first = f.onboarding.start("old"); f.onboarding.dismiss(); await first;
    f.chat.send.mockReturnValueOnce(next.promise);
    const second = f.onboarding.start("new"); let settled = false; void second.then(() => { settled = true; });
    f.response.resolve({ status: "completed", text: "old", conversation_id: "old" }); await Promise.resolve();
    expect(settled).toBe(false); next.resolve({ status: "completed", text: "new", conversation_id: "new" });
    await expect(second).resolves.toBe("new");
  });
  it.each(["cancelled", "error", "completed"])("does not complete onboarding from %s without a usable reply", async status => {
    const f = fixture(); const reply = f.onboarding.start("hello");
    f.response.resolve({ status, text: "", conversation_id: "conversation-a" });
    await expect(reply).rejects.toThrow("chat turn");
  });
  it("rejects failed sends and allows retry", async () => {
    const f = fixture(); const reply = f.onboarding.start("hello");
    f.response.reject(new Error("provider unavailable")); await expect(reply).rejects.toThrow("provider unavailable");
    f.chat.send.mockResolvedValueOnce({ status: "completed", text: "retry", conversation_id: "conversation-a" });
    await expect(f.onboarding.start("retry")).resolves.toBe("retry");
  });
});
