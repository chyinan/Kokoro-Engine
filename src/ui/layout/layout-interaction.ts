// pattern: Functional Core

/** Returns whether the background chat surface may receive pointer input. */
export function shouldEnableChatPanel(onboardingOpen: boolean): boolean {
  return !onboardingOpen;
}
