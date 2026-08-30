// pattern: Functional Core

import { describe, expect, it } from "vitest";

import type { RegistryEntry } from "@/lib/kokoro-bridge";

import {
  CONTENT_LIBRARY_OFFICIAL_REGISTRY,
  createContentLibraryState,
  getActionableContentError,
  getContentTrustLabel,
  getContentVersionState,
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
      { type: "operation-started", operation: "install", entryId: character.id },
      { type: "operation-succeeded", operation: "install", entryId: character.id, version: character.version },
      { type: "operation-started", operation: "update", entryId: character.id },
      { type: "operation-failed", operation: "update", entryId: character.id, error: "checksum mismatch" },
      { type: "error-dismissed" },
      { type: "operation-started", operation: "remove", entryId: character.id },
      { type: "operation-succeeded", operation: "remove", entryId: character.id },
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
    state = reduceContentLibraryState(state, { type: "url-warning-opened", url: "https://community.example/kokoro.zip" });
    expect(state.urlWarning).toBe("https://community.example/kokoro.zip");
    state = reduceContentLibraryState(state, { type: "url-warning-dismissed" });
    expect(state.urlWarning).toBeNull();
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
