// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EmotionModelPanel } from "./EmotionModelPanel";
import * as bridge from "../../../lib/kokoro-bridge";
import type { EmotionModelStatus, EmotionInferenceResult } from "../../../lib/kokoro-bridge";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { count?: number; defaultValue?: string }) => {
      if (opts?.defaultValue) return opts.defaultValue;
      return key;
    },
  }),
}));

const mockUninstalledStatus: EmotionModelStatus = {
  installed: false,
  is_active: false,
  repo_id: "Johnson8187/Chinese-Emotion-Small",
  download_url: "https://huggingface.co/Johnson8187/Chinese-Emotion-Small",
  install_dir: "C:\\mock\\models\\Chinese-Emotion-Small",
  model_path: "C:\\mock\\models\\Chinese-Emotion-Small\\model.onnx",
  required_files: ["model.onnx", "config.json", "tokenizer.json"],
  missing_files: ["model.onnx", "config.json", "tokenizer.json"],
  memory_bytes: null,
};

const mockInstalledStatus: EmotionModelStatus = {
  installed: true,
  is_active: true,
  repo_id: "Johnson8187/Chinese-Emotion-Small",
  download_url: "https://huggingface.co/Johnson8187/Chinese-Emotion-Small",
  install_dir: "C:\\mock\\models\\Chinese-Emotion-Small",
  model_path: "C:\\mock\\models\\Chinese-Emotion-Small\\model.onnx",
  required_files: ["model.onnx", "config.json", "tokenizer.json"],
  missing_files: [],
  memory_bytes: 45000000,
};

const mockInferenceResult: EmotionInferenceResult = {
  dominant_emotion: "happy",
  label_zh: "開心語調",
  confidence: 0.95,
  probabilities: [
    { label: "happy", label_zh: "開心語調", score: 0.95 },
    { label: "neutral", label_zh: "平淡語氣", score: 0.05 },
  ],
  mapped_cue: "happy",
  latency_ms: 12.5,
};

describe("EmotionModelPanel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    vi.spyOn(bridge, "getEmotionModelStatus").mockResolvedValue(mockUninstalledStatus);
    vi.spyOn(bridge, "downloadEmotionModel").mockResolvedValue(mockInstalledStatus);
    vi.spyOn(bridge, "uninstallEmotionModel").mockResolvedValue(mockUninstalledStatus);
    vi.spyOn(bridge, "toggleEmotionModel").mockResolvedValue(mockInstalledStatus);
    vi.spyOn(bridge, "inferEmotion").mockResolvedValue(mockInferenceResult);
    vi.spyOn(bridge, "onEmotionModelProgress").mockResolvedValue(() => {});
    vi.spyOn(bridge, "openEmotionModelDir").mockResolvedValue("C:\\mock\\models\\Chinese-Emotion-Small");
    vi.spyOn(bridge, "importEmotionModelPackage").mockResolvedValue(mockInstalledStatus);
  });

  afterEach(async () => {
    await act(async () => {
      root.unmount();
    });
    container.remove();
    vi.restoreAllMocks();
  });

  it("renders uninstalled state with download button, manual import, and open dir button", async () => {
    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(bridge.getEmotionModelStatus).toHaveBeenCalled();
    expect(container.textContent).toContain("Chinese-Emotion-Small");
    expect(container.textContent).toContain("未安装");
    expect(container.textContent).toContain("下载模型");
    expect(container.textContent).toContain("手动导入包体");
    expect(container.textContent).toContain("打开目录");
  });

  it("triggers openEmotionModelDir when open directory button is clicked", async () => {
    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const openDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("打开目录")
    );
    expect(openDirBtn).toBeDefined();

    await act(async () => {
      openDirBtn?.click();
    });

    expect(bridge.openEmotionModelDir).toHaveBeenCalled();
  });

  it("renders rich error card with offline actions when download fails", async () => {
    vi.spyOn(bridge, "downloadEmotionModel").mockRejectedValue(new Error("HTTP 404 Not Found"));

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const downloadBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("下载模型")
    );
    expect(downloadBtn).toBeDefined();

    await act(async () => {
      downloadBtn?.click();
    });

    expect(container.textContent).toContain("HTTP 404 Not Found");
    expect(container.textContent).toContain("导入已下载的离线包 (.zip)");
    expect(container.textContent).toContain("打开存储目录");
  });

  it("renders installed state with active toggle, uninstall button, and playground", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue(mockInstalledStatus);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("运行中");
    expect(container.textContent).toContain("卸载模型");
    expect(container.textContent).toContain("情感推理试炼场");
    expect(container.textContent).toContain("测算情感");
  });

  it("triggers inferEmotion and shows inference result when test button clicked", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue(mockInstalledStatus);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const testButton = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("测算情感")
    );
    expect(testButton).toBeDefined();

    await act(async () => {
      testButton?.click();
    });

    expect(bridge.inferEmotion).toHaveBeenCalled();
    expect(container.textContent).toContain("開心語調");
  });
});

