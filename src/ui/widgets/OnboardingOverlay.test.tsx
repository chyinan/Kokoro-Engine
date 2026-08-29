// @vitest-environment jsdom
// pattern: Imperative Shell

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, test, vi } from "vitest";

import {
  createOnboardingDraft,
  onboardingFlowReducer,
  type OnboardingDraft,
  type OnboardingFlowEvent,
} from "../../features/onboarding/onboarding-flow";
import type { ProviderSetup } from "../../features/onboarding/provider-setup";
import OnboardingOverlay, { type OnboardingCharacter, type OnboardingOverlayProps } from "./OnboardingOverlay";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { readonly defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}));

const providerSetup: ProviderSetup = {
  providerType: "ollama",
  presetId: null,
  endpoint: "http://localhost:11434",
  apiKey: null,
  model: "llama3",
};

const characters: ReadonlyArray<OnboardingCharacter> = [
  { id: "kokoro", name: "Kokoro", description: "A warm studio companion", avatarPath: null },
  { id: "pico", name: "Pico", description: "A focused coding partner", avatarPath: null },
];

function createDraft(step: OnboardingDraft["step"]): OnboardingDraft {
  let draft = createOnboardingDraft();
  if (step === "language") return draft;
  draft = onboardingFlowReducer(draft, { type: "select-language", language: "en" });
  if (step === "character") return draft;
  draft = onboardingFlowReducer(draft, { type: "select-character", characterId: "kokoro" });
  if (step === "provider") return draft;
  draft = onboardingFlowReducer(draft, { type: "configure-provider", providerId: "ollama" });
  if (step === "connection-test") return draft;
  draft = onboardingFlowReducer(draft, { type: "connection-test-succeeded" });
  return draft;
}

function renderOverlay(overrides: Partial<OnboardingOverlayProps> = {}): {
  container: HTMLDivElement;
  rerender: (next: Partial<OnboardingOverlayProps>) => void;
  unmount: () => void;
} {
  const container = document.createElement("div");
  document.body.appendChild(container);
  let props: OnboardingOverlayProps = {
    draft: createDraft("language"),
    characters,
    providerSetup,
    connectionResult: null,
    isTestingConnection: false,
    isSubmittingChat: false,
    providerError: null,
    characterError: null,
    onEvent: vi.fn<(event: OnboardingFlowEvent) => void>(),
    onLanguageSelect: vi.fn(),
    onCharacterSelect: vi.fn(),
    onProviderChange: vi.fn(),
    onProviderSave: vi.fn(),
    onTestConnection: vi.fn(),
    onChatSubmit: vi.fn().mockResolvedValue("Welcome to Kokoro"),
    onFirstReplySucceeded: vi.fn(),
    onDismiss: vi.fn(),
    onResume: vi.fn(),
    ...overrides,
  };
  const root: Root = createRoot(container);
  act(() => {
    root.render(<OnboardingOverlay {...props} />);
  });

  return {
    container,
    rerender: (next) => {
      props = { ...props, ...next };
      act(() => {
        root.render(<OnboardingOverlay {...props} />);
      });
    },
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function click(container: HTMLElement, selector: string): void {
  const element = container.querySelector<HTMLElement>(selector);
  expect(element, `expected ${selector} to exist`).not.toBeNull();
  act(() => {
    element?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("OnboardingOverlay workflow", () => {
  test("selects a language and reports the next setup action", () => {
    const onLanguageSelect = vi.fn();
    const { container, unmount } = renderOverlay({ onLanguageSelect });

    click(container, '[data-onboarding-language="en"]');

    expect(onLanguageSelect).toHaveBeenCalledWith("en");
    unmount();
  });

  test("selects a character from the focused onboarding catalog", () => {
    const onCharacterSelect = vi.fn();
    const { container, rerender, unmount } = renderOverlay({ onCharacterSelect });

    rerender({ draft: createDraft("character") });
    click(container, '[data-onboarding-character-id="pico"]');

    expect(onCharacterSelect).toHaveBeenCalledWith("pico");
    unmount();
  });

  test("surfaces activation failure with a localized retry action", async () => {
    const onCharacterSelect = vi
      .fn<OnboardingOverlayProps["onCharacterSelect"]>()
      .mockRejectedValueOnce(new Error("backend activation failed"))
      .mockResolvedValueOnce(undefined);
    const { container, rerender, unmount } = renderOverlay({ onCharacterSelect });

    rerender({ draft: createDraft("character") });
    click(container, '[data-onboarding-character-id="pico"]');
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("couldn't activate");
    click(container, '[data-onboarding-action="retry-character"]');
    expect(onCharacterSelect).toHaveBeenCalledTimes(2);
    unmount();
  });

  test("offers a retry after a failed connection test without losing provider setup", () => {
    const onEvent = vi.fn<(event: OnboardingFlowEvent) => void>();
    const onTestConnection = vi.fn();
    const failed = onboardingFlowReducer(createDraft("connection-test"), {
      type: "connection-test-failed",
      error: "provider unavailable",
    });
    const { container, unmount } = renderOverlay({ draft: failed, onEvent, onTestConnection });

    expect(container.textContent).toContain("provider unavailable");
    click(container, '[data-onboarding-action="retry"]');
    click(container, '[data-onboarding-action="test-connection"]');

    expect(onEvent).toHaveBeenCalledWith({ type: "retry" });
    expect(onTestConnection).toHaveBeenCalledTimes(1);
    unmount();
  });

  test("offers a provider edit action after a failed connection test", () => {
    const onEvent = vi.fn<(event: OnboardingFlowEvent) => void>();
    const failed = onboardingFlowReducer(createDraft("connection-test"), {
      type: "connection-test-failed",
      error: "provider unavailable",
    });
    const { container, unmount } = renderOverlay({ draft: failed, onEvent });

    click(container, '[data-onboarding-action="edit-provider"]');
    expect(onEvent).toHaveBeenCalledWith({ type: "edit-provider" });
    unmount();
  });

  test("surfaces provider save failures with a retry action", async () => {
    const onProviderSave = vi.fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("save failed"))
      .mockResolvedValueOnce(undefined);
    const { container, rerender, unmount } = renderOverlay({ onProviderSave });

    rerender({ draft: createDraft("provider") });
    expect(container.querySelector('[data-onboarding-action="save-provider"]')?.hasAttribute("disabled")).toBe(false);
    click(container, '[data-onboarding-action="save-provider"]');
    expect(onProviderSave).toHaveBeenCalledTimes(1);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("couldn't save");
    click(container, '[data-onboarding-action="retry-provider"]');
    expect(onProviderSave).toHaveBeenCalledTimes(2);
    unmount();
  });

  test("supports dismissal and resume without changing the saved draft", () => {
    const onEvent = vi.fn<(event: OnboardingFlowEvent) => void>();
    const onDismiss = vi.fn();
    const onResume = vi.fn();
    const dismissed = onboardingFlowReducer(createDraft("provider"), { type: "dismiss" });
    const { container, unmount } = renderOverlay({ draft: dismissed, onEvent, onDismiss, onResume });

    expect(container.querySelector('[data-onboarding-action="resume"]')).not.toBeNull();
    click(container, '[data-onboarding-action="resume"]');
    expect(onResume).toHaveBeenCalledTimes(1);
    expect(onEvent).toHaveBeenCalledWith({ type: "resume" });
    unmount();
  });

  test("completes only after the first successful chat reply callback", async () => {
    const onChatSubmit = vi.fn().mockResolvedValue("Hello from Kokoro");
    const onFirstReplySucceeded = vi.fn();
    const onEvent = vi.fn<(event: OnboardingFlowEvent) => void>();
    const { container, unmount } = renderOverlay({ draft: createDraft("chat"), onChatSubmit, onFirstReplySucceeded, onEvent });

    const input = container.querySelector<HTMLInputElement>('[data-onboarding-chat-input]');
    expect(input).not.toBeNull();
    act(() => {
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(input, "Say hello");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    click(container, '[data-onboarding-action="send-chat"]');
    await act(async () => {
      await Promise.resolve();
    });

    expect(onChatSubmit).toHaveBeenCalledWith("Say hello");
    expect(onFirstReplySucceeded).toHaveBeenCalledWith("Hello from Kokoro");
    expect(onEvent).not.toHaveBeenCalledWith(expect.objectContaining({ type: "first-reply-succeeded" }));
    unmount();
  });

  test("maps chat failures to actionable localized text and retries", async () => {
    const onChatSubmit = vi.fn<OnboardingOverlayProps["onChatSubmit"]>()
      .mockRejectedValueOnce(new Error("raw backend timeout"))
      .mockResolvedValueOnce("Welcome back");
    const onFirstReplySucceeded = vi.fn();
    const { container, rerender, unmount } = renderOverlay({ onChatSubmit, onFirstReplySucceeded });

    rerender({ draft: createDraft("chat") });
    const input = container.querySelector<HTMLInputElement>('[data-onboarding-chat-input]');
    act(() => {
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(input, "Hello again");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    click(container, '[data-onboarding-action="send-chat"]');
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("couldn't send");
    expect(container.textContent).not.toContain("raw backend timeout");
    click(container, '[data-onboarding-action="retry-chat"]');
    await act(async () => {
      await Promise.resolve();
    });
    expect(onChatSubmit).toHaveBeenCalledTimes(2);
    expect(onFirstReplySucceeded).toHaveBeenCalledWith("Welcome back");
    unmount();
  });
});
