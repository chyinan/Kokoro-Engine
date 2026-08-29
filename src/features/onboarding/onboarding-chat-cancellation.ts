// pattern: Imperative Shell

/** The portion of an onboarding chat turn needed to cancel it safely. */
export type PendingOnboardingChat = Readonly<{
  turnId: string | null;
  resolve: (reply: string) => void;
}>;

/**
 * Cancels a pending onboarding turn when the backend has assigned its id and
 * resolves the overlay promise so dismissal cannot leave a hanging request.
 */
export function cancelOnboardingChat(
  pending: PendingOnboardingChat | null,
  cancelChatTurn: (turnId: string, reason?: string) => Promise<void>,
): void {
  if (pending === null) return;

  if (pending.turnId !== null) {
    void cancelChatTurn(pending.turnId, "onboarding_dismissed").catch(() => {
      // Dismissal remains local even if the backend turn already finished.
    });
  }

  pending.resolve("");
}
