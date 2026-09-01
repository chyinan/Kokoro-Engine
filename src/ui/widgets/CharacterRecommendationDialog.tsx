// pattern: Imperative Shell

import { Bot, Brain, Eye, PlugZap, ShieldCheck, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getKokoroErrorMessage } from "@/lib/kokoro-bridge";

export type CharacterCapabilityRecommendations = {
  readonly vision: boolean;
  readonly memory: boolean;
  readonly mcpServers: ReadonlyArray<string>;
  readonly botPlatforms: ReadonlyArray<string>;
};

export type RecommendationItem = {
  readonly type: "vision" | "memory" | "mcp" | "bot";
  readonly value: string;
};

type ApplyRecommendationDecisionOptions = {
  readonly decision: "confirm" | "dismiss";
  readonly recommendations: Readonly<CharacterCapabilityRecommendations>;
  readonly enableCapabilities: (
    recommendations: Readonly<CharacterCapabilityRecommendations>,
  ) => Promise<void>;
};

export type CharacterRecommendationDialogProps = {
  readonly open: boolean;
  readonly characterName: string;
  readonly recommendations: Readonly<CharacterCapabilityRecommendations> | null;
  readonly onConfirm: (
    recommendations: Readonly<CharacterCapabilityRecommendations>,
  ) => Promise<void>;
  readonly onDismiss: () => void;
};

/** Converts the declarative recommendation payload into display-only items. */
export function getRecommendationItems(
  recommendations: Readonly<CharacterCapabilityRecommendations>,
): Array<RecommendationItem> {
  const items: Array<RecommendationItem> = [];
  if (recommendations.vision) items.push({ type: "vision", value: "vision" });
  if (recommendations.memory) items.push({ type: "memory", value: "memory" });
  items.push(...recommendations.mcpServers.map((value) => ({ type: "mcp" as const, value })));
  items.push(...recommendations.botPlatforms.map((value) => ({ type: "bot" as const, value })));
  return items;
}

/** Applies sensitive capability changes only for an explicit confirm decision. */
export async function applyRecommendationDecision(
  options: Readonly<ApplyRecommendationDecisionOptions>,
): Promise<void> {
  if (options.decision === "confirm") {
    await options.enableCapabilities(options.recommendations);
  }
}

/** Identifies one open recommendation session so transient state cannot leak. */
export function getRecommendationSessionKey(
  open: boolean,
  characterName: string,
  recommendations: Readonly<CharacterCapabilityRecommendations> | null,
): string {
  return JSON.stringify([open, characterName, recommendations]);
}

function itemIcon(type: RecommendationItem["type"]) {
  switch (type) {
    case "vision":
      return <Eye size={15} aria-hidden="true" />;
    case "memory":
      return <Brain size={15} aria-hidden="true" />;
    case "mcp":
      return <PlugZap size={15} aria-hidden="true" />;
    case "bot":
      return <Bot size={15} aria-hidden="true" />;
  }
}

/** Explicit-consent gate displayed after a successful character activation. */
export function CharacterRecommendationDialog(
  props: Readonly<CharacterRecommendationDialogProps>,
) {
  const { t } = useTranslation();
  const [isConfirming, setIsConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorSessionKey, setErrorSessionKey] = useState<string | null>(null);
  const sessionKey = getRecommendationSessionKey(
    props.open,
    props.characterName,
    props.recommendations,
  );
  const sessionKeyRef = useRef(sessionKey);
  // Keep async confirmation callbacks scoped to the props currently rendered.
  sessionKeyRef.current = sessionKey;
  useEffect(() => {
    setError(null);
    setErrorSessionKey(null);
    setIsConfirming(false);
  }, [sessionKey]);
  if (!props.open || props.recommendations === null) return null;

  const items = getRecommendationItems(props.recommendations);
  if (items.length === 0) return null;
  const recommendations = props.recommendations;

  const handleConfirm = async (): Promise<void> => {
    const confirmationSession = sessionKey;
    setIsConfirming(true);
    setError(null);
    setErrorSessionKey(null);
    try {
      await applyRecommendationDecision({
        decision: "confirm",
        recommendations,
        enableCapabilities: props.onConfirm,
      });
      if (sessionKeyRef.current === confirmationSession) props.onDismiss();
    } catch (reason) {
      if (sessionKeyRef.current === confirmationSession) {
        setError(getKokoroErrorMessage(reason));
        setErrorSessionKey(confirmationSession);
      }
    } finally {
      if (sessionKeyRef.current === confirmationSession) setIsConfirming(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[90] flex items-end justify-center bg-black/45 p-5 backdrop-blur-[2px] sm:items-center"
      role="presentation"
      onMouseDown={(event) => {
        if (event.currentTarget === event.target && !isConfirming) props.onDismiss();
      }}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="character-recommendation-title"
        className="w-full max-w-md overflow-hidden rounded-2xl border border-[var(--color-border-accent)]/50 bg-[color:var(--color-bg-surface)]/95 shadow-[0_24px_80px_rgba(0,0,0,0.58),0_0_32px_rgba(0,240,255,0.08)]"
      >
        <div className="flex items-start gap-3 border-b border-[var(--color-border)] px-5 py-4">
          <div className="mt-0.5 rounded-xl border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/10 p-2 text-[var(--color-accent)]">
            <ShieldCheck size={18} aria-hidden="true" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-[var(--color-accent)]">
              {t("characterCatalog.recommendations.eyebrow")}
            </p>
            <h2 id="character-recommendation-title" className="mt-1 font-heading text-base font-semibold text-[var(--color-text-primary)]">
              {t("characterCatalog.recommendations.title", { name: props.characterName })}
            </h2>
            <p className="mt-1 text-xs leading-5 text-[var(--color-text-muted)]">
              {t("characterCatalog.recommendations.description")}
            </p>
          </div>
          <button
            type="button"
            onClick={props.onDismiss}
            disabled={isConfirming}
            aria-label={t("characterCatalog.recommendations.dismiss")}
            className="rounded-lg p-1.5 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-text-primary)] disabled:opacity-50"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        <ul className="grid gap-2 px-5 py-4 sm:grid-cols-2">
          {items.map((item) => (
            <li
              key={`${item.type}:${item.value}`}
              className="flex items-center gap-2.5 rounded-xl border border-[var(--color-border)] bg-black/15 px-3 py-2.5 text-xs text-[var(--color-text-secondary)]"
            >
              <span className="text-[var(--color-accent)]">{itemIcon(item.type)}</span>
              <span className="min-w-0 truncate">
                {item.type === "mcp" || item.type === "bot"
                  ? t(`characterCatalog.recommendations.${item.type}`, { name: item.value })
                  : t(`characterCatalog.recommendations.${item.type}`)}
              </span>
            </li>
          ))}
        </ul>

        {error !== null && errorSessionKey === sessionKey && (
          <p role="alert" className="mx-5 mb-3 rounded-lg border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
            {error}
          </p>
        )}

        <div className="flex items-center justify-end gap-2 border-t border-[var(--color-border)] px-5 py-3.5">
          <button
            type="button"
            onClick={props.onDismiss}
            disabled={isConfirming}
            className="rounded-lg px-3 py-2 text-xs text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-text-primary)] disabled:opacity-50"
          >
            {t("characterCatalog.recommendations.notNow")}
          </button>
          <button
            type="button"
            onClick={() => void handleConfirm()}
            disabled={isConfirming}
            className="rounded-lg border border-[var(--color-accent)]/50 bg-[var(--color-accent)]/15 px-4 py-2 text-xs font-semibold text-[var(--color-accent)] transition hover:bg-[var(--color-accent)]/25 disabled:opacity-50"
          >
            {isConfirming
              ? t("characterCatalog.recommendations.enabling")
              : t("characterCatalog.recommendations.enable")}
          </button>
        </div>
      </section>
    </div>
  );
}
