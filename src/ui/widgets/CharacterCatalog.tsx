// pattern: Imperative Shell

import {
  Check,
  ChevronDown,
  Copy,
  Eye,
  FilePenLine,
  Import,
  RefreshCcw,
  Sparkles,
  Upload,
  UserRound,
  X,
} from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  getKokoroErrorMessage,
  type CharacterRecord,
  type CharacterTemplateManifest,
} from "@/lib/kokoro-bridge";

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
  readonly persona: string;
  readonly greeting: string;
  readonly exampleDialogue: string;
  readonly avatarPath: string | null;
  readonly source: "template" | "instance";
  readonly hasTemplate: boolean;
  readonly templateVersion: string | null;
  readonly availableTemplateVersion: string | null;
  readonly author: string | null;
};

export type CharacterCatalogProps = {
  readonly characters: ReadonlyArray<CharacterRecord>;
  readonly templates: ReadonlyArray<CharacterTemplateManifest>;
  readonly activeCharacterId: string;
  readonly actions: Readonly<CharacterCatalogActionDependencies>;
  /** Converts persisted filesystem/protocol references at the Tauri boundary. */
  readonly resolveAvatarUrl: (path: string) => string;
  readonly catalogError?: string | null;
  readonly actionError?: string | null;
  readonly isRetrying?: boolean;
  readonly onRetry?: () => void;
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
  const latestTemplates = getLatestCharacterTemplates(templates);
  const entries: Array<CharacterCatalogEntry> = [];

  for (const template of latestTemplates) {
    for (const character of characters.filter((candidate) => candidate.template_id === template.id)) {
      matchedInstanceIds.add(character.id);
      entries.push({
        actionId: character.id,
        name: character.name,
        description: character.description?.trim() || template.description,
        persona: character.persona,
        greeting: character.greeting ?? template.greeting,
        exampleDialogue: character.example_dialogue ?? template.example_dialogue ?? "",
        avatarPath: character.avatar_path ?? template.avatar,
        source: "instance",
        hasTemplate: true,
        templateVersion: character.template_version ?? null,
        availableTemplateVersion: template.version,
        author: template.author,
      });
    }
  }
  for (const character of characters) {
    if (matchedInstanceIds.has(character.id)) continue;
    const template = character.template_id === null || character.template_id === undefined
      ? undefined
      : latestTemplates.find((candidate) => candidate.id === character.template_id);
    entries.push({
      actionId: character.id,
      name: character.name,
      description: character.description?.trim() || character.persona.trim(),
      persona: character.persona,
      greeting: character.greeting ?? "",
      exampleDialogue: character.example_dialogue ?? "",
      avatarPath: character.avatar_path ?? null,
      source: "instance",
      hasTemplate: template !== undefined,
      templateVersion: character.template_version ?? null,
      availableTemplateVersion: template?.version ?? null,
      author: template?.author ?? null,
    });
  }
  return entries;
}

/** Keeps only the newest installed version for each template ID. */
export function getLatestCharacterTemplates(
  templates: ReadonlyArray<CharacterTemplateManifest>,
): Array<CharacterTemplateManifest> {
  const latestTemplates = new Map<string, CharacterTemplateManifest>();
  for (const template of templates) {
    const current = latestTemplates.get(template.id);
    if (current === undefined || compareCharacterTemplateVersions(template.version, current.version) > 0) {
      latestTemplates.set(template.id, template);
    }
  }
  return Array.from(latestTemplates.values());
}

type CharacterTemplateVersionParts = {
  readonly numbers: readonly [number, number, number];
  readonly prerelease: ReadonlyArray<string>;
};

function parseCharacterTemplateVersion(value: string): CharacterTemplateVersionParts | null {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(value);
  if (match === null) return null;
  return {
    numbers: [Number(match[1]), Number(match[2]), Number(match[3])],
    prerelease: match[4]?.split(".") ?? [],
  };
}

/** Compares valid SemVer strings so template updates choose the real newest version. */
export function compareCharacterTemplateVersions(left: string, right: string): number {
  const leftParts = parseCharacterTemplateVersion(left);
  const rightParts = parseCharacterTemplateVersion(right);
  if (leftParts === null || rightParts === null) return left.localeCompare(right);

  for (const index of [0, 1, 2] as const) {
    if (leftParts.numbers[index] !== rightParts.numbers[index]) {
      return leftParts.numbers[index] - rightParts.numbers[index];
    }
  }

  if (leftParts.prerelease.length === 0 || rightParts.prerelease.length === 0) {
    return leftParts.prerelease.length === rightParts.prerelease.length
      ? 0
      : leftParts.prerelease.length === 0 ? 1 : -1;
  }
  for (let index = 0; index < Math.max(leftParts.prerelease.length, rightParts.prerelease.length); index += 1) {
    const leftIdentifier = leftParts.prerelease[index];
    const rightIdentifier = rightParts.prerelease[index];
    if (leftIdentifier === undefined || rightIdentifier === undefined) {
      return leftIdentifier === rightIdentifier ? 0 : leftIdentifier === undefined ? -1 : 1;
    }
    if (leftIdentifier === rightIdentifier) continue;
    const leftNumber = Number(leftIdentifier);
    const rightNumber = Number(rightIdentifier);
    const leftIsNumber = Number.isInteger(leftNumber) && /^\d+$/.test(leftIdentifier);
    const rightIsNumber = Number.isInteger(rightNumber) && /^\d+$/.test(rightIdentifier);
    if (leftIsNumber && rightIsNumber) return leftNumber - rightNumber;
    if (leftIsNumber !== rightIsNumber) return leftIsNumber ? -1 : 1;
    return leftIdentifier.localeCompare(rightIdentifier);
  }
  return 0;
}

function initials(name: string): string {
  return Array.from(name.trim()).slice(0, 2).join("").toUpperCase() || "?";
}

/** Compact, main-surface character selector that leaves Live2D and chat visible. */
export function CharacterCatalog(props: Readonly<CharacterCatalogProps>) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [previewEntry, setPreviewEntry] = useState<CharacterCatalogEntry | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const previewDialogRef = useRef<HTMLElement | null>(null);
  const previewCloseButtonRef = useRef<HTMLButtonElement | null>(null);
  const previewTriggerRef = useRef<HTMLButtonElement | null>(null);
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const entries = useMemo(
    () => buildCatalogEntries(props.characters, props.templates),
    [props.characters, props.templates],
  );
  const active = entries.find((entry) => entry.actionId === props.activeCharacterId) ?? entries[0] ?? null;
  const displayedActionError = error ?? props.actionError ?? null;

  function closePreview(): void {
    setPreviewEntry(null);
    previewTriggerRef.current?.focus();
  }

  useEffect(() => {
    if (!isOpen || previewEntry !== null) return;

    const handlePointerDown = (event: MouseEvent | TouchEvent): void => {
      if (
        containerRef.current !== null &&
        event.target instanceof Node &&
        !containerRef.current.contains(event.target)
      ) {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("touchstart", handlePointerDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("touchstart", handlePointerDown);
    };
  }, [isOpen, previewEntry]);

  useEffect(() => {
    if (previewEntry === null) return;

    const handlePreviewKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        closePreview();
        return;
      }
      if (event.key !== "Tab") return;

      const focusable = previewDialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable || focusable.length === 0) {
        event.preventDefault();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };

    document.addEventListener("keydown", handlePreviewKeyDown);
    previewCloseButtonRef.current?.focus();
    return () => document.removeEventListener("keydown", handlePreviewKeyDown);
  }, [previewEntry]);

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
      setError(getKokoroErrorMessage(reason));
    } finally {
      setPendingAction(null);
    }
  };

  if (entries.length === 0) {
    if (props.catalogError) {
      return (
        <div className="pointer-events-auto w-[min(360px,calc(100vw-32px))] rounded-2xl border border-red-400/30 bg-[var(--color-bg-surface)]/95 p-4 text-sm shadow-lg backdrop-blur-xl" role="alert">
          <p className="text-xs text-red-200">{props.catalogError}</p>
          {props.onRetry && (
            <button
              type="button"
              onClick={props.onRetry}
              disabled={props.isRetrying}
              className="mt-3 rounded-lg border border-[var(--color-accent)]/50 px-3 py-2 text-xs text-[var(--color-accent)] disabled:opacity-50"
            >
              {props.isRetrying ? t("characterCatalog.retrying", { defaultValue: "Retrying..." }) : t("characterCatalog.retry", { defaultValue: "Retry" })}
            </button>
          )}
        </div>
      );
    }
    return (
      <div className="pointer-events-auto flex flex-col items-start gap-2 rounded-2xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/90 p-3 shadow-lg backdrop-blur-xl">
        {displayedActionError !== null && <p role="alert" className="max-w-[260px] text-[10px] text-red-200">{displayedActionError}</p>}
        <button
          type="button"
          onClick={() => void runAction({ type: "import" }, "")}
          disabled={pendingAction !== null}
          className="flex items-center gap-2 rounded-full border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-text-secondary)] disabled:opacity-50"
        >
          <Upload size={14} aria-hidden="true" />
          {t("characterCatalog.import")}
        </button>
      </div>
    );
  }

  return (
    <div ref={containerRef} className="pointer-events-auto relative w-[min(360px,calc(100vw-32px))]">
      <button
        type="button"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        onClick={() => setIsOpen((current) => !current)}
        className="ml-auto flex max-w-[240px] items-center gap-2 rounded-full border border-[var(--color-border)] bg-[var(--color-bg-surface)]/90 py-1.5 pl-2 pr-3 text-left shadow-[0_10px_35px_rgba(0,0,0,0.28)] backdrop-blur-xl transition hover:border-[var(--color-border-accent)]"
      >
        <span className="grid h-7 w-7 shrink-0 place-items-center overflow-hidden rounded-full border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/10 text-[10px] font-bold text-[var(--color-accent)]">
          {active?.avatarPath
            ? <img src={props.resolveAvatarUrl(active.avatarPath)} alt="" className="h-full w-full object-cover" />
            : initials(active?.name ?? "")}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[10px] uppercase tracking-[0.16em] text-[var(--color-text-muted)]">
            {t("characterCatalog.active")}
          </span>
          <span className="block truncate text-xs font-semibold text-[var(--color-text-primary)]">{active?.name}</span>
        </span>
        <ChevronDown size={14} className={`shrink-0 text-[var(--color-text-muted)] transition-transform duration-250 ease-out ${isOpen ? "rotate-180" : ""}`} aria-hidden="true" />
      </button>

      {!isOpen && props.catalogError && <CatalogErrorNotice message={props.catalogError} isRetrying={props.isRetrying ?? false} onRetry={props.onRetry} t={t} />}
      {!isOpen && props.actionError && <CatalogErrorNotice message={props.actionError} isRetrying={false} t={t} />}

      <AnimatePresence>
        {isOpen && (
          <motion.section
            initial={{ opacity: 0, scale: 0.95, y: -8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{
              opacity: 0,
              scale: 0.95,
              y: -8,
              transition: { duration: 0.15, ease: [0.32, 0, 0.67, 0] },
            }}
            transition={{
              type: "spring",
              stiffness: 420,
              damping: 28,
              mass: 0.6,
            }}
            style={{ transformOrigin: "top right" }}
            className="absolute right-0 top-12 max-h-[min(480px,calc(100vh-120px))] w-full overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/95 shadow-[0_22px_70px_rgba(0,0,0,0.5)] backdrop-blur-2xl"
          >
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

          <div role="listbox" aria-label={t("characterCatalog.title")} className="max-h-[390px] space-y-1 overflow-y-auto p-2 scrollable">
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
                      {entry.avatarPath
                        ? <img src={props.resolveAvatarUrl(entry.avatarPath)} alt="" className="h-full w-full object-cover" />
                        : entry.source === "template" ? <Sparkles size={17} aria-hidden="true" /> : <UserRound size={17} aria-hidden="true" />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1.5">
                        <span className="truncate text-sm font-semibold text-[var(--color-text-primary)]">{entry.name}</span>
                        {isActive && <Check size={13} className="shrink-0 text-[var(--color-accent)]" aria-hidden="true" />}
                      </span>
                      <span className="mt-0.5 line-clamp-2 text-[10px] leading-4 text-[var(--color-text-primary)]">{entry.description}</span>
                    </span>
                    {isBusy && <RefreshCcw size={13} className="animate-spin text-[var(--color-accent)]" aria-hidden="true" />}
                  </button>

                  <div className="mt-1 flex justify-end gap-0.5 border-t border-[var(--color-border)]/60 pt-1 opacity-80 transition group-hover:opacity-100">
                    <CatalogActionButton label={t("characterCatalog.preview")} icon={<Eye size={13} />} onClick={(event) => { previewTriggerRef.current = event.currentTarget; setPreviewEntry(entry); }} disabled={pendingAction !== null} />
                    {entry.source === "instance" && (
                      <>
                        <CatalogActionButton label={t("characterCatalog.edit")} icon={<FilePenLine size={13} />} onClick={() => void runAction({ type: "edit", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                        <CatalogActionButton label={t("characterCatalog.duplicate")} icon={<Copy size={13} />} onClick={() => void runAction({ type: "duplicate", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                        {entry.hasTemplate && (
                          <CatalogActionButton label={t("characterCatalog.restoreDefault")} icon={<RefreshCcw size={13} />} onClick={() => void runAction({ type: "restore-default", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                        )}
                        {entry.hasTemplate && entry.templateVersion !== null && entry.availableTemplateVersion !== null && compareCharacterTemplateVersions(entry.availableTemplateVersion, entry.templateVersion) > 0 && (
                          <CatalogActionButton label={t("characterCatalog.resolveConflict")} icon={<Sparkles size={13} />} onClick={() => void runAction({ type: "resolve-conflict", characterId: entry.actionId }, entry.name)} disabled={pendingAction !== null} />
                        )}
                      </>
                    )}
                  </div>
                </article>
              );
            })}
          </div>

          {props.catalogError && <CatalogErrorNotice message={props.catalogError} isRetrying={props.isRetrying ?? false} onRetry={props.onRetry} t={t} />}
          {props.actionError && <CatalogErrorNotice message={props.actionError} isRetrying={false} t={t} />}
          {error !== null && <p role="alert" className="border-t border-red-400/20 bg-red-500/10 px-4 py-2 text-[10px] text-red-200">{error}</p>}
        </motion.section>
      )}
    </AnimatePresence>

      {previewEntry !== null && (
        <div
          className="fixed inset-0 z-[80] flex items-center justify-center bg-black/45 p-4 backdrop-blur-sm"
          role="presentation"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target) closePreview();
          }}
        >
          <section ref={previewDialogRef} role="dialog" aria-modal="true" aria-labelledby="character-preview-title" className="pointer-events-auto w-full max-w-md overflow-hidden rounded-2xl border border-[var(--color-border-accent)]/60 bg-[var(--color-bg-surface)]/95 shadow-[0_24px_80px_rgba(0,0,0,0.58)]">
            <header className="flex items-start justify-between border-b border-[var(--color-border)] px-5 py-4">
              <div className="min-w-0">
                <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-[var(--color-accent)]">{t("characterCatalog.preview")}</p>
                <h2 id="character-preview-title" className="mt-1 truncate text-base font-semibold text-[var(--color-text-primary)]">{previewEntry.name}</h2>
              </div>
              <button ref={previewCloseButtonRef} type="button" onClick={closePreview} aria-label={t("characterCatalog.close")} className="rounded-lg p-1.5 text-[var(--color-text-muted)] hover:bg-white/5 hover:text-[var(--color-text-primary)]">
                <X size={16} aria-hidden="true" />
              </button>
            </header>
            <div className="max-h-[min(560px,calc(100vh-180px))] space-y-4 overflow-y-auto p-5 scrollable">
              <PreviewField label={t("characterCatalog.description")} value={previewEntry.description} />
              <PreviewField label={t("characterCatalog.persona")} value={previewEntry.persona} />
              <PreviewField label={t("characterCatalog.greeting")} value={previewEntry.greeting} />
              <PreviewField label={t("characterCatalog.exampleDialogue")} value={previewEntry.exampleDialogue} />
              {(previewEntry.author !== null || previewEntry.availableTemplateVersion !== null) && (
                <p className="border-t border-[var(--color-border)] pt-3 text-[10px] text-[var(--color-text-muted)]">
                  {[previewEntry.author, previewEntry.availableTemplateVersion ? `v${previewEntry.availableTemplateVersion}` : null].filter(Boolean).join(" · ")}
                </p>
              )}
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

function CatalogErrorNotice(props: Readonly<{ message: string; isRetrying: boolean; onRetry?: () => void; t: Translate }>) {
  return (
    <div role="alert" className="mt-2 rounded-lg border border-red-400/20 bg-red-500/10 px-4 py-2 text-[10px] text-red-200">
      <p>{props.message}</p>
      {props.onRetry && (
        <button type="button" onClick={props.onRetry} disabled={props.isRetrying} className="mt-2 rounded border border-red-300/30 px-2 py-1 text-red-100 disabled:opacity-50">
          {props.isRetrying ? props.t("characterCatalog.retrying", { defaultValue: "Retrying..." }) : props.t("characterCatalog.retry", { defaultValue: "Retry" })}
        </button>
      )}
    </div>
  );
}

function PreviewField(props: Readonly<{ label: string; value: string }>) {
  if (!props.value.trim()) return null;
  return (
    <div>
      <p className="mb-1 text-[10px] font-semibold uppercase tracking-[0.16em] text-[var(--color-text-muted)]">{props.label}</p>
      <p className="whitespace-pre-wrap break-words text-xs leading-5 text-[var(--color-text-secondary)]">{props.value}</p>
    </div>
  );
}

type CatalogActionButtonProps = {
  readonly label: string;
  readonly icon: React.ReactNode;
  readonly onClick: (event: React.MouseEvent<HTMLButtonElement>) => void;
  readonly disabled: boolean;
};

function CatalogActionButton(props: Readonly<CatalogActionButtonProps>) {
  return (
    <button type="button" title={props.label} aria-label={props.label} onClick={props.onClick} disabled={props.disabled} className="rounded-md p-1.5 text-[var(--color-text-muted)] transition hover:bg-white/5 hover:text-[var(--color-accent)] disabled:opacity-40">
      {props.icon}
    </button>
  );
}
