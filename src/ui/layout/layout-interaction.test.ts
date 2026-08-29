// pattern: Functional Core

import { describe, expect, it } from "vitest";

import { getChatPanelInteractionProps, shouldEnableChatPanel } from "./layout-interaction";

describe("onboarding layout interaction", () => {
  it("blocks background chat interaction while onboarding owns the turn", () => {
    expect(shouldEnableChatPanel(true)).toBe(false);
    expect(shouldEnableChatPanel(false)).toBe(true);
  });

  it("marks the background chat surface out of the keyboard interaction order", () => {
    expect(getChatPanelInteractionProps(true)).toMatchObject({
      "aria-disabled": true,
      "aria-hidden": true,
      tabIndex: -1,
    });
    expect(getChatPanelInteractionProps(false)).toMatchObject({
      "aria-disabled": false,
      "aria-hidden": undefined,
      tabIndex: undefined,
    });
  });
});
