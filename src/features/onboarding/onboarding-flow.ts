// pattern: Functional Core

/** The ordered setup stages required before a first successful chat reply. */
export type OnboardingStep =
  | "language"
  | "character"
  | "provider"
  | "connection-test"
  | "chat";

type ConnectionTestStatus = "idle" | "pending" | "success" | "error";
type ChatStatus = "idle" | "pending" | "success" | "error";

type OnboardingConnectionTest = Readonly<{
  status: ConnectionTestStatus;
  error: string | null;
}>;

type OnboardingChat = Readonly<{
  status: ChatStatus;
  reply: string | null;
  error: string | null;
}>;

/** Serializable state persisted while a user works through onboarding. */
export type OnboardingDraft = Readonly<{
  version: 1;
  step: OnboardingStep;
  language: string | null;
  characterId: string | null;
  providerId: string | null;
  connectionTest: OnboardingConnectionTest;
  chat: OnboardingChat;
  dismissed: boolean;
  completed: boolean;
}>;

export type OnboardingFlowEvent =
  | Readonly<{ type: "select-language"; language: string }>
  | Readonly<{ type: "select-character"; characterId: string }>
  | Readonly<{ type: "configure-provider"; providerId: string }>
  | Readonly<{ type: "connection-test-started" }>
  | Readonly<{ type: "connection-test-succeeded" }>
  | Readonly<{ type: "connection-test-failed"; error: string }>
  | Readonly<{ type: "chat-started" }>
  | Readonly<{ type: "chat-failed"; error: string }>
  | Readonly<{ type: "first-reply-succeeded"; reply: string }>
  | Readonly<{ type: "retry" }>
  | Readonly<{ type: "dismiss" }>
  | Readonly<{ type: "resume" }>;

const ONBOARDING_VERSION = 1 as const;

function nonEmpty(value: string): string | null {
  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
}

function withStep(draft: OnboardingDraft, step: OnboardingStep): OnboardingDraft {
  return { ...draft, step };
}

/** Creates a fresh, unconfigured onboarding draft. */
export function createOnboardingDraft(): OnboardingDraft {
  return {
    version: ONBOARDING_VERSION,
    step: "language",
    language: null,
    characterId: null,
    providerId: null,
    connectionTest: { status: "idle", error: null },
    chat: { status: "idle", reply: null, error: null },
    dismissed: false,
    completed: false,
  };
}

/**
 * Returns the first stage that still needs user action. Existing selections
 * and successful checks are never discarded while calculating continuation.
 */
export function continueOnboarding(draft: Readonly<OnboardingDraft>): OnboardingDraft {
  if (draft.completed) {
    return withStep(draft, "chat");
  }
  if (draft.language === null) {
    return withStep(draft, "language");
  }
  if (draft.characterId === null) {
    return withStep(draft, "character");
  }
  if (draft.providerId === null) {
    return withStep(draft, "provider");
  }
  if (draft.connectionTest.status !== "success") {
    return withStep(draft, "connection-test");
  }
  return withStep(draft, "chat");
}

/** Advances the serializable onboarding state in response to a user/system event. */
export function onboardingFlowReducer(
  draft: Readonly<OnboardingDraft>,
  event: OnboardingFlowEvent,
): OnboardingDraft {
  switch (event.type) {
    case "select-language": {
      const language = nonEmpty(event.language);
      return language === null ? draft : withStep({ ...draft, language }, "character");
    }
    case "select-character": {
      const characterId = nonEmpty(event.characterId);
      if (characterId === null) return draft;
      const next = { ...draft, characterId };
      return withStep(next, continueOnboarding(next).step);
    }
    case "configure-provider": {
      const providerId = nonEmpty(event.providerId);
      if (providerId === null) return draft;
      return withStep(
        {
          ...draft,
          providerId,
          connectionTest: { status: "idle", error: null },
          chat: { status: "idle", reply: null, error: null },
          completed: false,
        },
        "connection-test",
      );
    }
    case "connection-test-started":
      return withStep(
        { ...draft, connectionTest: { status: "pending", error: null } },
        "connection-test",
      );
    case "connection-test-succeeded":
      if (draft.providerId === null) return continueOnboarding(draft);
      return withStep(
        { ...draft, connectionTest: { status: "success", error: null } },
        "chat",
      );
    case "connection-test-failed":
      return withStep(
        {
          ...draft,
          connectionTest: { status: "error", error: event.error.trim() || "connection test failed" },
          completed: false,
        },
        "connection-test",
      );
    case "chat-started":
      if (draft.connectionTest.status !== "success") return continueOnboarding(draft);
      return withStep(
        { ...draft, chat: { status: "pending", reply: null, error: null }, completed: false },
        "chat",
      );
    case "chat-failed":
      return withStep(
        {
          ...draft,
          chat: { status: "error", reply: null, error: event.error.trim() || "chat request failed" },
          completed: false,
        },
        "chat",
      );
    case "first-reply-succeeded":
      if (draft.connectionTest.status !== "success") return continueOnboarding(draft);
      return withStep(
        {
          ...draft,
          chat: { status: "success", reply: event.reply, error: null },
          dismissed: false,
          completed: true,
        },
        "chat",
      );
    case "retry": {
      if (draft.connectionTest.status === "error") {
        return withStep(
          { ...draft, connectionTest: { status: "idle", error: null }, completed: false },
          "connection-test",
        );
      }
      if (draft.chat.status === "error") {
        return withStep(
          { ...draft, chat: { status: "idle", reply: null, error: null }, completed: false },
          "chat",
        );
      }
      return continueOnboarding(draft);
    }
    case "dismiss":
      return { ...draft, dismissed: true };
    case "resume":
      return { ...continueOnboarding({ ...draft, dismissed: false }), dismissed: false };
  }
}

/** Marks a draft as dismissed without changing any configured value. */
export function dismissOnboarding(draft: Readonly<OnboardingDraft>): OnboardingDraft {
  return onboardingFlowReducer(draft, { type: "dismiss" });
}

/** Resumes a dismissed draft at its first incomplete stage. */
export function resumeOnboarding(draft: Readonly<OnboardingDraft>): OnboardingDraft {
  return onboardingFlowReducer(draft, { type: "resume" });
}

/** Serializes a validated draft for local persistence. */
export function serializeOnboardingDraft(draft: Readonly<OnboardingDraft>): string {
  return JSON.stringify(draft);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isConnectionTest(value: unknown): value is OnboardingConnectionTest {
  if (!isRecord(value)) return false;
  const status = value.status;
  return (
    (status === "idle" || status === "pending" || status === "success" || status === "error") &&
    (typeof value.error === "string" || value.error === null)
  );
}

function isChat(value: unknown): value is OnboardingChat {
  if (!isRecord(value)) return false;
  const status = value.status;
  return (
    (status === "idle" || status === "pending" || status === "success" || status === "error") &&
    (typeof value.reply === "string" || value.reply === null) &&
    (typeof value.error === "string" || value.error === null)
  );
}

function isOnboardingStep(value: unknown): value is OnboardingStep {
  return value === "language"
    || value === "character"
    || value === "provider"
    || value === "connection-test"
    || value === "chat";
}

function isOnboardingDraft(value: unknown): value is OnboardingDraft {
  if (!isRecord(value)) return false;
  return (
    value.version === ONBOARDING_VERSION &&
    isOnboardingStep(value.step) &&
    (typeof value.language === "string" || value.language === null) &&
    (typeof value.characterId === "string" || value.characterId === null) &&
    (typeof value.providerId === "string" || value.providerId === null) &&
    isConnectionTest(value.connectionTest) &&
    isChat(value.chat) &&
    typeof value.dismissed === "boolean" &&
    typeof value.completed === "boolean"
  );
}

/** Parses persisted state and rejects malformed or outdated drafts safely. */
export function deserializeOnboardingDraft(raw: string | null): OnboardingDraft | null {
  if (raw === null || raw.trim().length === 0) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    return isOnboardingDraft(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
