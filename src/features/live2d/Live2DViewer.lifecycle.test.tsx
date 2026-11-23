// @vitest-environment jsdom
// pattern: Imperative Shell
import { act, createElement, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Live2DViewer from "./Live2DViewer";
import { audioPlayer } from "../../core/services";
import { onChatCue, type ChatViewSnapshot } from "../../lib/kokoro-bridge";
import { chatApplication } from "../../lib/chat/application";
import { listen } from "@tauri-apps/api/event";

vi.mock("../../lib/chat/application", () => ({ chatApplication: { getSnapshot: vi.fn() } }));
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

  it("filters delayed chat cues after a character or conversation switch and accepts the original view again", async () => {
    vi.mocked(listen).mockResolvedValue(() => {});
    let deliver: Parameters<typeof onChatCue>[0] = () => {};
    vi.mocked(onChatCue).mockImplementation(async callback => { deliver = callback; return () => {}; });
    const playCue = vi.fn();
    const controller = { playCue } as unknown as NonNullable<Parameters<typeof Live2DViewer>[0]["controller"]>;
    const view = { character_id:"a",conversation_id:"session-a",generation:1,turns:[] } as unknown as ChatViewSnapshot;
    vi.mocked(chatApplication.getSnapshot).mockReturnValue(view);
    await act(async () => root.render(createElement(Live2DViewer, {modelUrl:"test.model3.json",modelSource:"package",controller})));
    const cue = {cue:"smile",character_id:"a",conversation_id:"session-a",client_request_id:"request-a",turn_id:"turn-a",conversation_generation:1};
    deliver({cue:"manual",character_id:"a",conversation_id:null});
    deliver({cue:"other-role-manual",character_id:"b",conversation_id:null});
    expect(playCue).toHaveBeenCalledOnce();
    playCue.mockClear();
    deliver(cue);
    expect(playCue).toHaveBeenCalledTimes(1);
    vi.mocked(chatApplication.getSnapshot).mockReturnValue({...view,character_id:"b",conversation_id:"session-b",generation:2});
    deliver(cue);
    expect(playCue).toHaveBeenCalledTimes(1);
    vi.mocked(chatApplication.getSnapshot).mockReturnValue({...view,generation:3});
    deliver(cue);
    expect(playCue).toHaveBeenCalledTimes(2);
    vi.mocked(chatApplication.getSnapshot).mockReturnValue({...view,conversation_id:null,generation:4});
    deliver({...cue,conversation_id:"reserved",conversation_generation:4});
    expect(playCue).toHaveBeenCalledTimes(3);
    deliver({...cue,conversation_id:"reserved",conversation_generation:3});
    expect(playCue).toHaveBeenCalledTimes(3);
  });

  it("keeps an amplitude listener active after StrictMode effect remount", async () => {
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(onChatCue).mockResolvedValue(() => {});
    await act(async () => root.render(createElement(StrictMode, null,
      createElement(Live2DViewer, { modelUrl: "test.model3.json" }))));
    expect(vi.mocked(audioPlayer.addAmplitudeListener).mock.calls.length - vi.mocked(audioPlayer.removeAmplitudeListener).mock.calls.length).toBe(1);
  });
});
