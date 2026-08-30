// pattern: Functional Core

import { describe, expect, it } from "vitest";

import type { RegistryEntry } from "@/lib/kokoro-bridge";

import {
  CONTENT_LIBRARY_OFFICIAL_REGISTRY,
  createContentLibraryState,
  getActionableContentError,
  getContentTrustLabel,
  getContentVersionState,
  getSafePreviewUrl,
  getUrlInstallWarning,
  reduceContentLibraryState,
  selectRegistryEntries,
  type ContentLibraryEvent,
} from "./content-library-state";

const character: RegistryEntry = {
  content_type: "character",
  id: "kokoro",
  name: "Kokoro",
  version: "1.0.0",
  author: "Kokoro Engine",
  description: "A warm companion",
  preview: ["https://example.test/kokoro.png"],
  engine_version: ">=0.3.1, <0.4.0",
  download_url: "https://example.test/kokoro.zip",
  archive_size: 123,
  sha256: "a".repeat(64),
  trust: "official",
  trust_source: CONTENT_LIBRARY_OFFICIAL_REGISTRY,
  registry_identity: "github.com/chyinan/Kokoro-Engine/registry-v1",
  permissions: [],
  recommendations: { vision: false, memory: true, mcp_servers: [], bot_platforms: [] },
};

const mod: RegistryEntry = {
  ...character,
  content_type: "mod",
  id: "night-theme",
  name: "Night Theme",
  trust: "community",
  trust_source: "https://community.example/registry.json",
  registry_identity: undefined,
  permissions: ["theme", "components"],
  recommendations: { vision: false, memory: false, mcp_servers: [], bot_platforms: [] },
};

describe("content library state", () => {
  it("keeps Character and MOD entries in separate views", () => {
    expect(selectRegistryEntries([character, mod], "character")).toEqual([character]);
    expect(selectRegistryEntries([character, mod], "mod")).toEqual([mod]);
  });

  it("only calls an entry official when source is the canonical registry", () => {
    expect(getContentTrustLabel(character)).toBe("official");
    expect(getContentTrustLabel({ ...character, trust_source: "https://evil.example/index.json" })).toBe("community");
    expect(getContentTrustLabel({ ...character, trust: "unverified", registry_identity: undefined })).toBe("unverified");
    expect(getContentTrustLabel(character, "https://dev.example/registry.json")).toBe("community");
  });

  it("reports install, current, and update states from installed versions", () => {
    expect(getContentVersionState(character, null)).toBe("available");
    expect(getContentVersionState(character, "1.0.0")).toBe("installed");
    expect(getContentVersionState({ ...character, version: "1.1.0" }, "1.0.0")).toBe("update-available");
  });

  it("transitions through install, update, remove, and recoverable error states", () => {
    let state = createContentLibraryState();
    const events: ContentLibraryEvent[] = [
      { type: "operation-started", operation: "install", contentType: "character", entryId: character.id },
      { type: "operation-succeeded", operation: "install", contentType: "character", entryId: character.id, version: character.version },
      { type: "operation-started", operation: "update", contentType: "character", entryId: character.id },
      { type: "operation-failed", operation: "update", contentType: "character", entryId: character.id, error: "checksum mismatch" },
      { type: "error-dismissed" },
      { type: "operation-started", operation: "remove", contentType: "character", entryId: character.id },
      { type: "operation-succeeded", operation: "remove", contentType: "character", entryId: character.id },
    ];
    for (const event of events) state = reduceContentLibraryState(state, event);
    expect(state.pending).toBeNull();
    expect(state.installedVersions[character.id]).toBeUndefined();
    expect(state.error).toBeNull();
  });

  it("requires explicit confirmation for URL installs and explains trust implications", () => {
    const warning = getUrlInstallWarning("https://community.example/kokoro.zip");
    expect(warning).toContain("untrusted");
    let state = createContentLibraryState();
    state = reduceContentLibraryState(state, {
      type: "url-warning-opened",
      url: "https://community.example/kokoro.zip",
      contentType: "character",
    });
    expect(state.urlWarning).toEqual({ url: "https://community.example/kokoro.zip", contentType: "character" });
    state = reduceContentLibraryState(state, { type: "url-warning-dismissed" });
    expect(state.urlWarning).toBeNull();
  });

  it("keeps URL installation scoped to the selected content tab", () => {
    let state = createContentLibraryState();
    state = reduceContentLibraryState(state, { type: "tab-selected", tab: "mod" });
    state = reduceContentLibraryState(state, {
      type: "url-warning-opened",
      url: "https://community.example/theme.zip",
      contentType: "mod",
    });
    expect(state.urlWarning).toEqual({ url: "https://community.example/theme.zip", contentType: "mod" });
  });

  it("refreshes installed versions from authoritative package state", () => {
    let state = createContentLibraryState();
    state = reduceContentLibraryState(state, {
      type: "installed-refreshed",
      packages: [
        { contentType: "character", id: "kokoro", version: "1.0.0" },
        { contentType: "mod", id: "night-theme", version: "2.0.0" },
      ],
    });
    expect(state.installedVersions).toEqual({ "character:kokoro": "1.0.0", "mod:night-theme": "2.0.0" });
  });

  it("ignores malformed installed package snapshots before they become UI state", () => {
    let state = createContentLibraryState({
      kokoro: "1.0.0",
      "mod:night-theme": "1.0.0-beta.1",
      "character:../escape": "1.0.0",
      "mod:unsafe": "1.0.0-01",
    });
    expect(state.installedVersions).toEqual({
      "character:kokoro": "1.0.0",
      "mod:night-theme": "1.0.0-beta.1",
    });

    state = reduceContentLibraryState(state, {
      type: "installed-refreshed",
      packages: [
        { contentType: "character", id: "valid", version: "2.1.0" },
        { contentType: "character", id: "valid", version: "2.0.0" },
        { contentType: "character", id: "../escape", version: "3.0.0" },
        { contentType: "mod", id: "unsafe", version: "1.0.0-01" },
      ],
    });
    expect(state.installedVersions).toEqual({ "character:valid": "2.1.0" });
  });

  it("keeps same IDs isolated between Character and MOD packages", () => {
    let state = createContentLibraryState();
    state = reduceContentLibraryState(state, {
      type: "installed-refreshed",
      packages: [
        { contentType: "character", id: "shared", version: "1.0.0" },
        { contentType: "mod", id: "shared", version: "2.0.0" },
      ],
    });
    expect(state.installedVersions).toEqual({ "character:shared": "1.0.0", "mod:shared": "2.0.0" });

    state = reduceContentLibraryState(state, {
      type: "operation-started",
      operation: "remove",
      contentType: "character",
      entryId: "shared",
    });
    expect(state.pending).toEqual({ operation: "remove", contentType: "character", entryId: "shared", key: "character:shared" });
    state = reduceContentLibraryState(state, {
      type: "operation-succeeded",
      operation: "remove",
      contentType: "character",
      entryId: "shared",
    });
    expect(state.installedVersions).toEqual({ "mod:shared": "2.0.0" });
  });

  it("accepts only safe HTTPS preview URLs", () => {
    expect(getSafePreviewUrl("https://cdn.example.test/avatar.webp")).toBe("https://cdn.example.test/avatar.webp");
    expect(getSafePreviewUrl("assets/avatar.webp", "https://cdn.example.test/packages/kokoro-1.0.0.zip")).toBeNull();
    expect(getSafePreviewUrl("http://cdn.example.test/avatar.webp")).toBeNull();
    expect(getSafePreviewUrl("javascript:alert(1)")).toBeNull();
    expect(getSafePreviewUrl("data:image/svg+xml,<svg/onload=alert(1)>")).toBeNull();
    expect(getSafePreviewUrl("https://user:secret@cdn.example.test/avatar.webp")).toBeNull();
    expect(getSafePreviewUrl("//evil.example.test/avatar.webp", "https://cdn.example.test/packages/kokoro-1.0.0.zip")).toBeNull();
    expect(getSafePreviewUrl("%2e%2e/outside.webp", "https://cdn.example.test/packages/kokoro-1.0.0.zip")).toBeNull();
  });

  it("orders registry versions according to SemVer prerelease precedence", () => {
    expect(getContentVersionState({ ...character, version: "1.0.0" }, "1.0.0-beta.1")).toBe("update-available");
    expect(getContentVersionState({ ...character, version: "1.0.0-beta.10" }, "1.0.0-beta.2")).toBe("update-available");
    expect(getContentVersionState({ ...character, version: "1.0.0-beta.2" }, "1.0.0-beta.10")).toBe("installed");
    expect(getContentVersionState({ ...character, version: "1.0.0+build.2" }, "1.0.0+build.1")).toBe("installed");
  });

  it("rejects SemVer numeric components that overflow the Rust u64 contract", () => {
    expect(selectRegistryEntries([
      { ...character, version: "18446744073709551616.0.0" },
      { ...character, version: "1.0.0-18446744073709551616" },
    ], "character")).toEqual([]);
  });

  it("turns transport and compatibility failures into actionable recovery text", () => {
    expect(getActionableContentError(new Error("network timeout"))).toMatchObject({
      message: expect.stringContaining("download"),
      action: "retry",
    });
    expect(getActionableContentError({ message: "network timeout" })).toMatchObject({
      message: expect.stringContaining("download"),
      action: "retry",
    });
    expect(getActionableContentError("incompatible engine version")).toMatchObject({
      message: expect.stringContaining("compatible"),
      action: "review",
    });
  });
});
