// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { usePetChat, type PetChatState } from "./usePetChat";
import { chatViewFixture as chat } from "../../lib/chat/view-test-fixture";
import type { ChatTurnSnapshot } from "../../lib/kokoro-bridge";
import { deferred } from "../../lib/chat/__tests__/harness";
import type { StreamChatResponse } from "../../lib/kokoro-bridge";
vi.mock("../../lib/chat/application", async () => ({ chatApplication: (await import("../../lib/chat/view-test-fixture")).chatViewFixture }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => {}) }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const snapshot = (status: ChatTurnSnapshot["status"] = "streaming", revision = 2): ChatTurnSnapshot => ({
 revision, client_request_id: "pet_a", turn_id: "a", character_id: "char-1", conversation_id: "conv-1", user_message_id: 1, assistant_message_id: 2,
 message: "hello", images: [], hidden: false, status, text: "complete recovered text", translation: null, tools: [], error: null,
});
describe("pet chat display adapter", () => {
 let root: Root, container: HTMLDivElement, state: PetChatState;
 function Harness() { state = usePetChat(); return null; }
 beforeEach(async () => {
  vi.clearAllMocks(); chat.reset(); chat.send.mockResolvedValue({ status: "completed", conversation_id: "conv-1", text: "reply" });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(createElement(Harness)));
 });
 afterEach(() => { act(() => root.unmount()); container.remove(); vi.useRealTimers(); });
 it("submits directly without ChatPanel or an acceptance handshake", async () => {
  await act(async () => state.sendMessage("  hello  "));
  expect(chat.send).toHaveBeenCalledWith({ message: "hello", client_request_id: expect.stringMatching(/^pet_/) });
  expect(invoke).not.toHaveBeenCalledWith("stream_chat", expect.anything());
 });
 it("coalesces duplicate clicks while the same user intent is in flight", async () => {
  let done!: (value: any) => void; chat.send.mockImplementationOnce(() => new Promise(resolve => { done = resolve; }));
  let first!: Promise<void>; await act(async () => { first = state.sendMessage("one"); void state.sendMessage("one"); });
  expect(chat.send).toHaveBeenCalledOnce(); done({ status: "completed" }); await first;
 });
 it("shows the recovered reply on remount and updates the same bubble on later snapshots", async () => {
  await act(async () => chat.set({ turns: [snapshot()], busy: true }));
  act(() => root.unmount());
  vi.mocked(invoke).mockClear();
  root = createRoot(container);
  await act(async () => root.render(createElement(Harness)));
  expect(invoke).toHaveBeenCalledExactlyOnceWith("show_bubble_window", { text: "complete recovered text" });
  expect(state.isStreaming).toBe(true);
  await act(async () => chat.set({ turns: [{ ...snapshot("streaming", 3), text: "intermediate recovered text" }] }));
  expect(invoke).toHaveBeenLastCalledWith("update_bubble_text", { text: "intermediate recovered text" });
  expect(state.isStreaming).toBe(true);
  await act(async () => chat.set({ turns: [{ ...snapshot("completed", 4), text: "final recovered text" }], busy: false }));
  expect(vi.mocked(invoke).mock.calls).toEqual([
   ["show_bubble_window", { text: "complete recovered text" }],
   ["update_bubble_text", { text: "intermediate recovered text" }],
   ["update_bubble_text", { text: "final recovered text" }],
  ]);
 });
 it("keeps the submitted reply selected when the first history read appends older pet turns", async () => {
  const response = deferred<StreamChatResponse>();
  chat.send.mockReturnValueOnce(response.promise);
  let sending!: Promise<void>;
  await act(async () => { sending = state.sendMessage("new prompt"); });
  const id = chat.send.mock.calls[0][0].client_request_id!;
  const pending = { ...snapshot("pending", 0), client_request_id: id, text: "" };
  const old = { ...snapshot("completed", 5), text: "old reply" };
  await act(async () => chat.set({ turns: [pending, old], busy: true }));
  expect(invoke).not.toHaveBeenCalledWith("update_bubble_text", expect.anything());
  await act(async () => chat.set({ turns: [{ ...pending, status: "streaming", revision: 6, text: "new" }, old] }));
  await act(async () => {
   chat.set({ turns: [{ ...pending, status: "completed", revision: 7, text: "new reply" }, old], busy: false });
   response.resolve({ conversation_id: "conv-1", status: "completed" });
   await sending;
  });
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "update_bubble_text")).toEqual([
   ["update_bubble_text", { text: "new" }], ["update_bubble_text", { text: "new reply" }],
  ]);
 });
 it("does not concatenate replayed full snapshots", async () => {
  await act(async () => chat.set({ turns: [snapshot()] }));
  await act(async () => chat.set({ turns: [snapshot()] }));
  expect(invoke).toHaveBeenCalledExactlyOnceWith("show_bubble_window", { text: "complete recovered text" });
 });
 it("a late speech failure does not replay the reply or restart its hide timer", async () => {
  vi.useFakeTimers();
  await act(async () => chat.set({ turns: [snapshot()] }));
  const completed = { ...snapshot("completed", 3), completion_revision: 3 };
  await act(async () => chat.set({ turns: [completed] }));
  await act(async () => vi.advanceTimersByTimeAsync(4000));
  await act(async () => chat.set({ turns: [{ ...completed, revision: 4, speech_error: "Speech failed" }] }));
  expect(vi.mocked(invoke).mock.calls).toHaveLength(2);
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(invoke).toHaveBeenCalledWith("hide_bubble_window");
 });
 it("does not present another entry's reply as a pet request", async () => {
  await act(async () => chat.set({ turns: [{ ...snapshot(), client_request_id: "interaction_a" }] }));
  expect(invoke).not.toHaveBeenCalled();
 });
 it.each(["completed", "cancelled", "error"] as const)("settles %s from recovered state without waiting for finish", async status => {
  vi.useFakeTimers(); await act(async () => chat.set({ turns: [snapshot(status)], busy: false }));
  expect(state.isStreaming).toBe(false); await act(async () => vi.advanceTimersByTimeAsync(5000));
  expect(invoke).not.toHaveBeenCalled();
 });
 it("keeps an expired reply hidden after unmount and reopen", async () => {
  vi.useFakeTimers();
  await act(async () => chat.set({ turns: [snapshot()], busy: true }));
  await act(async () => chat.set({ turns: [snapshot("completed", 3)], busy: false }));
  await act(async () => vi.advanceTimersByTimeAsync(5000));
  expect(invoke).toHaveBeenLastCalledWith("hide_bubble_window");
  act(() => root.unmount());
  vi.mocked(invoke).mockClear();
  root = createRoot(container);
  await act(async () => root.render(createElement(Harness)));
  await act(async () => chat.set({ turns: [{ ...snapshot("completed", 4), completion_revision: 3, speech_error: "late failure" }] }));
  await act(async () => vi.advanceTimersByTimeAsync(6000));
  expect(invoke).not.toHaveBeenCalled();
  expect(chat.send).not.toHaveBeenCalled();
 });
 it("hides the former character's reply and follows the new character", async () => {
  const old = snapshot();
  await act(async () => chat.set({ turns: [old], busy: true }));
  vi.mocked(invoke).mockClear();
  await act(async () => chat.set({ character_id: "char-2" }));
  expect(invoke).toHaveBeenCalledExactlyOnceWith("hide_bubble_window");
  const current = { ...snapshot("streaming", 3), character_id: "char-2", client_request_id: "pet_b", text: "new character reply" };
  await act(async () => chat.set({ turns: [old, current] }));
  expect(invoke).toHaveBeenLastCalledWith("show_bubble_window", { text: "new character reply" });
  await act(async () => chat.set({ turns: [{ ...old, revision: 5, text: "late foreign text" }, current] }));
  expect(vi.mocked(invoke).mock.calls).toHaveLength(2);
 });
 it("ignores a foreign active reply on reopen even when it is newer", async () => {
  await act(async () => chat.set({ character_id: "char-2", turns: [snapshot()] }));
  act(() => root.unmount());
  vi.mocked(invoke).mockClear();
  root = createRoot(container);
  await act(async () => root.render(createElement(Harness)));
  expect(invoke).not.toHaveBeenCalled();
 });
 it("does not reopen a foreign bubble when its delayed update fails after switching away and back", async () => {
  await act(async () => chat.set({ turns: [snapshot()], busy: true }));
  const update = deferred<void>();
  vi.mocked(invoke).mockImplementationOnce(() => update.promise);
  await act(async () => chat.set({ turns: [snapshot("streaming", 3)] }));
  await act(async () => chat.set({ character_id: "char-2", turns: [snapshot("completed", 4)] }));
  await act(async () => chat.set({ character_id: "char-1" }));
  vi.mocked(invoke).mockClear();
  await act(async () => update.reject(new Error("bubble was destroyed")));
  expect(invoke).not.toHaveBeenCalled();
 });
 it("suppresses a rejected send after switching character and releases the old request selection", async () => {
  const response = deferred<StreamChatResponse>();
  chat.send.mockReturnValueOnce(response.promise);
  let sending!: Promise<void>;
  await act(async () => { sending = state.sendMessage("old character question"); });
  await act(async () => chat.set({ character_id: "char-2" }));
  vi.mocked(invoke).mockClear();
  await act(async () => { response.reject(new Error("old character failure")); await sending; });
  expect(invoke).not.toHaveBeenCalled();
  await act(async () => chat.set({ turns: [{ ...snapshot(), character_id: "char-2" }] }));
  expect(invoke).toHaveBeenCalledExactlyOnceWith("show_bubble_window", { text: "complete recovered text" });
 });
 it("unmount does not cancel an in-flight business request", async () => {
  await act(async () => chat.set({ turns: [snapshot()], busy: true }));
  act(() => root.unmount()); expect(chat.cancel).not.toHaveBeenCalled();
  root = createRoot(container); await act(async () => root.render(createElement(Harness)));
  expect(state.isStreaming).toBe(true); expect(chat.send).not.toHaveBeenCalled();
 });
 it("displays authoritative busy rejection and permits a subsequent send", async () => {
  chat.send.mockRejectedValueOnce(new Error("chat_turn_busy"));
  await act(async () => state.sendMessage("one"));
  expect(invoke).toHaveBeenCalledWith("show_bubble_window", { text: "chat_turn_busy" });
  await act(async () => state.sendMessage("two")); expect(chat.send).toHaveBeenCalledTimes(2);
 });
 it("ignores empty input", async () => { await state.sendMessage("  "); expect(chat.send).not.toHaveBeenCalled(); });
 it("preserves structured backend rejection text in the bubble", async () => {
  chat.send.mockRejectedValueOnce({ code: "Chat", message: "chat_turn_busy: active request" });
  await act(async () => state.sendMessage("one"));
  expect(invoke).toHaveBeenCalledWith("show_bubble_window", { text: "chat_turn_busy: active request" });
 });
});
