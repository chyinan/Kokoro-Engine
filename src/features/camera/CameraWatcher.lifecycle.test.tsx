// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CameraWatcher } from "./CameraWatcher";
import { getLatestCameraFrame, setLatestCameraFrame } from "@/lib/camera-frame-cache";

describe("CameraWatcher playback lifecycle", () => {
  let root: Root;
  let container: HTMLDivElement;
  let finishPlay: () => void;
  let stopTrack: ReturnType<typeof vi.fn>;
  let mediaDevices: PropertyDescriptor | undefined;
  let actEnvironment: PropertyDescriptor | undefined;

  beforeEach(() => {
    vi.useFakeTimers();
    actEnvironment = Object.getOwnPropertyDescriptor(globalThis, "IS_REACT_ACT_ENVIRONMENT");
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true, writable: true });
    mediaDevices = Object.getOwnPropertyDescriptor(navigator, "mediaDevices");
    stopTrack = vi.fn();
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop: stopTrack }] })) },
    });
    vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(() => new Promise<void>(resolve => { finishPlay = resolve; }));
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    setLatestCameraFrame(null);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    setLatestCameraFrame(null);
    vi.restoreAllMocks();
    vi.useRealTimers();
    if (mediaDevices) Object.defineProperty(navigator, "mediaDevices", mediaDevices);
    else Reflect.deleteProperty(navigator, "mediaDevices");
    if (actEnvironment) Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", actEnvironment);
    else Reflect.deleteProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT");
  });

  it("does not restart the capture interval when play finishes after disabling", async () => {
    await act(async () => root.render(createElement(CameraWatcher, { enabled: true })));
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledOnce();
    await act(async () => root.render(createElement(CameraWatcher, { enabled: false })));
    expect(stopTrack).toHaveBeenCalled();
    await act(async () => { finishPlay(); });
    expect(vi.getTimerCount()).toBe(0);
    expect(getLatestCameraFrame()).toBeNull();
  });
});
