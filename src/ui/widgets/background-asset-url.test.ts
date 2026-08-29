// pattern: Functional Core

import { describe, expect, it, vi } from "vitest";

import { mapBackgroundAssetUrl } from "./background-asset-url";

describe("background asset URL mapping", () => {
  it("maps absolute package filesystem paths through the Tauri asset URL adapter", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);

    expect(mapBackgroundAssetUrl("C:/app/characters/kokoro/background.webp", convertFileSrc)).toBe(
      "asset://C:/app/characters/kokoro/background.webp",
    );
    expect(convertFileSrc).toHaveBeenCalledWith("C:/app/characters/kokoro/background.webp");
  });

  it("preserves already browser-addressable background URLs", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);

    expect(mapBackgroundAssetUrl("https://example.test/background.webp", convertFileSrc)).toBe(
      "https://example.test/background.webp",
    );
    expect(mapBackgroundAssetUrl("asset://localhost/background.webp", convertFileSrc)).toBe(
      "asset://localhost/background.webp",
    );
    expect(convertFileSrc).not.toHaveBeenCalled();
  });

  it("preserves public and relative web paths", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);

    expect(mapBackgroundAssetUrl("/backgrounds/default-cozy-room.png", convertFileSrc)).toBe(
      "/backgrounds/default-cozy-room.png",
    );
    expect(mapBackgroundAssetUrl("./backgrounds/custom.webp", convertFileSrc)).toBe(
      "./backgrounds/custom.webp",
    );
    expect(mapBackgroundAssetUrl("../backgrounds/custom.webp", convertFileSrc)).toBe(
      "../backgrounds/custom.webp",
    );
    expect(convertFileSrc).not.toHaveBeenCalled();
  });

  it("returns null when no background is selected", () => {
    expect(mapBackgroundAssetUrl(null, vi.fn())).toBeNull();
  });
});
