// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VisionConfig } from "@/lib/kokoro-bridge";
import { saveVisionConfig } from "@/lib/kokoro-bridge";
import VisionTab from "./VisionTab";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Mock window.scrollTo for jsdom
window.scrollTo = vi.fn();

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

vi.mock("@/lib/kokoro-bridge", () => ({
  getVisionConfig: vi.fn(async () => defaultConfig),
  saveVisionConfig: vi.fn(async () => undefined),
  captureScreenNow: vi.fn(async () => "test capture"),
  listOllamaModels: vi.fn(async () => []),
  listAnthropicModels: vi.fn(async () => []),
  getLlamaCppStatus: vi.fn(async () => ({ available_models: [], current_model: null })),
  listVisionScreens: vi.fn(async () => []),
  getKokoroErrorMessage: (err: unknown) => String(err),
}));

const defaultConfig: VisionConfig = {
  vlm_enabled: true,
  auto_vision_enabled: false,
  vision_context_history_mode: "latest",
  capture_interval_secs: 5,
  change_threshold: 0.1,
  display_id: null,
  vlm_region: null,
  proactive_vision_enabled: false,
  camera_enabled: true,
  camera_device_id: "mock-camera-1",
  vlm_provider: "llm",
  vlm_base_url: "http://localhost:11434",
  vlm_model: "llava",
  vlm_api_key: "",
};

describe("VisionTab lifecycle", () => {
  let previewTrackStop: ReturnType<typeof vi.fn>;
  let permissionTrackStop: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    previewTrackStop = vi.fn();
    permissionTrackStop = vi.fn();

    let callIndex = 0;
    Object.defineProperty(globalThis.navigator, "mediaDevices", {
      value: {
        getUserMedia: vi.fn(async () => {
          callIndex++;
          if (callIndex === 1) {
            // Permission check stream in enumerateDevices
            const track = {
              stop: permissionTrackStop,
              kind: "video",
              enabled: true,
              readyState: "live",
            } as unknown as MediaStreamTrack;
            return {
              getTracks: () => [track],
              getVideoTracks: () => [track],
            } as unknown as MediaStream;
          }

          // Active preview stream
          const previewTrack = {
            stop: previewTrackStop,
            kind: "video",
            enabled: true,
            readyState: "live",
          } as unknown as MediaStreamTrack;
          return {
            getTracks: () => [previewTrack],
            getVideoTracks: () => [previewTrack],
          } as unknown as MediaStream;
        }),
        enumerateDevices: vi.fn(async () => [
          { deviceId: "mock-camera-1", kind: "videoinput", label: "Camera 1" },
        ]),
      },
      configurable: true,
      writable: true,
    });

    Object.defineProperty(HTMLMediaElement.prototype, "play", {
      value: vi.fn(async () => undefined),
      configurable: true,
      writable: true,
    });
  });

  afterEach(() => {
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it("modifying non-camera fields preserves active preview stream without stopping tracks", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    let currentConfig: VisionConfig = { ...defaultConfig };
    const onConfigChange = vi.fn((next: VisionConfig) => {
      currentConfig = next;
    });

    // 1. Initial mount with camera enabled
    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: currentConfig,
          committedConfig: defaultConfig,
          onConfigChange,
        }),
      );
    });

    // Wait for enumerateDevices and startPreview to finish
    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Permission track was stopped as part of enumerateDevices
    expect(permissionTrackStop).toHaveBeenCalled();
    // Live preview track MUST be running and NOT stopped
    expect(previewTrackStop).not.toHaveBeenCalled();

    // 2. User updates non-camera fields (e.g. VLM base url, capture interval)
    const updatedDraft: VisionConfig = {
      ...currentConfig,
      vlm_base_url: "http://192.168.1.100:11434",
      capture_interval_secs: 15,
      change_threshold: 0.2,
    };

    // SettingsPanel passes down the updated draft as initialConfig
    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: updatedDraft,
          committedConfig: defaultConfig,
          onConfigChange,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // CRITICAL: Live preview track MUST NOT be stopped when non-camera fields change!
    expect(previewTrackStop).not.toHaveBeenCalled();

    // 3. Unmount component -> now previewTrack MUST be stopped
    act(() => {
      root.unmount();
    });

    expect(previewTrackStop).toHaveBeenCalledTimes(1);
  });

  it("toggling switches and selects updates draft via onConfigChange without calling saveVisionConfig or localStorage", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    const onConfigChange = vi.fn();
    const setItemSpy = vi.spyOn(Storage.prototype, "setItem");

    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, vlm_enabled: true, auto_vision_enabled: false },
          committedConfig: defaultConfig,
          onConfigChange,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    const vlmToggle = container.querySelector<HTMLButtonElement>('[aria-label="vlm-enable-toggle"]');
    expect(vlmToggle).not.toBeNull();

    await act(async () => {
      vlmToggle?.click();
    });

    expect(saveVisionConfig).not.toHaveBeenCalled();
    expect(setItemSpy).not.toHaveBeenCalledWith("kokoro_vision_config", expect.anything());
    expect(onConfigChange).toHaveBeenCalledWith(
      expect.objectContaining({ vlm_enabled: false }),
    );

    const autoToggle = container.querySelector<HTMLButtonElement>('[aria-label="auto-vision-enable-toggle"]');
    expect(autoToggle).not.toBeNull();

    await act(async () => {
      autoToggle?.click();
    });

    expect(saveVisionConfig).not.toHaveBeenCalled();
    expect(setItemSpy).not.toHaveBeenCalledWith("kokoro_vision_config", expect.anything());
    expect(onConfigChange).toHaveBeenCalledWith(
      expect.objectContaining({ auto_vision_enabled: true }),
    );

    act(() => {
      root.unmount();
    });
    setItemSpy.mockRestore();
  });

  it("stops tracks when getUserMedia resolves after component has unmounted", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    let resolveGetUserMedia: ((stream: MediaStream) => void) | null = null;
    const pendingTrackStop = vi.fn();

    (navigator.mediaDevices.getUserMedia as any).mockImplementation(() => {
      return new Promise<MediaStream>((resolve) => {
        resolveGetUserMedia = resolve;
      });
    });

    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true },
          committedConfig: defaultConfig,
        }),
      );
    });

    // Unmount while getUserMedia is pending
    act(() => {
      root.unmount();
    });

    // Resolve getUserMedia late
    const lateTrack = {
      stop: pendingTrackStop,
      kind: "video",
      enabled: true,
      readyState: "live",
    } as unknown as MediaStreamTrack;

    const lateStream = {
      getTracks: () => [lateTrack],
      getVideoTracks: () => [lateTrack],
    } as unknown as MediaStream;

    await act(async () => {
      resolveGetUserMedia?.(lateStream);
      await Promise.resolve();
    });

    expect(pendingTrackStop).toHaveBeenCalledTimes(1);
  });

  it("stops stale stream when switching camera device while previous request is pending", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    let resolveFirstStream: ((stream: MediaStream) => void) | null = null;
    const firstTrackStop = vi.fn();
    const secondTrackStop = vi.fn();

    let callCount = 0;
    (navigator.mediaDevices.getUserMedia as any).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        // Enumerate permission check
        const track = { stop: vi.fn(), kind: "video" } as any;
        return { getTracks: () => [track], getVideoTracks: () => [track] } as any;
      }
      if (callCount === 2) {
        // First preview request - pending
        return new Promise<MediaStream>((resolve) => {
          resolveFirstStream = resolve;
        });
      }
      // Second preview request - resolves immediately
      const secondTrack = {
        stop: secondTrackStop,
        kind: "video",
        enabled: true,
        readyState: "live",
      } as unknown as MediaStreamTrack;
      return {
        getTracks: () => [secondTrack],
        getVideoTracks: () => [secondTrack],
      } as unknown as MediaStream;
    });

    (navigator.mediaDevices.enumerateDevices as any).mockImplementation(async () => [
      { deviceId: "cam-1", kind: "videoinput", label: "Cam 1" },
      { deviceId: "cam-2", kind: "videoinput", label: "Cam 2" },
    ]);

    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true, camera_device_id: "cam-1" },
          committedConfig: defaultConfig,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Switch device to cam-2
    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true, camera_device_id: "cam-2" },
          committedConfig: defaultConfig,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Now resolve the first (stale) request
    const firstTrack = {
      stop: firstTrackStop,
      kind: "video",
      enabled: true,
      readyState: "live",
    } as unknown as MediaStreamTrack;
    const firstStream = {
      getTracks: () => [firstTrack],
      getVideoTracks: () => [firstTrack],
    } as unknown as MediaStream;

    await act(async () => {
      resolveFirstStream?.(firstStream);
      await Promise.resolve();
    });

    // Stale stream MUST be stopped
    expect(firstTrackStop).toHaveBeenCalled();
    // Active stream MUST NOT be stopped
    expect(secondTrackStop).not.toHaveBeenCalled();

    act(() => {
      root.unmount();
    });
    expect(secondTrackStop).toHaveBeenCalledTimes(1);
  });

  it("stale getUserMedia failure does not stop active preview stream or clear ready state", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    let rejectFirstStream: ((err: unknown) => void) | null = null;
    const secondTrackStop = vi.fn();

    let callCount = 0;
    (navigator.mediaDevices.getUserMedia as any).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        const track = { stop: vi.fn(), kind: "video" } as any;
        return { getTracks: () => [track], getVideoTracks: () => [track] } as any;
      }
      if (callCount === 2) {
        // Request 1: will fail after request 2 succeeds
        return new Promise<MediaStream>((_resolve, reject) => {
          rejectFirstStream = reject;
        });
      }
      // Request 2: succeeds immediately
      const secondTrack = {
        stop: secondTrackStop,
        kind: "video",
        enabled: true,
        readyState: "live",
      } as unknown as MediaStreamTrack;
      return {
        getTracks: () => [secondTrack],
        getVideoTracks: () => [secondTrack],
      } as unknown as MediaStream;
    });

    (navigator.mediaDevices.enumerateDevices as any).mockImplementation(async () => [
      { deviceId: "cam-1", kind: "videoinput", label: "Cam 1" },
      { deviceId: "cam-2", kind: "videoinput", label: "Cam 2" },
    ]);

    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true, camera_device_id: "cam-1" },
          committedConfig: defaultConfig,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Switch device to cam-2 (callCount = 3 succeeds)
    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true, camera_device_id: "cam-2" },
          committedConfig: defaultConfig,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Now reject the first (stale) request with an error
    await act(async () => {
      rejectFirstStream?.(new Error("Camera 1 disconnected"));
      await Promise.resolve();
    });

    // CRITICAL: Request 1's failure MUST NOT stop the active stream!
    expect(secondTrackStop).not.toHaveBeenCalled();

    act(() => {
      root.unmount();
    });
    expect(secondTrackStop).toHaveBeenCalledTimes(1);
  });

  it("disabling camera while getUserMedia is pending stops tracks upon resolution", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);

    let resolveGetUserMedia: ((stream: MediaStream) => void) | null = null;
    const pendingTrackStop = vi.fn();

    let callCount = 0;
    (navigator.mediaDevices.getUserMedia as any).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        const track = { stop: vi.fn(), kind: "video" } as any;
        return { getTracks: () => [track], getVideoTracks: () => [track] } as any;
      }
      return new Promise<MediaStream>((resolve) => {
        resolveGetUserMedia = resolve;
      });
    });

    (navigator.mediaDevices.enumerateDevices as any).mockImplementation(async () => [
      { deviceId: "cam-1", kind: "videoinput", label: "Cam 1" },
    ]);

    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: true, camera_device_id: "cam-1" },
          committedConfig: defaultConfig,
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Disable camera while startPreview is pending
    await act(async () => {
      root.render(
        createElement(VisionTab as any, {
          initialConfig: { ...defaultConfig, camera_enabled: false },
          committedConfig: defaultConfig,
        }),
      );
    });

    // Now resolve the pending getUserMedia
    const lateTrack = {
      stop: pendingTrackStop,
      kind: "video",
      enabled: true,
      readyState: "live",
    } as unknown as MediaStreamTrack;

    const lateStream = {
      getTracks: () => [lateTrack],
      getVideoTracks: () => [lateTrack],
    } as unknown as MediaStream;

    await act(async () => {
      resolveGetUserMedia?.(lateStream);
      await Promise.resolve();
    });

    // CRITICAL: Tracks must be stopped when camera was disabled before resolution!
    expect(pendingTrackStop).toHaveBeenCalledTimes(1);

    act(() => {
      root.unmount();
    });
  });
});
