// pattern: Functional Core

import { describe, expect, it, vi } from "vitest";

import { mapLive2dModelUrl } from "./live2d-model-url";

describe("Live2D model URL mapping", () => {
  it("converts package-owned absolute paths for the webview", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);
    const live2dUrl = vi.fn((path: string) => `live2d://${path}`);

    expect(mapLive2dModelUrl(
      "C:/app/characters/kokoro/models/kokoro.model3.json",
      "package",
      convertFileSrc,
      live2dUrl,
    )).toBe("asset://C:/app/characters/kokoro/models/kokoro.model3.json");
    expect(live2dUrl).not.toHaveBeenCalled();
  });

  it("keeps library IDs on the live2d protocol", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);
    const live2dUrl = vi.fn((path: string) => `live2d://${path}`);

    expect(mapLive2dModelUrl("hiyori/hiyori.model3.json", "user", convertFileSrc, live2dUrl))
      .toBe("live2d://hiyori/hiyori.model3.json");
    expect(convertFileSrc).not.toHaveBeenCalled();
  });
});
