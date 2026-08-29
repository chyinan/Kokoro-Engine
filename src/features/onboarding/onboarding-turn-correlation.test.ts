// pattern: Functional Core

import { describe, expect, it } from "vitest";

import { isOnboardingTurnEvent } from "./onboarding-turn-correlation";

describe("onboarding turn correlation", () => {
  it("accepts only events carrying the request correlation id", () => {
    expect(isOnboardingTurnEvent("onboarding-1", "onboarding-1")).toBe(true);
    expect(isOnboardingTurnEvent("onboarding-1", "normal-chat")).toBe(false);
    expect(isOnboardingTurnEvent("onboarding-1", null)).toBe(false);
    expect(isOnboardingTurnEvent("onboarding-1", undefined)).toBe(false);
  });
});
