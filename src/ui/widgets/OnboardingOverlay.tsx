// pattern: Imperative Shell

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronRight, Languages, Loader2, MessageCircle, RotateCcw, Sparkles, X } from "lucide-react";

import {
  type OnboardingDraft,
  type OnboardingFlowEvent,
  type OnboardingStep,
} from "../../features/onboarding/onboarding-flow";
import type { ProviderSetup } from "../../features/onboarding/provider-setup";
import type { LlmConnectionTestResult } from "../../lib/kokoro-bridge";
import { ProviderSetupStep } from "./onboarding/ProviderSetupStep";

export type OnboardingLanguageCode = "en" | "zh" | "zh-TW" | "ja" | "ko" | "ru";

export type OnboardingCharacter = Readonly<{
  id: string;
  name: string;
  description: string;
  avatarPath: string | null;
}>;

export type OnboardingOverlayProps = Readonly<{
  draft: OnboardingDraft;
  characters: ReadonlyArray<OnboardingCharacter>;
  providerSetup: ProviderSetup;
  connectionResult: LlmConnectionTestResult | null;
  isTestingConnection: boolean;
  isSubmittingChat: boolean;
  discoveredModels?: ReadonlyArray<string>;
  isDiscoveringModels?: boolean;
  onDiscoverModels?: () => Promise<void> | void;
  isSavingProvider?: boolean;
  providerError?: string | null;
  characterError?: string | null;
  onEvent: (event: OnboardingFlowEvent) => void;
  onLanguageSelect: (language: OnboardingLanguageCode) => void;
  onCharacterSelect: (characterId: string) => void;
  onProviderChange: (setup: ProviderSetup) => void;
  onProviderSave?: () => Promise<void> | void;
  onTestConnection: () => Promise<void> | void;
  onChatSubmit: (message: string) => Promise<string>;
  onFirstReplySucceeded: (reply: string) => void;
  onDismiss: () => void;
  onResume: () => void;
}>;

const LANGUAGE_OPTIONS: ReadonlyArray<{ code: OnboardingLanguageCode; label: string }> = [
  { code: "en", label: "English" },
  { code: "zh", label: "简体中文" },
  { code: "zh-TW", label: "繁體中文" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
  { code: "ru", label: "Русский" },
];

const STEP_ORDER: ReadonlyArray<OnboardingStep> = [
  "language",
  "character",
  "provider",
  "connection-test",
  "chat",
];

function initials(name: string): string {
  return Array.from(name.trim()).slice(0, 2).join("").toUpperCase() || "?";
}

function stepLabel(step: OnboardingStep, t: (key: string, options?: { defaultValue?: string }) => string): string {
  return t(`onboarding.workflow.steps.${step}`, { defaultValue: step === "connection-test" ? "Test connection" : step });
}

/**
 * Outcome-led first-run workflow. The shell renders the current draft and
 * delegates persistence and side effects to App's onboarding service.
 */
export default function OnboardingOverlay(props: OnboardingOverlayProps) {
  const { t } = useTranslation();
  const [chatMessage, setChatMessage] = useState("");
  const [chatError, setChatError] = useState<string | null>(null);
  const [providerError, setProviderError] = useState<string | null>(props.providerError ?? null);
  const [isSavingProvider, setIsSavingProvider] = useState(false);
  const [characterError, setCharacterError] = useState<string | null>(props.characterError ?? null);
  const [characterRetryId, setCharacterRetryId] = useState<string | null>(null);
  const [isSelectingCharacter, setIsSelectingCharacter] = useState(false);

  const selectCharacter = async (characterId: string): Promise<void> => {
    setCharacterRetryId(characterId);
    setCharacterError(null);
    setIsSelectingCharacter(true);
    try {
      await props.onCharacterSelect(characterId);
    } catch {
      setCharacterError(t("onboarding.workflow.errors.character_activate", {
        defaultValue: "We couldn't activate this character. Check the character package and retry.",
      }));
    } finally {
      setIsSelectingCharacter(false);
    }
  };

  const saveProvider = async (): Promise<void> => {
    if (!props.onProviderSave) return;
    setProviderError(null);
    setIsSavingProvider(true);
    try {
      await props.onProviderSave();
    } catch {
      setProviderError(t("onboarding.workflow.errors.provider_save", {
        defaultValue: "We couldn't save this provider. Check the endpoint, model, and key, then retry.",
      }));
    } finally {
      setIsSavingProvider(false);
    }
  };

  useEffect(() => {
    if (props.providerError !== undefined) setProviderError(props.providerError);
  }, [props.providerError]);

  useEffect(() => {
    if (props.characterError !== undefined) setCharacterError(props.characterError);
  }, [props.characterError]);

  if (props.draft.completed) return null;

  const isDismissed = props.draft.dismissed;
  const activeStep = props.draft.step;
  const activeStepIndex = STEP_ORDER.indexOf(activeStep);

  const submitChat = async (): Promise<void> => {
    const message = chatMessage.trim();
    if (!message || props.isSubmittingChat) return;
    setChatError(null);
    try {
      const reply = await props.onChatSubmit(message);
      if (reply.trim().length > 0) {
        props.onFirstReplySucceeded(reply);
      }
    } catch {
      const errorMessage = t("onboarding.workflow.errors.chat_failed", {
        defaultValue: "We couldn't send that message. Check the provider connection and retry.",
      });
      setChatError(errorMessage);
      props.onEvent({ type: "chat-failed", error: errorMessage });
    }
  };

  return (
    <div className="pointer-events-none fixed inset-0 z-[140] flex items-center justify-center p-4">
      <div className="pointer-events-auto w-full max-w-[520px] overflow-hidden rounded-3xl border border-[var(--color-border-accent)]/60 bg-[var(--color-bg-elevated)]/95 shadow-[0_24px_100px_rgba(0,0,0,0.6)] backdrop-blur-2xl">
        <header className="flex items-start justify-between gap-4 border-b border-[var(--color-border)] px-6 py-5">
          <div>
            <div className="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.24em] text-[var(--color-accent)]">
              <Sparkles size={14} aria-hidden="true" />
              {t("onboarding.workflow.eyebrow", { defaultValue: "Your first reply" })}
              <span className="text-[var(--color-text-muted)]">{Math.max(activeStepIndex + 1, 1)}/5</span>
            </div>
            <h2 className="mt-2 font-heading text-xl font-bold tracking-wide text-[var(--color-text-primary)]">
              {isDismissed
                ? t("onboarding.workflow.resume_title", { defaultValue: "Pick up where you left off" })
                : t(`onboarding.workflow.titles.${activeStep}`, { defaultValue: "Meet your companion" })}
            </h2>
            <p className="mt-1 text-sm leading-5 text-[var(--color-text-secondary)]">
              {isDismissed
                ? t("onboarding.workflow.resume_description", { defaultValue: "Your choices are saved. Resume whenever you are ready." })
                : t(`onboarding.workflow.descriptions.${activeStep}`, { defaultValue: "A few focused steps are all it takes to start chatting." })}
            </p>
          </div>
          <button
            type="button"
            data-onboarding-action="dismiss"
            onClick={props.onDismiss}
            className="rounded-xl p-2 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-text-primary)]"
            aria-label={t("onboarding.workflow.dismiss", { defaultValue: "Save and close" })}
          >
            <X size={17} aria-hidden="true" />
          </button>
        </header>

        <div className="flex gap-1 border-b border-[var(--color-border)] px-6 py-3" aria-label={t("onboarding.workflow.progress", { defaultValue: "Onboarding progress" })}>
          {STEP_ORDER.map((step, index) => (
            <div key={step} className="flex min-w-0 flex-1 items-center gap-1.5">
              <span className={`grid h-6 w-6 shrink-0 place-items-center rounded-full border text-[10px] font-semibold ${index < activeStepIndex || props.draft.completed ? "border-emerald-300/50 bg-emerald-400/15 text-emerald-200" : index === activeStepIndex ? "border-[var(--color-accent)] bg-[var(--color-accent)]/15 text-[var(--color-accent)]" : "border-[var(--color-border)] text-[var(--color-text-muted)]"}`}>
                {index < activeStepIndex || props.draft.completed ? <Check size={12} aria-hidden="true" /> : index + 1}
              </span>
              <span className="hidden truncate text-[10px] uppercase tracking-[0.12em] text-[var(--color-text-muted)] sm:block">{stepLabel(step, t)}</span>
              {index < STEP_ORDER.length - 1 && <ChevronRight size={12} className="ml-auto text-[var(--color-border)]" aria-hidden="true" />}
            </div>
          ))}
        </div>

        {isDismissed ? (
          <div className="space-y-4 px-6 py-7">
            <div className="rounded-2xl border border-[var(--color-border)] bg-black/15 px-4 py-4 text-sm text-[var(--color-text-secondary)]">
              <p>{t("onboarding.workflow.saved_hint", { defaultValue: "Your language, character, and provider selections are safe." })}</p>
            </div>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={props.onDismiss} className="rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.14em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)]">
                {t("onboarding.workflow.dismiss", { defaultValue: "Save and close" })}
              </button>
              <button type="button" data-onboarding-action="resume" onClick={() => { props.onEvent({ type: "resume" }); props.onResume(); }} className="inline-flex items-center gap-2 rounded-xl bg-[var(--color-accent)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.14em] text-black transition hover:bg-white">
                <RotateCcw size={13} aria-hidden="true" />
                {t("onboarding.workflow.resume", { defaultValue: "Resume" })}
              </button>
            </div>
          </div>
        ) : (
          <div className="space-y-5 px-6 py-6">
            {activeStep === "language" && (
              <div className="space-y-3">
                <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.16em] text-[var(--color-text-muted)]">
                  <Languages size={14} aria-hidden="true" />
                  {t("onboarding.workflow.language_label", { defaultValue: "Interface language" })}
                </div>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                  {LANGUAGE_OPTIONS.map((option) => (
                    <button
                      key={option.code}
                      type="button"
                      data-onboarding-language={option.code}
                      onClick={() => props.onLanguageSelect(option.code)}
                      className={`rounded-xl border px-3 py-3 text-left text-sm transition ${props.draft.language === option.code ? "border-[var(--color-accent)] bg-[var(--color-accent)]/15 text-[var(--color-accent)]" : "border-[var(--color-border)] bg-black/15 text-[var(--color-text-secondary)] hover:border-[var(--color-border-accent)] hover:text-[var(--color-text-primary)]"}`}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {activeStep === "character" && (
              <div className="grid gap-2 sm:grid-cols-2">
                {props.characters.map((character) => (
                  <button
                    key={character.id}
                    type="button"
                    data-onboarding-character-id={character.id}
                    onClick={() => void selectCharacter(character.id)}
                    disabled={isSelectingCharacter}
                    className={`flex items-center gap-3 rounded-2xl border p-3 text-left transition ${props.draft.characterId === character.id ? "border-[var(--color-accent)] bg-[var(--color-accent)]/12" : "border-[var(--color-border)] bg-black/15 hover:border-[var(--color-border-accent)]"}`}
                  >
                    <span className="grid h-11 w-11 shrink-0 place-items-center overflow-hidden rounded-xl border border-[var(--color-border)] bg-[var(--color-accent)]/10 text-xs font-bold text-[var(--color-accent)]">
                      {character.avatarPath ? <img src={character.avatarPath} alt="" className="h-full w-full object-cover" /> : initials(character.name)}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-semibold text-[var(--color-text-primary)]">{character.name}</span>
                      <span className="mt-1 line-clamp-2 block text-xs leading-4 text-[var(--color-text-muted)]">{character.description}</span>
                    </span>
                  </button>
                ))}
                {characterError && (
                  <div className="col-span-full space-y-2">
                    <p role="alert" className="rounded-xl border border-red-300/30 bg-red-400/10 px-3 py-2 text-xs leading-5 text-red-200">{characterError}</p>
                    {characterRetryId && (
                      <button type="button" data-onboarding-action="retry-character" onClick={() => void selectCharacter(characterRetryId)} disabled={isSelectingCharacter} className="inline-flex items-center gap-2 rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)] disabled:opacity-50">
                        <RotateCcw size={13} aria-hidden="true" />
                        {t("onboarding.workflow.retry", { defaultValue: "Retry" })}
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}

            {activeStep === "provider" && (
              <div className="space-y-2">
                <ProviderSetupStep
                  setup={props.providerSetup}
                  onChange={props.onProviderChange}
                  discoveredModels={props.discoveredModels}
                  isDiscovering={props.isDiscoveringModels}
                  onDiscoverModels={props.onDiscoverModels}
                  error={providerError}
                  isSaving={props.isSavingProvider ?? isSavingProvider}
                  onSave={props.onProviderSave ? saveProvider : undefined}
                />
                {providerError && (
                  <button type="button" data-onboarding-action="retry-provider" onClick={() => void saveProvider()} disabled={props.isSavingProvider ?? isSavingProvider} className="inline-flex items-center gap-2 rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)] disabled:opacity-50">
                    <RotateCcw size={13} aria-hidden="true" />
                    {t("onboarding.workflow.retry", { defaultValue: "Retry" })}
                  </button>
                )}
              </div>
            )}

            {activeStep === "connection-test" && (
              <div className="space-y-4">
                <div className="rounded-2xl border border-[var(--color-border)] bg-black/15 px-4 py-4">
                  <p className="text-sm font-semibold text-[var(--color-text-primary)]">{t("onboarding.workflow.connection_title", { defaultValue: "Check the connection" })}</p>
                  <p className="mt-1 text-xs leading-5 text-[var(--color-text-muted)]">{t("onboarding.workflow.connection_description", { defaultValue: "We will make one safe request to confirm your provider can answer." })}</p>
                </div>
                {(props.draft.connectionTest.error || props.connectionResult === null) && props.draft.connectionTest.error && (
                  <p role="alert" className="rounded-xl border border-red-300/30 bg-red-400/10 px-3 py-2 text-xs leading-5 text-red-200">{props.draft.connectionTest.error}</p>
                )}
                {props.connectionResult && <p role="status" className="flex items-center gap-2 rounded-xl border border-emerald-300/25 bg-emerald-400/10 px-3 py-2 text-xs text-emerald-200"><Check size={13} aria-hidden="true" />{t("onboarding.workflow.connection_success", { defaultValue: "Connection looks good." })}</p>}
                <div className="flex flex-wrap justify-end gap-2">
                  {props.draft.connectionTest.status === "error" && <button type="button" data-onboarding-action="retry" onClick={() => props.onEvent({ type: "retry" })} className="inline-flex items-center gap-2 rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)]"><RotateCcw size={13} aria-hidden="true" />{t("onboarding.workflow.retry", { defaultValue: "Retry" })}</button>}
                  {props.draft.connectionTest.status === "error" && <button type="button" data-onboarding-action="edit-provider" onClick={() => props.onEvent({ type: "edit-provider" })} className="inline-flex items-center gap-2 rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)]">{t("onboarding.workflow.edit_provider", { defaultValue: "Edit provider" })}</button>}
                  <button type="button" data-onboarding-action="test-connection" onClick={() => void props.onTestConnection()} disabled={props.isTestingConnection} className="inline-flex items-center gap-2 rounded-xl bg-[var(--color-accent)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-black transition hover:bg-white disabled:cursor-wait disabled:opacity-60">
                    {props.isTestingConnection ? <Loader2 size={13} className="animate-spin" aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
                    {props.isTestingConnection ? t("onboarding.workflow.testing", { defaultValue: "Testing" }) : t("onboarding.workflow.test_connection", { defaultValue: "Test connection" })}
                  </button>
                </div>
              </div>
            )}

            {activeStep === "chat" && (
              <div className="space-y-4">
                <div className="flex items-start gap-3 rounded-2xl border border-[var(--color-border)] bg-black/15 px-4 py-4">
                  <MessageCircle size={17} className="mt-0.5 shrink-0 text-[var(--color-accent)]" aria-hidden="true" />
                  <p className="text-sm leading-6 text-[var(--color-text-secondary)]">{t("onboarding.workflow.chat_description", { defaultValue: "Send one message so your companion can answer. On that first successful reply, onboarding is complete." })}</p>
                </div>
                <div className="flex gap-2">
                  <input
                    data-onboarding-chat-input
                    value={chatMessage}
                    onChange={(event) => setChatMessage(event.target.value)}
                    onKeyDown={(event) => { if (event.key === "Enter") void submitChat(); }}
                    placeholder={t("onboarding.workflow.chat_placeholder", { defaultValue: "Say hello…" })}
                    className="min-w-0 flex-1 rounded-xl border border-[var(--color-border)] bg-black/20 px-3 py-2.5 text-sm text-[var(--color-text-primary)] outline-none transition placeholder:text-[var(--color-text-muted)] focus:border-[var(--color-border-accent)]"
                  />
                  <button type="button" data-onboarding-action="send-chat" onClick={() => void submitChat()} disabled={props.isSubmittingChat || chatMessage.trim().length === 0} className="inline-flex items-center gap-2 rounded-xl bg-[var(--color-accent)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-black transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-50">
                    {props.isSubmittingChat ? <Loader2 size={13} className="animate-spin" aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
                    {t("onboarding.workflow.send", { defaultValue: "Send" })}
                  </button>
                </div>
                {(chatError || props.draft.chat.error) && (
                  <div className="space-y-2">
                    <p role="alert" className="rounded-xl border border-red-300/30 bg-red-400/10 px-3 py-2 text-xs leading-5 text-red-200">{chatError ?? props.draft.chat.error}</p>
                    <button type="button" data-onboarding-action="retry-chat" onClick={() => void submitChat()} disabled={props.isSubmittingChat || chatMessage.trim().length === 0} className="inline-flex items-center gap-2 rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)] disabled:opacity-50">
                      <RotateCcw size={13} aria-hidden="true" />
                      {t("onboarding.workflow.retry", { defaultValue: "Retry" })}
                    </button>
                  </div>
                )}
              </div>
            )}

            <div className="flex items-center justify-between gap-3 border-t border-[var(--color-border)] pt-4">
              <button type="button" data-onboarding-action="dismiss" onClick={props.onDismiss} className="rounded-xl border border-[var(--color-border)] px-4 py-2 text-xs font-semibold uppercase tracking-[0.13em] text-[var(--color-text-secondary)] transition hover:border-[var(--color-border-accent)]">
                {t("onboarding.workflow.dismiss", { defaultValue: "Save and close" })}
              </button>
              <span className="text-right text-[10px] uppercase tracking-[0.12em] text-[var(--color-text-muted)]">{t("onboarding.workflow.keep_primary_surface", { defaultValue: "Live2D + chat stay visible" })}</span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
