// pattern: Functional Core

import type { RegistryEntry } from "@/lib/kokoro-bridge";

export const CONTENT_LIBRARY_OFFICIAL_REGISTRY =
  "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json";
export const CONTENT_LIBRARY_OFFICIAL_IDENTITY = "github.com/chyinan/Kokoro-Engine/registry-v1";

export type ContentLibraryTab = "character" | "mod";
export type ContentOperation = "install" | "update" | "remove";
export type ContentVersionState = "available" | "installed" | "update-available";

export type ContentPendingOperation = {
  readonly operation: ContentOperation;
  readonly contentType: ContentLibraryTab;
  readonly entryId: string;
  readonly key: string;
};

export type ContentActionableError = {
  readonly message: string;
  readonly messageKey: string;
  readonly action: "retry" | "review";
};

export type ContentUrlWarning = {
  readonly url: string;
  readonly contentType: ContentLibraryTab;
};

export type InstalledContentPackage = {
  readonly contentType: ContentLibraryTab;
  readonly id: string;
  readonly version: string;
};

export type ContentLibraryState = {
  readonly activeTab: ContentLibraryTab;
  readonly pending: ContentPendingOperation | null;
  readonly installedVersions: Readonly<Record<string, string>>;
  readonly error: ContentActionableError | null;
  readonly urlWarning: ContentUrlWarning | null;
};

export type ContentLibraryEvent =
  | { readonly type: "tab-selected"; readonly tab: ContentLibraryTab }
  | { readonly type: "operation-started"; readonly operation: ContentOperation; readonly contentType: ContentLibraryTab; readonly entryId: string }
  | { readonly type: "operation-succeeded"; readonly operation: ContentOperation; readonly contentType: ContentLibraryTab; readonly entryId: string; readonly version?: string }
  | { readonly type: "operation-failed"; readonly operation: ContentOperation; readonly contentType: ContentLibraryTab; readonly entryId: string; readonly error: unknown }
  | { readonly type: "url-warning-opened"; readonly url: string; readonly contentType: ContentLibraryTab }
  | { readonly type: "installed-refreshed"; readonly packages: ReadonlyArray<InstalledContentPackage> }
  | { readonly type: "url-warning-dismissed" }
  | { readonly type: "error-dismissed" };

export function createContentLibraryState(
  installedVersions: Readonly<Record<string, string>> = {},
): ContentLibraryState {
  const normalizedInstalledVersions: Record<string, string> = {};
  for (const [key, version] of Object.entries(installedVersions)) {
    normalizedInstalledVersions[key.includes(":") ? key : contentKey("character", key)] = version;
  }
  return {
    activeTab: "character",
    pending: null,
    installedVersions: normalizedInstalledVersions,
    error: null,
    urlWarning: null,
  };
}

export function reduceContentLibraryState(
  state: ContentLibraryState,
  event: ContentLibraryEvent,
): ContentLibraryState {
  switch (event.type) {
    case "tab-selected":
      return { ...state, activeTab: event.tab };
    case "operation-started":
      return {
        ...state,
        pending: {
          operation: event.operation,
          contentType: event.contentType,
          entryId: event.entryId,
          key: contentKey(event.contentType, event.entryId),
        },
        error: null,
      };
    case "operation-succeeded": {
      const installedVersions = { ...state.installedVersions };
      const key = contentKey(event.contentType, event.entryId);
      if (event.operation === "remove") {
        delete installedVersions[key];
      } else if (event.version) {
        installedVersions[key] = event.version;
      }
      return { ...state, pending: null, installedVersions, error: null };
    }
    case "installed-refreshed": {
      const installedVersions: Record<string, string> = {};
      for (const packageEntry of event.packages) {
        if (packageEntry.id.trim() && packageEntry.version.trim()) {
          installedVersions[contentKey(packageEntry.contentType, packageEntry.id)] = packageEntry.version;
        }
      }
      return { ...state, installedVersions };
    }
    case "operation-failed":
      return {
        ...state,
        pending: null,
        error: getActionableContentError(event.error),
      };
    case "url-warning-opened":
      return { ...state, urlWarning: { url: event.url, contentType: event.contentType } };
    case "url-warning-dismissed":
      return { ...state, urlWarning: null };
    case "error-dismissed":
      return { ...state, error: null };
  }
}

export function selectRegistryEntries(
  entries: ReadonlyArray<RegistryEntry>,
  tab: ContentLibraryTab,
): Array<RegistryEntry> {
  return entries.filter((entry) => entry.content_type === tab);
}

export function contentKey(contentType: ContentLibraryTab, id: string): string {
  return `${contentType}:${id}`;
}

/** Never trust a self-asserted official label from a custom registry or URL. */
export function getContentTrustLabel(
  entry: Readonly<RegistryEntry>,
  registryUrl: string = CONTENT_LIBRARY_OFFICIAL_REGISTRY,
): "official" | "community" | "unverified" {
  if (
    entry.trust === "official" &&
    registryUrl === CONTENT_LIBRARY_OFFICIAL_REGISTRY &&
    entry.trust_source === CONTENT_LIBRARY_OFFICIAL_REGISTRY &&
    entry.registry_identity === CONTENT_LIBRARY_OFFICIAL_IDENTITY
  ) {
    return "official";
  }
  if (entry.trust === "unverified") return "unverified";
  return "community";
}

function compareVersions(left: string, right: string): number {
  const parse = (value: string): number[] => value.split(/[.+-]/).map((part) => Number.parseInt(part, 10) || 0);
  const a = parse(left);
  const b = parse(right);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

export function getContentVersionState(
  entry: Readonly<RegistryEntry>,
  installedVersion: string | null | undefined,
): ContentVersionState {
  if (!installedVersion) return "available";
  return compareVersions(entry.version, installedVersion) > 0 ? "update-available" : "installed";
}

export function getUrlInstallWarning(url: string): string {
  return `This URL is not part of the official registry. The package is untrusted until validation succeeds (${url}). Continue only if you trust the source.`;
}

export function getSafePreviewUrl(value: string): string | null {
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== "https:" || parsed.username !== "" || parsed.password !== "") return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

export function getActionableContentError(error: unknown): ContentActionableError {
  const raw = error instanceof Error
    ? error.message
    : typeof error === "object" && error !== null && "message" in error && typeof error.message === "string"
      ? error.message
      : String(error);
  const normalized = raw.toLowerCase();
  if (normalized.includes("incompatible") || normalized.includes("engine version")) {
    return {
      message: "This package is not compatible with the current engine. Review the compatibility range and choose another version.",
      messageKey: "contentLibrary.errors.incompatible",
      action: "review",
    };
  }
  if (normalized.includes("checksum") || normalized.includes("validation") || normalized.includes("corrupt")) {
    return {
      message: "Package validation failed. Check the source and checksum, then review before retrying.",
      messageKey: "contentLibrary.errors.validation",
      action: "review",
    };
  }
  if (normalized.includes("network") || normalized.includes("timeout") || normalized.includes("download")) {
    return {
      message: "download failed. Check your connection and retry.",
      messageKey: "contentLibrary.errors.download",
      action: "retry",
    };
  }
  return {
    message: "Content operation failed. Review the package and retry.",
    messageKey: "contentLibrary.errors.generic",
    action: "retry",
  };
}
