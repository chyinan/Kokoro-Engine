// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EmotionModelPanel } from "./EmotionModelPanel";
import * as bridge from "../../../lib/kokoro-bridge";
import type { EmotionModelStatus, EmotionInferenceResult } from "../../../lib/kokoro-bridge";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const mockOpenDialog = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: any[]) => mockOpenDialog(...args),
}));

import zhLocale from "../../locales/zh.json";
import enLocale from "../../locales/en.json";

function getNestedValue(obj: any, path: string): string | undefined {
  const parts = path.split(".");
  let current = obj;
  for (const part of parts) {
    if (current == null) return undefined;
    current = current[part];
  }
  return typeof current === "string" ? current : undefined;
}

let currentLocale = "zh";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, any>) => {
      const activeDict = currentLocale === "en" ? enLocale : zhLocale;
      let str = getNestedValue(activeDict, key);
      if (str === undefined) {
        if (opts?.defaultValue !== undefined) return opts.defaultValue;
        return key;
      }
      if (opts) {
        for (const [k, v] of Object.entries(opts)) {
          str = str.replace(new RegExp(`{{${k}}}`, "g"), String(v));
        }
      }
      return str;
    },
    i18n: {
      language: currentLocale,
      changeLanguage: vi.fn((lang: string) => {
        currentLocale = lang;
      }),
    },
  }),
}));

const mockUninstalledStatus: EmotionModelStatus = {
  installed: false,
  is_active: false,
  is_valid: false,
  error_message: null,
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
  is_valid: true,
  error_message: null,
  repo_id: "Johnson8187/Chinese-Emotion-Small",
  download_url: "https://huggingface.co/Johnson8187/Chinese-Emotion-Small",
  install_dir: "C:\\mock\\models\\Chinese-Emotion-Small",
  model_path: "C:\\mock\\models\\Chinese-Emotion-Small\\model.onnx",
  required_files: ["model.onnx", "config.json", "tokenizer.json"],
  missing_files: [],
  memory_bytes: 45000000,
};

const mockCorruptedStatus: EmotionModelStatus = {
  installed: true,
  is_active: false,
  is_valid: false,
  error_message: "model.onnx 尺寸过小 (0.0 MB)，疑似下载中断或损坏",
  repo_id: "Johnson8187/Chinese-Emotion-Small",
  download_url: "https://huggingface.co/Johnson8187/Chinese-Emotion-Small",
  install_dir: "C:\\mock\\models\\Chinese-Emotion-Small",
  model_path: "C:\\mock\\models\\Chinese-Emotion-Small\\model.onnx",
  required_files: ["model.onnx", "config.json", "tokenizer.json"],
  missing_files: [],
  memory_bytes: null,
};

const mockInferenceResult: EmotionInferenceResult = {
  dominant_emotion: "happy",
  dominant_emotion_id: "happy",
  label_zh: "開心語調",
  confidence: 0.95,
  probabilities: [
    { id: "happy", label: "happy", label_zh: "開心語調", score: 0.95 },
    { id: "neutral", label: "neutral", label_zh: "平淡語氣", score: 0.05 },
  ],
  mapped_cue: "happy",
  latency_ms: 12.5,
};

describe("EmotionModelPanel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    currentLocale = "zh";
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mockOpenDialog.mockReset();
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

  it("renders uninstalled state with download button, manual import, directory import, and open dir button", async () => {
    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(bridge.getEmotionModelStatus).toHaveBeenCalled();
    expect(container.textContent).toContain("Chinese-Emotion-Small");
    expect(container.textContent).toContain("未安装");
    expect(container.textContent).toContain("下载模型");
    expect(container.textContent).toContain("手动导入包体");
    expect(container.textContent).toContain("导入模型目录");
    expect(container.textContent).toContain("打开存储目录");
  });

  it("triggers openEmotionModelDir when open directory button is clicked", async () => {
    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const openDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("打开存储目录")
    );
    expect(openDirBtn).toBeDefined();

    await act(async () => {
      openDirBtn?.click();
    });

    expect(bridge.openEmotionModelDir).toHaveBeenCalled();
  });

  it("triggers manual file import when import package button is clicked", async () => {
    mockOpenDialog.mockResolvedValue("C:\\mock\\downloads\\model.zip");

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const importFileBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("手动导入包体")
    );
    expect(importFileBtn).toBeDefined();

    await act(async () => {
      importFileBtn?.click();
    });

    expect(mockOpenDialog).toHaveBeenCalledWith(
      expect.objectContaining({
        directory: false,
        multiple: false,
      })
    );
    expect(bridge.importEmotionModelPackage).toHaveBeenCalledWith("C:\\mock\\downloads\\model.zip");
  });

  it("triggers manual directory import when import directory button is clicked", async () => {
    mockOpenDialog.mockResolvedValue("C:\\mock\\models\\Chinese-Emotion-Small");

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const importDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("导入模型目录")
    );
    expect(importDirBtn).toBeDefined();

    await act(async () => {
      importDirBtn?.click();
    });

    expect(mockOpenDialog).toHaveBeenCalledWith(
      expect.objectContaining({
        directory: true,
        multiple: false,
      })
    );
    expect(bridge.importEmotionModelPackage).toHaveBeenCalledWith("C:\\mock\\models\\Chinese-Emotion-Small");
  });

  it("handles cancelled directory import gracefully without error", async () => {
    mockOpenDialog.mockResolvedValue(null);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const importDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("导入模型目录")
    );
    expect(importDirBtn).toBeDefined();

    await act(async () => {
      importDirBtn?.click();
    });

    expect(mockOpenDialog).toHaveBeenCalled();
    expect(bridge.importEmotionModelPackage).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Manual import failed");
  });

  it("renders error banner when directory import fails", async () => {
    mockOpenDialog.mockResolvedValue("C:\\mock\\models\\empty_dir");
    vi.mocked(bridge.importEmotionModelPackage).mockRejectedValue(
      new Error("目录 C:\\mock\\models\\empty_dir 中缺少必需模型文件: model.onnx")
    );

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const importDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("导入模型目录")
    );

    await act(async () => {
      importDirBtn?.click();
    });

    expect(container.textContent).toContain("缺少必需模型文件: model.onnx");
    expect(container.textContent).toContain("导入解压目录");
    expect(container.textContent).toContain("导入已下载的离线包 (.zip)");
  });

  it("triggers directory import from offline error card", async () => {
    vi.spyOn(bridge, "downloadEmotionModel").mockRejectedValue(new Error("HTTP 404 Not Found"));
    mockOpenDialog.mockResolvedValue("C:\\offline\\extracted-model");

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    const downloadBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("下载模型")
    );
    await act(async () => {
      downloadBtn?.click();
    });

    expect(container.textContent).toContain("HTTP 404 Not Found");
    expect(container.textContent).toContain("导入解压目录");

    const offlineImportDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("导入解压目录")
    );
    expect(offlineImportDirBtn).toBeDefined();

    await act(async () => {
      offlineImportDirBtn?.click();
    });

    expect(mockOpenDialog).toHaveBeenCalledWith(
      expect.objectContaining({
        directory: true,
        multiple: false,
      })
    );
    expect(bridge.importEmotionModelPackage).toHaveBeenCalledWith("C:\\offline\\extracted-model");
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
    expect(container.textContent).toContain("导入解压目录");
    expect(container.textContent).toContain("打开存储目录");
  });

  it("renders human-readable message instead of [object Object] when download fails with structured error", async () => {
    vi.spyOn(bridge, "downloadEmotionModel").mockRejectedValue({
      code: "IO_ERROR",
      message: "磁盘空间不足，无法写入模型文件",
    });

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

    expect(container.textContent).not.toContain("[object Object]");
    expect(container.textContent).toContain("磁盘空间不足，无法写入模型文件");
  });

  it("renders installed state with active toggle, reimport buttons, uninstall button, and playground", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue(mockInstalledStatus);
    mockOpenDialog.mockResolvedValue("C:\\models\\new-folder");

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("运行中");
    expect(container.textContent).toContain("重新导入文件");
    expect(container.textContent).toContain("重新导入目录");
    expect(container.textContent).toContain("卸载模型");
    expect(container.textContent).toContain("情感推理试炼场");
    expect(container.textContent).toContain("测算情感");

    const reimportDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("重新导入目录")
    );
    expect(reimportDirBtn).toBeDefined();

    await act(async () => {
      reimportDirBtn?.click();
    });

    expect(mockOpenDialog).toHaveBeenCalledWith(
      expect.objectContaining({
        directory: true,
        multiple: false,
      })
    );
    expect(bridge.importEmotionModelPackage).toHaveBeenCalledWith("C:\\models\\new-folder");
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
    // In Chinese locale, translates stable emotion ID "happy" via zh.json rather than backend label_zh "開心語調"
    expect(container.textContent).toContain("开心语调");
    expect(container.textContent).not.toContain("開心語調");
    expect(container.textContent).toContain("平淡语气");
    expect(container.textContent).toContain("置信度");
    expect(container.textContent).toContain("耗时");
    expect(container.textContent).toContain("触发表情: happy");
  });

  it("renders emotion labels and UI text in English when locale is en", async () => {
    currentLocale = "en";
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue(mockInstalledStatus);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("Emotion Inference Playground");
    const testButton = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Infer Emotion")
    );
    expect(testButton).toBeDefined();

    await act(async () => {
      testButton?.click();
    });

    expect(bridge.inferEmotion).toHaveBeenCalled();
    // Verifies frontend maps stable emotion IDs to English translations
    expect(container.textContent).toContain("Happy");
    expect(container.textContent).toContain("Neutral");
    expect(container.textContent).toContain("Confidence");
    expect(container.textContent).toContain("Latency");
    expect(container.textContent).toContain("Trigger Cue: happy");
    // Verifies no Chinese leak occurs in English locale
    expect(container.textContent).not.toContain("開心語調");
    expect(container.textContent).not.toContain("开心语调");
    expect(container.textContent).not.toContain("置信度");
    expect(container.textContent).not.toContain("耗时");
  });

  it("renders corrupted model state with repair button, recovery actions, and hides playground", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue(mockCorruptedStatus);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("模型损坏");
    expect(container.textContent).toContain("model.onnx 尺寸过小");

    const repairBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("修复模型")
    );
    expect(repairBtn).toBeDefined();

    // Verify recovery actions are available
    const reimportFileBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.getAttribute("title")?.includes("重新导入文件") || b.textContent?.includes("重新导入文件")
    );
    expect(reimportFileBtn).toBeDefined();

    const reimportDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.getAttribute("title")?.includes("重新导入目录") || b.textContent?.includes("重新导入目录")
    );
    expect(reimportDirBtn).toBeDefined();

    const openDirBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("打开存储目录")
    );
    expect(openDirBtn).toBeDefined();

    const uninstallBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("卸载模型")
    );
    expect(uninstallBtn).toBeDefined();

    // Crucial: Playground must NOT be rendered when model is corrupted / invalid
    expect(container.textContent).not.toContain("情感测算演练场");
    expect(container.textContent).not.toContain("测算情感");

    await act(async () => {
      repairBtn?.click();
    });

    expect(bridge.downloadEmotionModel).toHaveBeenCalled();
  });

  it("handles corrupted model defensively even if installed is false when error_message is present", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue({
      ...mockCorruptedStatus,
      installed: false,
    });

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("模型损坏");
    expect(container.textContent).toContain("model.onnx 尺寸过小");

    const repairBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("修复模型")
    );
    expect(repairBtn).toBeDefined();

    expect(container.textContent).not.toContain("情感测算演练场");
    expect(container.textContent).not.toContain("测算情感");
  });

  it("renders detected local model button and triggers import when clicked", async () => {
    vi.mocked(bridge.getEmotionModelStatus).mockResolvedValue({
      ...mockUninstalledStatus,
      local_cache_available: true,
      local_cache_path: "D:\\Kokoro-Engine\\scratch\\emotion_onnx_export",
    });
    vi.mocked(bridge.importEmotionModelPackage).mockResolvedValue(mockInstalledStatus);

    await act(async () => {
      root.render(createElement(EmotionModelPanel));
    });

    expect(container.textContent).toContain("载入检测到的本地模型");

    const loadLocalBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("载入检测到的本地模型")
    );
    expect(loadLocalBtn).toBeDefined();

    await act(async () => {
      loadLocalBtn?.click();
    });

    expect(bridge.importEmotionModelPackage).toHaveBeenCalledWith("D:\\Kokoro-Engine\\scratch\\emotion_onnx_export");
  });
});

