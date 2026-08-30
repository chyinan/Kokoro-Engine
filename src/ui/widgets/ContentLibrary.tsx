// pattern: Imperative Shell

import { AlertCircle, Download, ExternalLink, Package, RefreshCw, Trash2, Upload, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  installCharacterFromRegistry,
  installCharacterFromUrl,
  installModFromRegistry,
  listInstalledContent,
  listRegistryEntries,
  removeCharacterPackage,
  safeInvoke,
  type InstalledCharacterPackage,
  type InstalledContentPackage,
  type ModManifest,
  type RegistryEntry,
  type RegistryIndex,
} from "@/lib/kokoro-bridge";

import {
  createContentLibraryState,
  getContentTrustLabel,
  getContentVersionState,
  getSafePreviewUrl,
  getUrlInstallWarning,
  reduceContentLibraryState,
  selectRegistryEntries,
  type ContentLibraryEvent,
  type ContentLibraryState,
  type ContentLibraryTab,
  type ContentOperation,
} from "./content-library-state";

export type ContentLibraryDependencies = {
  readonly listRegistry: (registryUrl?: string) => Promise<RegistryIndex>;
  readonly listInstalled: () => Promise<Array<InstalledContentPackage>>;
  readonly installCharacter: (characterId: string, version: string, registryUrl?: string) => Promise<InstalledCharacterPackage>;
  readonly installCharacterFromUrl?: (url: string) => Promise<InstalledCharacterPackage>;
  readonly installMod: (entry: Readonly<RegistryEntry>, permissionConfirmed: boolean, registryUrl?: string) => Promise<ModManifest | void>;
  readonly installModFromUrl?: (url: string, permissionConfirmed: boolean) => Promise<ModManifest | void>;
  readonly update: (entry: Readonly<RegistryEntry>, permissionConfirmed: boolean, registryUrl?: string) => Promise<unknown>;
  readonly removeCharacter?: (entry: Readonly<RegistryEntry>) => Promise<unknown>;
  readonly remove: (entry: Readonly<RegistryEntry>) => Promise<unknown>;
};

export type ContentLibraryProps = {
  readonly dependencies?: Readonly<ContentLibraryDependencies>;
  readonly installedVersions?: Readonly<Record<string, string>>;
  readonly registryUrl?: string;
};

const defaultDependencies: ContentLibraryDependencies = {
  listRegistry: listRegistryEntries,
  listInstalled: listInstalledContent,
  installCharacter: installCharacterFromRegistry,
  installCharacterFromUrl,
  installMod: async (entry, permissionConfirmed, registryUrl) => installModFromRegistry({ entry, registryUrl, permissionConfirmed }),
  installModFromUrl: async (url, permissionConfirmed) => safeInvoke<ModManifest>("install_mod_from_url", { url, confirmUntrustedCode: permissionConfirmed }),
  update: async (entry, permissionConfirmed, registryUrl) => installModFromRegistry({ entry, registryUrl, permissionConfirmed }),
  removeCharacter: async (entry) => removeCharacterPackage(entry.id, entry.version),
  remove: async (entry) => safeInvoke<void>("remove_mod", { modId: entry.id }),
};

function mergeDependencies(
  dependencies: Readonly<ContentLibraryDependencies> | undefined,
): ContentLibraryDependencies {
  return { ...defaultDependencies, ...dependencies };
}

function recommendationLabels(entry: Readonly<RegistryEntry>, translate: (key: string, options?: { readonly defaultValue?: string; readonly name?: string }) => string): Array<string> {
  const labels: string[] = [];
  if (entry.recommendations.vision) labels.push(translate("contentLibrary.recommendations.vision", { defaultValue: "Vision" }));
  if (entry.recommendations.memory) labels.push(translate("contentLibrary.recommendations.memory", { defaultValue: "Memory" }));
  labels.push(...entry.recommendations.mcp_servers.map((name) => translate("contentLibrary.recommendations.mcp", { defaultValue: `MCP: ${name}`, name })));
  labels.push(...entry.recommendations.bot_platforms.map((name) => translate("contentLibrary.recommendations.bot", { defaultValue: `Bot: ${name}`, name })));
  return labels;
}

export default function ContentLibrary(props: Readonly<ContentLibraryProps>) {
  const { t } = useTranslation();
  const dependencies = useMemo(() => mergeDependencies(props.dependencies), [props.dependencies]);
  const [state, setState] = useState<ContentLibraryState>(() => createContentLibraryState(props.installedVersions));
  const [entries, setEntries] = useState<RegistryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [url, setUrl] = useState("");
  const [lastAction, setLastAction] = useState<(() => Promise<void>) | null>(null);
  const [permissionTarget, setPermissionTarget] = useState<{ readonly entry: RegistryEntry; readonly operation: "install" | "update" } | null>(null);

  const dispatch = (event: ContentLibraryEvent) => setState((current) => reduceContentLibraryState(current, event));

  const loadRegistry = async (): Promise<void> => {
    setLoading(true);
    try {
      const index = await dependencies.listRegistry(props.registryUrl);
      setEntries(index.entries);
      dispatch({ type: "error-dismissed" });
    } catch (error) {
      setLastAction(() => loadRegistry);
      dispatch({ type: "operation-failed", operation: "install", entryId: "registry", error });
    } finally {
      setLoading(false);
    }
  };

  const loadInstalled = async (): Promise<void> => {
    try {
      const packages = await dependencies.listInstalled();
      dispatch({
        type: "installed-refreshed",
        packages: packages.map((packageEntry) => ({
          contentType: packageEntry.content_type,
          id: packageEntry.id,
          version: packageEntry.version,
        })),
      });
    } catch (error) {
      setLastAction(() => loadInstalled);
      dispatch({ type: "operation-failed", operation: "install", entryId: "installed", error });
    }
  };

  useEffect(() => {
    void loadRegistry();
    void loadInstalled();
    // The caller owns dependency identity; loading once per registry endpoint avoids request loops.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.registryUrl]);

  const runOperation = async (entry: Readonly<RegistryEntry>, operation: ContentOperation, permissionConfirmed = false): Promise<void> => {
    if (entry.content_type === "mod" && (operation === "install" || operation === "update") && !permissionConfirmed) {
      setPermissionTarget({ entry: { ...entry }, operation });
      return;
    }
    const action = async (): Promise<void> => {
      dispatch({ type: "operation-started", operation, entryId: entry.id });
      try {
        if (operation === "install") {
          if (entry.content_type === "character") {
            const installed = await dependencies.installCharacter(entry.id, entry.version, props.registryUrl);
            dispatch({ type: "operation-succeeded", operation, entryId: entry.id, version: installed.version || entry.version });
          } else {
            await dependencies.installMod(entry, permissionConfirmed, props.registryUrl);
            dispatch({ type: "operation-succeeded", operation, entryId: entry.id, version: entry.version });
          }
        } else if (operation === "update") {
          if (entry.content_type === "character") {
            const installed = await dependencies.installCharacter(entry.id, entry.version, props.registryUrl);
            dispatch({ type: "operation-succeeded", operation, entryId: entry.id, version: installed.version || entry.version });
          } else {
            await dependencies.update(entry, permissionConfirmed, props.registryUrl);
            dispatch({ type: "operation-succeeded", operation, entryId: entry.id, version: entry.version });
          }
        } else {
          if (entry.content_type === "character" && dependencies.removeCharacter) {
            await dependencies.removeCharacter(entry);
          } else {
            await dependencies.remove(entry);
          }
          dispatch({ type: "operation-succeeded", operation, entryId: entry.id });
        }
      } catch (error) {
        dispatch({ type: "operation-failed", operation, entryId: entry.id, error });
      }
    };
    setLastAction(() => action);
    await action();
  };

  const confirmUrlInstall = async (): Promise<void> => {
    const target = state.urlWarning;
    dispatch({ type: "url-warning-dismissed" });
    if (!target) return;
    dispatch({ type: "operation-started", operation: "install", entryId: `url:${target.url}` });
    try {
      if (state.urlWarning?.contentType === "mod") {
        if (!dependencies.installModFromUrl) throw new Error("MOD URL installation is unavailable");
        const installed = await dependencies.installModFromUrl(target.url, true);
        dispatch({ type: "operation-succeeded", operation: "install", entryId: `url:${target.url}`, version: installed && "version" in installed ? installed.version : undefined });
      } else {
        if (!dependencies.installCharacterFromUrl) throw new Error("character URL installation is unavailable");
        const installed = await dependencies.installCharacterFromUrl(target.url);
        dispatch({ type: "operation-succeeded", operation: "install", entryId: installed.id, version: installed.version });
      }
      setUrl("");
    } catch (error) {
      dispatch({ type: "operation-failed", operation: "install", entryId: `url:${target.url}`, error });
    }
  };

  const visibleEntries = selectRegistryEntries(entries, state.activeTab);
  const emptyText = state.activeTab === "character"
    ? t("contentLibrary.emptyCharacters", { defaultValue: "No character packages in this registry." })
    : t("contentLibrary.emptyMods", { defaultValue: "No MOD packages in this registry." });

  return (
    <section data-content-library className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface)]/95 text-[var(--color-text-primary)] shadow-lg backdrop-blur-xl">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--color-border)] px-4 py-3">
        <div className="flex items-center gap-2">
          <Package size={17} className="text-[var(--color-accent)]" aria-hidden="true" />
          <div>
            <p className="text-[10px] uppercase tracking-[0.2em] text-[var(--color-accent)]">{t("contentLibrary.eyebrow", { defaultValue: "Content library" })}</p>
            <h2 className="text-sm font-semibold">{t("contentLibrary.title", { defaultValue: "Characters & MODs" })}</h2>
          </div>
        </div>
        <button type="button" data-content-action="refresh" onClick={() => void loadRegistry()} disabled={loading} className="rounded-lg p-2 text-[var(--color-text-muted)] hover:text-[var(--color-accent)] disabled:opacity-50" aria-label={t("contentLibrary.refresh", { defaultValue: "Refresh registry" })}>
          <RefreshCw size={15} className={loading ? "animate-spin" : ""} aria-hidden="true" />
        </button>
      </header>

      <div className="flex items-center gap-1 border-b border-[var(--color-border)] px-3 py-2">
        {(["character", "mod"] as const).map((tab: ContentLibraryTab) => (
          <button key={tab} type="button" data-content-tab={tab} aria-pressed={state.activeTab === tab} onClick={() => dispatch({ type: "tab-selected", tab })} className={`rounded-lg px-3 py-1.5 text-xs font-semibold uppercase tracking-[0.14em] transition ${state.activeTab === tab ? "bg-[var(--color-accent)]/15 text-[var(--color-accent)]" : "text-[var(--color-text-muted)] hover:bg-white/5"}`}>
            {tab === "character" ? t("contentLibrary.characters", { defaultValue: "Characters" }) : t("contentLibrary.mods", { defaultValue: "MODs" })}
          </button>
        ))}
      </div>

      <div className="border-b border-[var(--color-border)] px-3 py-2">
        <form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (url.trim()) dispatch({ type: "url-warning-opened", url: url.trim(), contentType: state.activeTab }); }}>
          <input value={url} onChange={(event) => setUrl(event.target.value)} data-content-url-input placeholder={t(state.activeTab === "mod" ? "contentLibrary.modUrlPlaceholder" : "contentLibrary.urlPlaceholder", { defaultValue: state.activeTab === "mod" ? "Install a MOD from an HTTPS URL" : "Install a character package from an HTTPS URL" })} className="min-w-0 flex-1 rounded-lg border border-[var(--color-border)] bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-[var(--color-text-muted)] focus:border-[var(--color-accent)]" />
          <button type="submit" data-content-action="install-url" disabled={!url.trim() || state.pending !== null} className="flex items-center gap-1 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-text-secondary)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] disabled:opacity-40">
            <Upload size={13} aria-hidden="true" /> {t("contentLibrary.installUrl", { defaultValue: "Install URL" })}
          </button>
        </form>
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-3">
        {loading && entries.length === 0 && <p className="py-8 text-center text-xs text-[var(--color-text-muted)]">{t("contentLibrary.loading", { defaultValue: "Loading registry…" })}</p>}
        {!loading && visibleEntries.length === 0 && <p className="py-8 text-center text-xs text-[var(--color-text-muted)]">{emptyText}</p>}
        {visibleEntries.map((entry) => {
          const trust = getContentTrustLabel(entry, props.registryUrl ?? undefined);
          const versionState = getContentVersionState(entry, state.installedVersions[entry.id]);
          const pending = state.pending?.entryId === entry.id;
          const recommendations = recommendationLabels(entry, t);
          const action = versionState === "available" ? "install" : versionState === "update-available" ? "update" : "remove";
          return (
            <article key={`${entry.content_type}:${entry.id}`} data-content-entry={entry.id} className="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-elevated)]/70 p-3">
              <div className="flex gap-3">
                <div className="h-16 w-16 shrink-0 overflow-hidden rounded-lg border border-[var(--color-border)] bg-black/20">
                  {getSafePreviewUrl(entry.preview[0] ?? "") ? <img src={getSafePreviewUrl(entry.preview[0] ?? "") ?? undefined} alt="" className="h-full w-full object-cover" /> : <div className="grid h-full w-full place-items-center text-[var(--color-text-muted)]"><Package size={20} aria-hidden="true" /></div>}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="truncate text-sm font-semibold">{entry.name}</h3>
                    <span data-content-trust={entry.id} className="rounded-full border border-[var(--color-border)] px-1.5 py-0.5 text-[9px] uppercase tracking-wider text-[var(--color-text-muted)]">{trust}</span>
                    <span data-content-status={entry.id} className="text-[9px] uppercase tracking-wider text-[var(--color-text-muted)]">{versionState === "installed" ? t("contentLibrary.statusInstalled", { defaultValue: "installed" }) : versionState === "update-available" ? t("contentLibrary.statusUpdate", { defaultValue: "update available" }) : t("contentLibrary.statusAvailable", { defaultValue: "available" })}</span>
                  </div>
                  <p className="mt-0.5 text-[10px] text-[var(--color-text-muted)]">v{entry.version} · {entry.author}</p>
                  <p className="mt-1 line-clamp-2 text-xs leading-5 text-[var(--color-text-secondary)]">{entry.description}</p>
                </div>
                <button type="button" data-content-action={`${action}:${entry.id}`} onClick={() => void runOperation(entry, action)} disabled={pending || state.pending !== null} className="flex h-fit items-center gap-1 rounded-lg border border-[var(--color-border)] px-2.5 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] disabled:opacity-45">
                  {pending ? <RefreshCw size={12} className="animate-spin" aria-hidden="true" /> : action === "install" ? <Download size={12} aria-hidden="true" /> : action === "update" ? <RefreshCw size={12} aria-hidden="true" /> : <Trash2 size={12} aria-hidden="true" />}
                  {pending ? t("contentLibrary.working", { defaultValue: "Working" }) : t(`contentLibrary.${action}`, { defaultValue: action })}
                </button>
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-[var(--color-text-muted)]">
                <span data-content-compatibility={entry.id}>{t("contentLibrary.compatibility", { defaultValue: "Engine" })}: {entry.engine_version}</span>
                {entry.permissions.map((permission) => <span key={permission} data-content-permission={permission} className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-200">{permission}</span>)}
                {recommendations.map((recommendation) => <span key={recommendation} data-content-recommendation={recommendation} className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-200">{recommendation}</span>)}
                <a href={entry.download_url} target="_blank" rel="noreferrer" className="ml-auto inline-flex items-center gap-1 hover:text-[var(--color-accent)]"><ExternalLink size={11} aria-hidden="true" /> {t("contentLibrary.source", { defaultValue: "Source" })}</a>
              </div>
            </article>
          );
        })}
      </div>

      {state.error && <div role="alert" className="flex items-center justify-between gap-2 border-t border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-100"><span className="flex items-center gap-2"><AlertCircle size={14} aria-hidden="true" />{t(state.error.messageKey, { defaultValue: state.error.message })}</span><div className="flex items-center gap-1">{state.error.action === "retry" && lastAction && <button type="button" data-content-action="retry" onClick={() => void lastAction()} className="rounded px-2 py-1 text-[10px] font-semibold uppercase hover:bg-white/10">{t("contentLibrary.retry", { defaultValue: "Retry" })}</button>}<button type="button" data-content-action="dismiss-error" onClick={() => dispatch({ type: "error-dismissed" })} aria-label={t("contentLibrary.dismissError", { defaultValue: "Dismiss error" })} className="rounded p-1 hover:bg-white/10"><X size={13} aria-hidden="true" /></button></div></div>}

      {permissionTarget && <div role="dialog" aria-modal="true" className="absolute inset-0 z-10 grid place-items-center bg-black/60 p-4"><div className="w-full max-w-md rounded-xl border border-amber-400/40 bg-[var(--color-bg-surface)] p-4 shadow-2xl"><h3 className="text-sm font-semibold text-amber-100">{t("contentLibrary.permissionWarningTitle", { defaultValue: "Review MOD permissions" })}</h3><p className="mt-2 text-xs leading-5 text-[var(--color-text-secondary)]">{t("contentLibrary.permissionWarning", { defaultValue: "This MOD contains executable code and requests permissions. Review the registry entry before installing." })}</p><div className="mt-4 flex justify-end gap-2"><button type="button" data-content-action="cancel-permission" onClick={() => setPermissionTarget(null)} className="rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-xs text-[var(--color-text-secondary)]">{t("contentLibrary.cancel", { defaultValue: "Cancel" })}</button><button type="button" data-content-action="confirm-permission" onClick={() => { const target = permissionTarget; setPermissionTarget(null); void runOperation(target.entry, target.operation, true); }} className="rounded-lg bg-amber-400 px-3 py-1.5 text-xs font-semibold text-black">{t("contentLibrary.confirmPermissions", { defaultValue: "Review and install" })}</button></div></div></div>}
      {state.urlWarning && <div role="dialog" aria-modal="true" className="absolute inset-0 z-10 grid place-items-center bg-black/60 p-4"><div className="w-full max-w-md rounded-xl border border-amber-400/40 bg-[var(--color-bg-surface)] p-4 shadow-2xl"><h3 className="text-sm font-semibold text-amber-100">{t(state.urlWarning.contentType === "mod" ? "contentLibrary.modUrlWarningTitle" : "contentLibrary.urlWarningTitle", { defaultValue: "Review external package" })}</h3><p className="mt-2 text-xs leading-5 text-[var(--color-text-secondary)]">{t("contentLibrary.urlWarning", { defaultValue: getUrlInstallWarning(state.urlWarning.url), url: state.urlWarning.url })}</p><div className="mt-4 flex justify-end gap-2"><button type="button" data-content-action="cancel-url" onClick={() => dispatch({ type: "url-warning-dismissed" })} className="rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-xs text-[var(--color-text-secondary)]">{t("contentLibrary.cancel", { defaultValue: "Cancel" })}</button><button type="button" data-content-action="confirm-url" onClick={() => void confirmUrlInstall()} className="rounded-lg bg-amber-400 px-3 py-1.5 text-xs font-semibold text-black">{t("contentLibrary.confirmUrl", { defaultValue: "Trust and install" })}</button></div></div></div>}
    </section>
  );
}
