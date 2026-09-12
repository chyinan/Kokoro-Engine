// @vitest-environment jsdom
// pattern: Imperative Shell
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useBackgroundSlideshow } from "./useBackgroundSlideshow";
import { db } from "../../lib/db";

vi.mock("../../lib/db", () => ({ db: { getAllImages: vi.fn(), deleteImage: vi.fn(), addImage: vi.fn(), clearAll: vi.fn() } }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("background image lifecycle", () => {
  let root: Root;
  let container: HTMLDivElement;
  let state: ReturnType<typeof useBackgroundSlideshow>;
  function Harness() { state = useBackgroundSlideshow(); return null; }

  beforeEach(() => {
    localStorage.clear();
    vi.mocked(db.getAllImages).mockResolvedValue([
      { id: 1, blob: new Blob(["A"]), created: 1 },
      { id: 2, blob: new Blob(["B"]), created: 2 },
    ]);
    vi.mocked(db.deleteImage).mockResolvedValue(undefined);
    let sequence = 0;
    vi.stubGlobal("URL", class extends URL {
      static createObjectURL = vi.fn(() => `blob:image-${++sequence}`);
      static revokeObjectURL = vi.fn();
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it("selects a remaining live image after deleting the displayed image", async () => {
    await act(async () => root.render(createElement(Harness)));
    expect(state.currentUrl).toBe("blob:image-1");
    await act(async () => state.removeImage(0));
    expect(db.deleteImage).toHaveBeenCalledWith(1);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:image-1");
    expect(state.images).toEqual(["blob:image-2"]);
    expect(state.currentUrl).toBe("blob:image-2");
  });

  it("releases all loaded object URLs when the hook unmounts", async () => {
    await act(async () => root.render(createElement(Harness)));
    await act(async () => root.render(null));
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:image-1");
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:image-2");
  });
});
