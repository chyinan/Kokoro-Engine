// pattern: Functional Core

/** Accepts an event only when the backend echoes this request's correlation id. */
export function isOnboardingTurnEvent(
  requestId: string,
  eventRequestId: string | null | undefined,
): boolean {
  return requestId.length > 0 && eventRequestId === requestId;
}

/**
 * Accepts either a matching terminal lifecycle event or a stream response
 * fallback. Stream responses do not carry the backend turn id, so request
 * correlation is the only safe identity available for that fallback path.
 */
export function canSettleOnboardingTurn(
  requestId: string,
  pendingTurnId: string | null,
  eventRequestId: string | null | undefined,
  eventTurnId: string | undefined,
): boolean {
  if (!isOnboardingTurnEvent(requestId, eventRequestId)) return false;
  if (eventTurnId === undefined) return true;
  return pendingTurnId !== null && eventTurnId === pendingTurnId;
}
