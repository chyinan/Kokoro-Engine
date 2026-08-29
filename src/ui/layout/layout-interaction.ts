// pattern: Functional Core

/** Returns whether the background chat surface may receive pointer input. */
export function shouldEnableChatPanel(onboardingOpen: boolean): boolean {
  return !onboardingOpen;
}

/** Attributes applied to the background chat surface while onboarding owns focus. */
export function getChatPanelInteractionProps(interactionDisabled: boolean): Readonly<{
  "aria-disabled": boolean;
  "aria-hidden": boolean | undefined;
  tabIndex: number | undefined;
}> {
  return interactionDisabled
    ? { "aria-disabled": true, "aria-hidden": true, tabIndex: -1 }
    : { "aria-disabled": false, "aria-hidden": undefined, tabIndex: undefined };
}
