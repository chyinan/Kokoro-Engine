// @vitest-environment jsdom
// pattern: Imperative Shell
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { useWakeWord } from "./useWakeWord";

vi.mock("../../utils/env", () => ({ isTauriEnvironment: () => false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => { vi.unstubAllGlobals(); document.body.replaceChildren(); });

it("review_r10 releases microphone permission granted after wake-word unmount", async () => {
  let grant!: (stream: MediaStream) => void;
  const stopped = vi.fn();
  const stream = { getTracks: () => [{ stop: stopped }] } as unknown as MediaStream;
  const getUserMedia = vi.fn(() => new Promise<MediaStream>(resolve => { grant = resolve; }));
  Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia } });
  const contextCreated = vi.fn();
  vi.stubGlobal("AudioContext", class {
    constructor() { contextCreated(); }
    destination = {};
    createMediaStreamSource() { return { connect: vi.fn(), disconnect: vi.fn() }; }
    createScriptProcessor() { return { connect: vi.fn(), disconnect: vi.fn(), onaudioprocess: null }; }
    close = vi.fn().mockResolvedValue(undefined);
  });
  function Harness(): null {
    useWakeWord({ enabled: true, mode: "speech", onWakeWordDetected: vi.fn() });
    return null;
  }
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => { root.render(createElement(Harness)); });
  expect(getUserMedia).toHaveBeenCalledOnce();
  await act(async () => { root.unmount(); });
  await act(async () => { grant(stream); await Promise.resolve(); });
  const released = stopped.mock.calls.length;
  // Cleanup the fake resource even when the expected lifecycle assertion fails.
  stopped();
  expect(released).toBe(1);
  expect(contextCreated).not.toHaveBeenCalled();
});
