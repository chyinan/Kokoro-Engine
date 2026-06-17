import { describe, expect, it, vi } from "vitest";
import {
  createModActionDispatcher,
  getModActionFromEvent,
} from "./dispatcher";

describe("mod action dispatcher", () => {
  it("extracts action envelope from CustomEvent detail", () => {
    const event = new CustomEvent("kokoro:mod-action", {
      detail: { action: "close_settings", data: { source: "test" } },
    });

    expect(getModActionFromEvent(event)).toEqual({
      action: "close_settings",
      data: { source: "test" },
    });
  });

  it("ignores malformed events", () => {
    expect(getModActionFromEvent(new Event("kokoro:mod-action"))).toBeNull();
    expect(getModActionFromEvent(new CustomEvent("kokoro:mod-action", { detail: {} }))).toBeNull();
  });

  it("dispatches registered handler and ignores unknown action", async () => {
    const close = vi.fn();
    const dispatcher = createModActionDispatcher({
      close_settings: close,
    });

    await dispatcher({ action: "close_settings" });
    await dispatcher({ action: "unknown_action" });

    expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledWith({ action: "close_settings" });
  });

  it("reports whether an action was handled", async () => {
    const dispatcher = createModActionDispatcher({
      close_settings: () => undefined,
    });

    await expect(dispatcher({ action: "close_settings" })).resolves.toBe(true);
    await expect(dispatcher({ action: "unknown_action" })).resolves.toBe(false);
  });
});
