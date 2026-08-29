// pattern: Imperative Shell

/** The portion of an onboarding chat turn needed to cancel it safely. */
export type PendingOnboardingChat = Readonly<{
  clientRequestId: string;
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
  rememberCancelledRequest?: (clientRequestId: string) => void,
): void {
  if (pending === null) return;

  if (pending.turnId !== null) {
    void cancelChatTurn(pending.turnId, "onboarding_dismissed").catch(() => {
      // Dismissal remains local even if the backend turn already finished.
    });
  } else {
    rememberCancelledRequest?.(pending.clientRequestId);
  }

  pending.resolve("");
}

/** Cancels a turn whose start event arrived after the onboarding UI was dismissed. */
export function cancelDeferredOnboardingChat(
  cancelledRequestIds: Set<string>,
  clientRequestId: string | null | undefined,
  turnId: string,
  cancelChatTurn: (turnId: string, reason?: string) => Promise<void>,
): boolean {
  if (clientRequestId === null || clientRequestId === undefined || !cancelledRequestIds.delete(clientRequestId)) {
    return false;
  }

  void cancelChatTurn(turnId, "onboarding_dismissed").catch(() => {
    // The turn may have finished between dismissal and the delayed start event.
  });
  return true;
}
