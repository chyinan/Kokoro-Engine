// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";

import { cancelOnboardingChat, type PendingOnboardingChat } from "./onboarding-chat-cancellation";

describe("cancelOnboardingChat", () => {
  it("propagates dismissal to the backend turn before resolving the pending chat", () => {
    const cancelChatTurn = vi.fn().mockResolvedValue(undefined);
    const resolve = vi.fn();
    const pending: PendingOnboardingChat = {
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

    cancelOnboardingChat({ turnId: null, resolve }, cancelChatTurn);

    expect(cancelChatTurn).not.toHaveBeenCalled();
    expect(resolve).toHaveBeenCalledWith("");
  });
});
