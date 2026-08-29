// pattern: Functional Core

/** Accepts an event only when the backend echoes this request's correlation id. */
export function isOnboardingTurnEvent(
  requestId: string,
  eventRequestId: string | null | undefined,
): boolean {
  return requestId.length > 0 && eventRequestId === requestId;
}
