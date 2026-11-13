// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ChatViewSnapshot, StreamChatResponse } from "../../lib/kokoro-bridge";

const ipc = vi.hoisted(() => ({ send: vi.fn(), read: vi.fn(), listen: vi.fn(async () => () => {}) }));
vi.mock("../../lib/kokoro-bridge", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/kokoro-bridge")>(),
  streamChat: ipc.send, getChatView: ipc.read, onChatRuntimeChanged: ipc.listen,
}));
let app: typeof import("../../lib/chat/application").chatApplication;
let service: typeof import("./interaction-service").interactionService;
const controller = { playCue: vi.fn(), resolveInteractionSemanticCue: () => null };
const gesture = { gesture: "tap" as const, hitArea: "Head", consecutiveTaps: 1 };
beforeEach(async () => {
  vi.resetModules(); vi.useFakeTimers(); vi.setSystemTime(10000); vi.clearAllMocks();
  const module = await import("../../lib/chat/application");
  app = module.chatApplication;
  ipc.read.mockResolvedValue({ ...module.emptyChatView } satisfies ChatViewSnapshot);
  ipc.send.mockResolvedValue({ status: "completed", conversation_id: "conv" } satisfies StreamChatResponse);
  service = (await import("./interaction-service")).interactionService;
  await vi.advanceTimersByTimeAsync(0);
});
afterEach(() => { service.destroy(); app.disconnect(); vi.useRealTimers(); });

it("retries when real ChatApplication publishes idle before the busy rejection reaches the gesture", async () => {
  let reject!: (error: Error) => void;
  ipc.send.mockImplementationOnce(() => new Promise((_, no) => { reject = no; }));
  const sending = service.handleGesture(gesture, controller);
  expect(app.getSnapshot().busy).toBe(true);
  reject(new Error("chat_turn_busy"));
  await sending;
  expect(app.getSnapshot().busy).toBe(false);
  expect(ipc.send).toHaveBeenCalledOnce();
  // No additional state publication is required to wake the retained gesture.
  await vi.advanceTimersByTimeAsync(500);
  expect(ipc.send).toHaveBeenCalledTimes(2);
  expect(ipc.send.mock.calls[1][0].message).toBe(ipc.send.mock.calls[0][0].message);
});

it("bounds retries against stale idle snapshots and stops them on destroy", async () => {
  ipc.send.mockRejectedValue(new Error("chat_turn_busy"));
  await service.handleGesture(gesture, controller);
  await vi.advanceTimersByTimeAsync(499);
  expect(ipc.send).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1);
  expect(ipc.send).toHaveBeenCalledTimes(2);
  service.destroy();
  await vi.advanceTimersByTimeAsync(1000);
  expect(ipc.send).toHaveBeenCalledTimes(2);
});
