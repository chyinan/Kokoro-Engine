// pattern: Functional Core

import { describe, expect, test } from "vitest";
import {
  continueOnboarding,
  createOnboardingDraft,
  deserializeOnboardingDraft,
  dismissOnboarding,
  onboardingFlowReducer,
  resumeOnboarding,
  serializeOnboardingDraft,
  type OnboardingDraft,
} from "./onboarding-flow";

function configureThroughProvider(): OnboardingDraft {
  let draft = createOnboardingDraft();
  draft = onboardingFlowReducer(draft, { type: "select-language", language: "en" });
  draft = onboardingFlowReducer(draft, { type: "select-character", characterId: "kokoro" });
  return onboardingFlowReducer(draft, {
    type: "configure-provider",
    providerId: "ollama",
  });
}

describe("onboarding flow", () => {
  test("continues from the first incomplete persisted step", () => {
    const draft = configureThroughProvider();

    expect(draft.step).toBe("connection-test");
    expect(continueOnboarding(draft)).toMatchObject({
      step: "connection-test",
      language: "en",
      characterId: "kokoro",
      providerId: "ollama",
    });
  });

  test("retries a failed connection test without losing configured values", () => {
    const configured = configureThroughProvider();
    const failed = onboardingFlowReducer(configured, {
      type: "connection-test-failed",
      error: "provider unavailable",
    });
    const retried = onboardingFlowReducer(failed, { type: "retry" });

    expect(retried).toMatchObject({
      step: "connection-test",
      language: "en",
      characterId: "kokoro",
      providerId: "ollama",
      connectionTest: { status: "idle", error: null },
    });
  });

  test("dismissal preserves the draft and resume returns to its step", () => {
    const configured = configureThroughProvider();
    const dismissed = dismissOnboarding(configured);
    const resumed = resumeOnboarding(dismissed);

    expect(dismissed).toMatchObject({
      dismissed: true,
      language: "en",
      characterId: "kokoro",
      providerId: "ollama",
    });
    expect(resumed).toMatchObject({
      dismissed: false,
      step: "connection-test",
      language: "en",
      characterId: "kokoro",
      providerId: "ollama",
    });
  });

  test("round-trips a draft through its serializable representation", () => {
    const draft = configureThroughProvider();
    const restored = deserializeOnboardingDraft(serializeOnboardingDraft(draft));

    expect(restored).toEqual(draft);
  });

  test("does not complete until the first chat reply succeeds", () => {
    let draft = configureThroughProvider();
    draft = onboardingFlowReducer(draft, { type: "connection-test-succeeded" });
    draft = onboardingFlowReducer(draft, { type: "chat-started" });
    draft = onboardingFlowReducer(draft, { type: "chat-failed", error: "request failed" });

    expect(draft.completed).toBe(false);
    expect(draft.step).toBe("chat");
    expect(draft.chat.error).toBe("request failed");

    const completed = onboardingFlowReducer(draft, {
      type: "first-reply-succeeded",
      reply: "Hello!",
    });

    expect(completed).toMatchObject({
      completed: true,
      dismissed: false,
      step: "chat",
      chat: { status: "success", reply: "Hello!", error: null },
    });
  });
});
