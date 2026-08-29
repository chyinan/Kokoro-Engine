// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";

import {
  cancelDeferredOnboardingChat,
  cancelOnboardingChat,
  type PendingOnboardingChat,
} from "./onboarding-chat-cancellation";

describe("cancelOnboardingChat", () => {
  it("propagates dismissal to the backend turn before resolving the pending chat", () => {
    const cancelChatTurn = vi.fn().mockResolvedValue(undefined);
    const resolve = vi.fn();
    const pending: PendingOnboardingChat = {
      clientRequestId: "request-42",
      turnId: "turn-42",
      resolve,
    };

    cancelOnboardingChat(pending, cancelChatTurn);

    expect(cancelChatTurn).toHaveBeenCalledWith("turn-42", "onboarding_dismissed");
    expect(resolve).toHaveBeenCalledWith("");
  });

  it("does not call the backend when the turn id has not arrived yet", () => {
    const cancelChatTurn = vi.fn().mockResolvedValue(undefined);
    const resolve = vi.fn();
    const rememberCancelledRequest = vi.fn();

    cancelOnboardingChat(
      { clientRequestId: "request-43", turnId: null, resolve },
      cancelChatTurn,
      rememberCancelledRequest,
    );

    expect(cancelChatTurn).not.toHaveBeenCalled();
    expect(rememberCancelledRequest).toHaveBeenCalledWith("request-43");
    expect(resolve).toHaveBeenCalledWith("");
  });

  it("cancels a correlated backend turn when its start event arrives late", () => {
    const cancelChatTurn = vi.fn().mockResolvedValue(undefined);
    const cancelledRequestIds = new Set(["request-43"]);

    expect(
      cancelDeferredOnboardingChat(
        cancelledRequestIds,
        "request-43",
        "turn-43",
        cancelChatTurn,
      ),
    ).toBe(true);
    expect(cancelChatTurn).toHaveBeenCalledWith("turn-43", "onboarding_dismissed");
    expect(cancelledRequestIds.has("request-43")).toBe(false);
  });
});
