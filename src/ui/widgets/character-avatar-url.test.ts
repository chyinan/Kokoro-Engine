// pattern: Functional Core

import { describe, expect, it, vi } from "vitest";

import { mapCharacterAvatarUrl } from "./character-avatar-url";

describe("character avatar URL mapping", () => {
  it("keeps managed character protocol references browser-addressable", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);

    expect(mapCharacterAvatarUrl("character-instance-resource://custom/avatar.png", convertFileSrc))
      .toBe("character-instance-resource://custom/avatar.png");
    expect(convertFileSrc).not.toHaveBeenCalled();
  });

  it("converts filesystem avatar paths at the Tauri boundary", () => {
    const convertFileSrc = vi.fn((path: string) => `asset://${path}`);

    expect(mapCharacterAvatarUrl("C:/avatars/custom.png", convertFileSrc))
      .toBe("asset://C:/avatars/custom.png");
    expect(convertFileSrc).toHaveBeenCalledWith("C:/avatars/custom.png");
  });
});
