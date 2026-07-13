// pattern: Imperative Shell

import {
  Check,
  ChevronDown,
  Copy,
  FilePenLine,
  Import,
  RefreshCcw,
  Sparkles,
  Upload,
  UserRound,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { CharacterRecord, CharacterTemplateManifest } from "@/lib/kokoro-bridge";

import {
  getRecommendationItems,
  type CharacterCapabilityRecommendations,
} from "./CharacterRecommendationDialog";

export type CharacterCatalogAction =
  | { readonly type: "select"; readonly characterId: string }
  | { readonly type: "import" }
  | { readonly type: "edit"; readonly characterId: string }
  | { readonly type: "duplicate"; readonly characterId: string }
  | { readonly type: "restore-default"; readonly characterId: string }
  | { readonly type: "resolve-conflict"; readonly characterId: string };

export type CharacterCatalogActionDependencies = {
  readonly activateCharacter: (
    characterId: string,
  ) => Promise<CharacterCapabilityRecommendations | null>;
  readonly importCharacter: () => Promise<void>;
  readonly editCharacter: (characterId: string) => Promise<void>;
  readonly duplicateCharacter: (characterId: string) => Promise<void>;
  readonly restoreCharacterDefaults: (characterId: string) => Promise<void>;
  readonly resolveTemplateConflict: (characterId: string) => Promise<void>;
};

type CharacterCatalogEntry = {
  readonly actionId: string;
  readonly name: string;
  readonly description: string;
  readonly avatarPath: string | null;
  readonly source: "template" | "instance";
  readonly hasTemplate: boolean;
};

export type CharacterCatalogProps = {
  readonly characters: ReadonlyArray<CharacterRecord>;
  readonly templates: ReadonlyArray<CharacterTemplateManifest>;
  readonly activeCharacterId: string;
  readonly actions: Readonly<CharacterCatalogActionDependencies>;
  readonly resolveAvatarUrl?: (path: string) => string;
  readonly onRecommendations: (
    characterName: string,
    recommendations: Readonly<CharacterCapabilityRecommendations>,
  ) => void;
};

/** Executes one catalog command and exposes recommendations only after selection succeeds. */
export async function executeCharacterCatalogAction(
  action: Readonly<CharacterCatalogAction>,
  dependencies: Readonly<CharacterCatalogActionDependencies>,
): Promise<CharacterCapabilityRecommendations | null> {
  switch (action.type) {
    case "select":
      return dependencies.activateCharacter(action.characterId);
    case "import":
      await dependencies.importCharacter();
      return null;
    case "edit":
      await dependencies.editCharacter(action.characterId);
      return null;
    case "duplicate":
      await dependencies.duplicateCharacter(action.characterId);
      return null;
    case "restore-default":
      await dependencies.restoreCharacterDefaults(action.characterId);
      return null;
    case "resolve-conflict":
      await dependencies.resolveTemplateConflict(action.characterId);
      return null;
  }
}

function buildCatalogEntries(
  characters: ReadonlyArray<CharacterRecord>,
  templates: ReadonlyArray<CharacterTemplateManifest>,
): Array<CharacterCatalogEntry> {
  const matchedInstanceIds = new Set<string>();
  const entries = templates.map((template) => {
    const instance = characters.find((candidate) => candidate.template_id === template.id);
    if (instance) matchedInstanceIds.add(instance.id);
    return {
      actionId: instance?.id ?? `template:${template.id}`,
      name: instance?.name ?? template.name,
      description: instance?.description?.trim() || template.description,
      avatarPath: instance?.avatar_path ?? template.avatar,
      source: instance ? "instance" as const : "template" as const,
      hasTemplate: true,
    };
  });
  for (const character of characters) {
    if (matchedInstanceIds.has(character.id)) continue;
    entries.push({
      actionId: character.id,
      name: character.name,
      description: character.description?.trim() || character.persona.trim(),
      avatarPath: character.avatar_path ?? null,
      source: "instance",
      hasTemplate: character.template_id != null,
    });
  }
  return entries;
}

function initials(name: string): string {
  return Array.from(name.trim()).slice(0, 2).join("").toUpperCase() || "?";
}

/** Compact, main-surface character selector that leaves Live2D and chat visible. */
export function CharacterCatalog(props: Readonly<CharacterCatalogProps>) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const entries = useMemo(
    () => buildCatalogEntries(props.characters, props.templates),
    [props.characters, props.templates],
  );
  const active = entries.find((entry) => entry.actionId === props.activeCharacterId) ?? entries[0] ?? null;

  const runAction = async (
    action: Readonly<CharacterCatalogAction>,
    characterName: string,
  ): Promise<void> => {
    const key = action.type === "import" ? action.type : `${action.type}:${action.characterId}`;
    setPendingAction(key);
    setError(null);
    try {
      const recommendation = await executeCharacterCatalogAction(action, props.actions);
      if (recommendation !== null && getRecommendationItems(recommendation).length > 0) {
        props.onRecommendations(characterName, recommendation);
      }
      if (action.type === "select") setIsOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setPendingAction(null);
    }
  };

  if (entries.length === 0) {
    return (
      <button
        type="button"
        onClick={() => void runAction({ type: "import" }, "")}
        className="pointer-events-auto flex items-center gap-2 rounded-full border border-[var(--color-border)] bg-[var(--color-bg-surface)]/90 px-3 py-2 text-xs text-[var(--color-text-secondary)] shadow-lg backdrop-blur-xl"
      >
        <Upload size={14} aria-hidden="true" />
        {t("characterCatalog.import")}
      </button>
    );
  }

  return (
    <div className="pointer-events-auto relative w-[min(360px,calc(100vw-32px))]">
      <button
        type="button"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        onClick={() => setIsOpen((current) => !current)}
        className="ml-auto flex max-w-[240px] items-center gap-2 rounded-full border border-[var(--color-border)] bg-[var(--color-bg-surface)]/90 py-1.5 pl-2 pr-3 text-left shadow-[0_10px_35px_rgba(0,0,0,0.28)] backdrop-blur-xl transition hover:border-[var(--color-border-accent)]"
      >
        <span className="grid h-7 w-7 shrink-0 place-items-center overflow-hidden rounded-full border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/10 text-[10px] font-bold text-[var(--color-accent)]">
          {active?.avatarPath && props.resolveAvatarUrl
            ? <img src={props.resolveAvatarUrl(active.avatarPath)} alt="" className="h-full w-full object-cover" />
            : initials(active?.name ?? "")}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[10px] uppercase tracking-[0.16em] text-[var(--color-text-muted)]">
            {t("characterCatalog.active")}
          </span>
          <span className="block truncate text-xs font-semibold text-[var(--color-text-primary)]">{active?.name}</span>
        </span>
        <ChevronDown size={14} className={`shrink-0 text-[var(--color-text-muted)] transition ${isOpen ? "rotate-180" : ""}`} aria-hidden="true" />
      </button>

      {isOpen && (
        <section className="absolute right-0 top-12 max-h-[min(480px,calc(100vh-120px))] w-full overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/95 shadow-[0_22px_70px_rgba(0,0,0,0.5)] backdrop-blur-2xl">
          <header className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
            <div>
              <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-[var(--color-accent)]">{t("characterCatalog.eyebrow")}</p>
              <h2 className="mt-0.5 text-sm font-semibold text-[var(--color-text-primary)]">{t("characterCatalog.title")}</h2>
            </div>
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => void runAction({ type: "import" }, "")}
                disabled={pendingAction !== null}
                aria-label={t("characterCatalog.import")}
                className="rounded-lg p-2 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-accent)] disabled:opacity-40"
              >
                <Import size={15} aria-hidden="true" />
              </button>
              <button type="button" onClick={() => setIsOpen(false)} aria-label={t("characterCatalog.close")} className="rounded-lg p-2 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-text-primary)]">
                <X size={15} aria-hidden="true" />
              </button>
            </div>
          </header>

          <div role="listbox" aria-label={t("characterCatalog.title")} className="max-h-[390px] space-y-1 overflow-y-auto p-2">
            {entries.map((entry) => {
              const isActive = entry.actionId === props.activeCharacterId;
              const isBusy = pendingAction?.endsWith(`:${entry.actionId}`) ?? false;
              return (
                <article key={`${entry.source}:${entry.actionId}`} className={`group rounded-xl border p-2 transition ${isActive ? "border-[var(--color-accent)]/45 bg-[var(--color-accent)]/8" : "border-transparent hover:border-[var(--color-border)] hover:bg-white/[0.025]"}`}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    disabled={pendingAction !== null}
                    onClick={() => void runAction({ type: "select", characterId: entry.actionId }, entry.name)}
                    className="flex w-full items-center gap-3 rounded-lg p-1.5 text-left disabled:opacity-55"
                  >
                    <span className="grid h-10 w-10 shrink-0 place-items-center overflow-hidden rounded-xl border border-[var(--color-border)] bg-black/20 text-xs font-bold text-[var(--color-text-secondary)]">
                      {entry.avatarPath && props.resolveAvatarUrl
                        ? <img src={props.resolveAvatarUrl(entry.avatarPath)} alt="" className="h-full w-full object-cover" />
                        : entry.source === "template" ? <Sparkles size={17} aria-hidden="true" /> : <UserRound size={17} aria-hidden="true" />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1.5">
                        <span className="truncate text-sm font-semibold text-[var(--color-text-primary)]">{entry.name}</span>
                        {isActive && <Check size={13} className="shrink-0 text-[var(--color-accent)]" aria-hidden="true" />}
                      </span>
                      <span className="mt-0.5 line-clamp-2 block text-[10px] leading-4 text-[var(--color-text-muted)]">{entry.description}</span>
                    </span>
                    {isBusy && <RefreshCcw size={13} className="animate-spin text-[var(--color-accent)]" aria-hidden="true" />}
                  </button>

                  {entry.source === "instance" && (
                    <div className="mt-1 flex justify-end gap-0.5 border-t border-[var(--color-border)]/60 pt-1 opacity-80 transition group-hover:opacity-100">
                      <CatalogActionButton label={t("characterCatalog.edit")} icon={<FilePenLine size={13} />} onClick={() => void runAction({ type: "edit", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                      <CatalogActionButton label={t("characterCatalog.duplicate")} icon={<Copy size={13} />} onClick={() => void runAction({ type: "duplicate", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                      {entry.hasTemplate && (
                        <>
                          <CatalogActionButton label={t("characterCatalog.restoreDefault")} icon={<RefreshCcw size={13} />} onClick={() => void runAction({ type: "restore-default", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                          <CatalogActionButton label={t("characterCatalog.resolveConflict")} icon={<Sparkles size={13} />} onClick={() => void runAction({ type: "resolve-conflict", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                        </>
                      )}
                    </div>
                  )}
                </article>
              );
            })}
          </div>

          {error !== null && <p role="alert" className="border-t border-red-400/20 bg-red-500/10 px-4 py-2 text-[10px] text-red-200">{error}</p>}
        </section>
      )}
    </div>
  );
}

type CatalogActionButtonProps = {
  readonly label: string;
  readonly icon: React.ReactNode;
  readonly onClick: () => void;
  readonly disabled: boolean;
};

function CatalogActionButton(props: Readonly<CatalogActionButtonProps>) {
  return (
    <button type="button" title={props.label} aria-label={props.label} onClick={props.onClick} disabled={props.disabled} className="rounded-md p-1.5 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-accent)] disabled:opacity-40">
      {props.icon}
    </button>
  );
}
