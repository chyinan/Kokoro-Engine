// pattern: Functional Core

import { describe, expect, it } from "vitest";

import { shouldEnableChatPanel } from "./layout-interaction";

describe("onboarding layout interaction", () => {
  it("blocks background chat interaction while onboarding owns the turn", () => {
    expect(shouldEnableChatPanel(true)).toBe(false);
    expect(shouldEnableChatPanel(false)).toBe(true);
  });
});
