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
    const separator = key.indexOf(":");
    const contentType = separator === -1 ? "character" : key.slice(0, separator);
    const id = separator === -1 ? key : key.slice(separator + 1);
    if (isValidContentType(contentType) && isValidContentId(id) && isValidContentVersion(version)) {
      normalizedInstalledVersions[contentKey(contentType, id)] = version;
    }
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
      } else if (event.version && isValidContentId(event.entryId) && isValidContentVersion(event.version)) {
        installedVersions[key] = event.version;
      }
      return { ...state, pending: null, installedVersions, error: null };
    }
    case "installed-refreshed": {
      const installedVersions: Record<string, string> = {};
      for (const packageEntry of event.packages) {
        if (isValidContentType(packageEntry.contentType)
          && isValidContentId(packageEntry.id)
          && isValidContentVersion(packageEntry.version)) {
          const key = contentKey(packageEntry.contentType, packageEntry.id);
          const previous = installedVersions[key];
          if (!previous || compareContentVersions(packageEntry.version, previous) > 0) {
            installedVersions[key] = packageEntry.version;
          }
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
  return entries.filter((entry) => entry.content_type === tab && isValidContentId(entry.id) && isValidContentVersion(entry.version));
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

type ParsedContentVersion = {
  readonly core: readonly [bigint, bigint, bigint];
  readonly prerelease: ReadonlyArray<bigint | string>;
};

const CONTENT_ID_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;

function isValidContentType(value: string): value is ContentLibraryTab {
  return value === "character" || value === "mod";
}

function isValidContentId(value: string): boolean {
  return CONTENT_ID_PATTERN.test(value);
}

function parseContentVersion(value: string): ParsedContentVersion | null {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(value);
  if (!match) return null;
  const prerelease = (match[4] ?? "").split(".").filter(Boolean).map((identifier) => {
    if (/^\d+$/.test(identifier)) {
      if (identifier.length > 1 && identifier.startsWith("0")) return null;
      return BigInt(identifier);
    }
    return identifier;
  });
  if (prerelease.some((identifier): identifier is null => identifier === null)) {
    return null;
  }
  return {
    core: [BigInt(match[1]), BigInt(match[2]), BigInt(match[3])],
    prerelease: prerelease as ReadonlyArray<bigint | string>,
  };
}

function isValidContentVersion(value: string): boolean {
  return parseContentVersion(value) !== null;
}

/** Compare validated registry versions using SemVer 2.0 precedence rules. */
export function compareContentVersions(left: string, right: string): number {
  const a = parseContentVersion(left);
  const b = parseContentVersion(right);
  if (!a || !b) return left === right ? 0 : left < right ? -1 : 1;
  for (let index = 0; index < a.core.length; index += 1) {
    if (a.core[index] !== b.core[index]) return a.core[index] > b.core[index] ? 1 : -1;
  }
  if (a.prerelease.length === 0 || b.prerelease.length === 0) {
    if (a.prerelease.length === b.prerelease.length) return 0;
    return a.prerelease.length === 0 ? 1 : -1;
  }
  for (let index = 0; index < Math.max(a.prerelease.length, b.prerelease.length); index += 1) {
    const leftIdentifier = a.prerelease[index];
    const rightIdentifier = b.prerelease[index];
    if (leftIdentifier === undefined || rightIdentifier === undefined) {
      return leftIdentifier === undefined ? -1 : 1;
    }
    if (typeof leftIdentifier === "bigint" && typeof rightIdentifier === "bigint") {
      if (leftIdentifier !== rightIdentifier) return leftIdentifier > rightIdentifier ? 1 : -1;
    } else if (typeof leftIdentifier === "bigint") {
      return -1;
    } else if (typeof rightIdentifier === "bigint") {
      return 1;
    } else if (leftIdentifier !== rightIdentifier) {
      return leftIdentifier < rightIdentifier ? -1 : 1;
    }
  }
  return 0;
}

export function getContentVersionState(
  entry: Readonly<RegistryEntry>,
  installedVersion: string | null | undefined,
): ContentVersionState {
  if (!installedVersion) return "available";
  return compareContentVersions(entry.version, installedVersion) > 0 ? "update-available" : "installed";
}

export function getUrlInstallWarning(url: string): string {
  return `This URL is not part of the official registry. The package is untrusted until validation succeeds (${url}). Continue only if you trust the source.`;
}

export function getSafePreviewUrl(value: string, downloadUrl?: string): string | null {
  if (!value || value.trim() !== value || /\s/.test(value)) return null;
  try {
    const isAbsolute = value.startsWith("https://");
    if (!isAbsolute) {
      if (!downloadUrl || value.startsWith("/") || value.startsWith("//") || value.includes("\\") || value.includes(":") || value.includes("%")) return null;
      if (value.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) return null;
    }
    const base = downloadUrl ? new URL(downloadUrl) : undefined;
    if (base && (base.protocol !== "https:" || base.username !== "" || base.password !== "")) return null;
    const parsed = new URL(value, base);
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
