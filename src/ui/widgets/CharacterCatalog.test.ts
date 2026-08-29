// pattern: Imperative Shell

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  CharacterCatalog,
  executeCharacterCatalogAction,
  type CharacterCatalogActionDependencies,
} from "./CharacterCatalog";
import type { CharacterCapabilityRecommendations } from "./CharacterRecommendationDialog";
import type { CharacterRecord } from "@/lib/kokoro-bridge";

function recommendations(): CharacterCapabilityRecommendations {
  return {
    vision: true,
    memory: true,
    mcpServers: ["calendar"],
    botPlatforms: ["telegram"],
  };
}

function dependencies(
  overrides: Partial<CharacterCatalogActionDependencies> = {},
): CharacterCatalogActionDependencies {
  return {
    activateCharacter: vi.fn(async () => recommendations()),
    importCharacter: vi.fn(async () => undefined),
    editCharacter: vi.fn(async () => undefined),
    duplicateCharacter: vi.fn(async () => undefined),
    restoreCharacterDefaults: vi.fn(async () => undefined),
    resolveTemplateConflict: vi.fn(async () => undefined),
    ...overrides,
  };
}

describe("main character catalog actions", () => {
  it("renders a non-null avatar through the supplied resolver", () => {
    const character: CharacterRecord = {
      id: "custom",
      name: "Custom",
      persona: "A custom companion",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: "character-instance-resource://custom/avatar.png",
    };
    const html = renderToStaticMarkup(createElement(CharacterCatalog, {
      characters: [character],
      templates: [],
      activeCharacterId: "custom",
      actions: dependencies(),
      resolveAvatarUrl: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
      onRecommendations: () => undefined,
    }));

    expect(html).toContain("asset://localhost/character-instance-resource%3A%2F%2Fcustom%2Favatar.png");
    expect(html).toContain("<img");
  });

  it("returns recommendations only after character activation succeeds", async () => {
    const deps = dependencies();

    const result = await executeCharacterCatalogAction(
      { type: "select", characterId: "pico" },
      deps,
    );

    expect(deps.activateCharacter).toHaveBeenCalledWith("pico");
    expect(result).toEqual(recommendations());
  });

  it("does not expose recommendations when character activation fails", async () => {
    const deps = dependencies({
      activateCharacter: vi.fn(async () => {
        throw new Error("activation failed");
      }),
    });

    await expect(
      executeCharacterCatalogAction({ type: "select", characterId: "seren" }, deps),
    ).rejects.toThrow("activation failed");
  });

  it.each([
    ["import", "importCharacter"],
    ["edit", "editCharacter"],
    ["duplicate", "duplicateCharacter"],
    ["restore-default", "restoreCharacterDefaults"],
    ["resolve-conflict", "resolveTemplateConflict"],
  ] as const)("routes the %s action through its catalog dependency", async (type, dependency) => {
    const deps = dependencies();
    const action = type === "import"
      ? { type } as const
      : { type, characterId: "kokoro" } as const;

    const result = await executeCharacterCatalogAction(action, deps);

    if (type === "import") {
      expect(deps[dependency]).toHaveBeenCalledWith();
    } else {
      expect(deps[dependency]).toHaveBeenCalledWith("kokoro");
    }
    expect(result).toBeNull();
  });
});
