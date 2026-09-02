// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VisionConfig } from "@/lib/kokoro-bridge";
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
});
