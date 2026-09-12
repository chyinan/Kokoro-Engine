// @vitest-environment jsdom
// pattern: Imperative Shell
import { act, createElement, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Live2DViewer from "./Live2DViewer";
import { audioPlayer } from "../../core/services";
import { onChatCue } from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";

vi.mock("pixi.js", () => ({
  utils: { skipHello: vi.fn(), clearTextureCache: vi.fn() }, Ticker: class {},
  Application: class {
    view = document.createElement("canvas");
    stage = { on: vi.fn(), off: vi.fn(), interactive: false, hitArea: null };
    ticker = { maxFPS: 0, add: vi.fn(), remove: vi.fn() };
    screen = {};
    renderer = { resize: vi.fn() };
    destroy = vi.fn();
  },
}));
vi.mock("pixi-live2d-display/cubism4", () => ({ Live2DModel: { registerTicker: vi.fn(), from: vi.fn(() => new Promise(() => {})) } }));
vi.mock("../../core/services", () => ({ audioPlayer: { addAmplitudeListener: vi.fn(), removeAmplitudeListener: vi.fn() } }));
vi.mock("../../core/services/interaction-service", () => ({ interactionService: {} }));
vi.mock("../../lib/kokoro-bridge", () => ({ onChatCue: vi.fn(async () => () => {}), getLive2dModelProfile: vi.fn(async () => null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("Live2DViewer subscription lifecycle", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("releases subscriptions whose registration finishes after unmount", async () => {
    const resolveRegistrations: Array<(cleanup: () => void) => void> = [];
    vi.mocked(listen).mockImplementation(() => new Promise(resolve => resolveRegistrations.push(resolve)));
    vi.mocked(onChatCue).mockImplementation(() => new Promise(resolve => resolveRegistrations.push(resolve)));
    await act(async () => root.render(createElement(Live2DViewer, { modelUrl: "test.model3.json" })));
    expect(resolveRegistrations).toHaveLength(3);
    await act(async () => root.render(null));
    const cleanups = resolveRegistrations.map(() => vi.fn());
    await act(async () => resolveRegistrations.forEach((resolve, index) => resolve(cleanups[index])));
    cleanups.forEach(cleanup => expect(cleanup).toHaveBeenCalledOnce());
  });

  it("keeps an amplitude listener active after StrictMode effect remount", async () => {
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(onChatCue).mockResolvedValue(() => {});
    await act(async () => root.render(createElement(StrictMode, null,
      createElement(Live2DViewer, { modelUrl: "test.model3.json" }))));
    expect(vi.mocked(audioPlayer.addAmplitudeListener).mock.calls.length - vi.mocked(audioPlayer.removeAmplitudeListener).mock.calls.length).toBe(1);
  });
});
