// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chatViewFixture as chat } from "../../lib/chat/view-test-fixture";
import { InteractionService } from "./interaction-service";
vi.mock("../../lib/chat/application", async () => {
 const { chatViewFixture } = await import("../../lib/chat/view-test-fixture");
 return { chatApplication: chatViewFixture, submitChat: (...args: unknown[]) => (chatViewFixture.send as Function)(...args) };
});
describe("InteractionService application input adapter", () => {
 let service: InteractionService;
 const controller = { playCue: vi.fn(), resolveInteractionSemanticCue: vi.fn(() => null) };
 beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(10000); chat.reset(); localStorage.clear(); service = new InteractionService(); chat.send.mockResolvedValue({ status: "completed", conversation_id: "conv-1", text: "reply" }); });
 afterEach(() => { service.destroy(); vi.useRealTimers(); });
 it("submits hidden gestures without a panel acceptance event", async () => {
  await service.handleGesture({ gesture: "tap", hitArea: "Head", consecutiveTaps: 1 }, controller);
  expect(chat.send).toHaveBeenCalledWith(expect.objectContaining({ message: "(User gently poked your head)", hidden: true, client_request_id: expect.stringMatching(/^interaction_/) }));
 });
 it("preserves gesture cooldown without creating model work", async () => {
  const gesture = { gesture: "tap" as const, hitArea: "Head", consecutiveTaps: 1 };
  await service.handleGesture(gesture, controller); await service.handleGesture(gesture, controller);
  expect(chat.send).toHaveBeenCalledOnce();
 });
 it("retains the existing single coalesced retry only after backend busy rejection", async () => {
  chat.send.mockRejectedValueOnce(new Error("chat_turn_busy"));
  await service.handleGesture({ gesture: "tap", hitArea: "Head", consecutiveTaps: 1 }, controller);
  expect(chat.send).toHaveBeenCalledOnce(); chat.set({ busy: false }); await Promise.resolve();
  expect(chat.send).toHaveBeenCalledTimes(2);
 });
 it("does not retry provider failure and permits a later gesture", async () => {
  chat.send.mockRejectedValueOnce(new Error("provider unavailable"));
  await service.handleGesture({ gesture: "tap", hitArea: "Head", consecutiveTaps: 1 }, controller);
  chat.set({ busy: false }); expect(chat.send).toHaveBeenCalledOnce();
  vi.advanceTimersByTime(501); await service.handleGesture({ gesture: "long_press", hitArea: "Body", consecutiveTaps: 1 }, controller);
  expect(chat.send).toHaveBeenCalledTimes(2);
 });
 it("formats rapid tapping and response-language hints", async () => {
  localStorage.setItem("kokoro_response_language", "Chinese");
  for (let n = 0; n < 3; n++) { await service.handleGesture({ gesture: "tap", hitArea: "Head", consecutiveTaps: 1 }, controller); vi.advanceTimersByTime(501); }
  expect(chat.send).toHaveBeenLastCalledWith(expect.objectContaining({ message: expect.stringContaining("rapidly pokes") }));
  expect(chat.send).toHaveBeenLastCalledWith(expect.objectContaining({ message: expect.stringContaining("Respond in Chinese") }));
 });
 it("destroy drops a pending gesture without cancelling unrelated work", async () => {
  chat.send.mockRejectedValueOnce(new Error("chat_turn_busy"));
  await service.handleGesture({ gesture: "tap", hitArea: "Head", consecutiveTaps: 1 }, controller);
  service.destroy(); chat.set({ busy: false }); expect(chat.send).toHaveBeenCalledOnce(); expect(chat.cancel).not.toHaveBeenCalled();
 });
});
