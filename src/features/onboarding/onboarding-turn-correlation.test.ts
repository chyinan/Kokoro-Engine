// pattern: Functional Core

import { describe, expect, it } from "vitest";

import { canSettleOnboardingTurn, isOnboardingTurnEvent } from "./onboarding-turn-correlation";

describe("onboarding turn correlation", () => {
  it("accepts only events carrying the request correlation id", () => {
    expect(isOnboardingTurnEvent("onboarding-1", "onboarding-1")).toBe(true);
    expect(isOnboardingTurnEvent("onboarding-1", "normal-chat")).toBe(false);
    expect(isOnboardingTurnEvent("onboarding-1", null)).toBe(false);
    expect(isOnboardingTurnEvent("onboarding-1", undefined)).toBe(false);
  });

  it("accepts a matching finish event only when its turn id matches", () => {
    expect(canSettleOnboardingTurn("onboarding-1", "turn-1", "onboarding-1", "turn-1")).toBe(true);
    expect(canSettleOnboardingTurn("onboarding-1", "turn-1", "onboarding-1", "other-turn")).toBe(false);
    expect(canSettleOnboardingTurn("onboarding-1", "turn-1", "other-request", "turn-1")).toBe(false);
  });

  it("accepts a matching stream response fallback without a turn id", () => {
    expect(canSettleOnboardingTurn("onboarding-1", null, "onboarding-1", undefined)).toBe(true);
    expect(canSettleOnboardingTurn("onboarding-1", "turn-1", "other-request", undefined)).toBe(false);
  });
});
