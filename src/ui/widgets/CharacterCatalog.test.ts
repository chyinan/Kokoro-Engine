// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  CharacterCatalog,
  executeCharacterCatalogAction,
  type CharacterCatalogActionDependencies,
} from "./CharacterCatalog";
import type { CharacterCapabilityRecommendations } from "./CharacterRecommendationDialog";
import type { CharacterRecord, CharacterTemplateManifest } from "@/lib/kokoro-bridge";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

function template(version: string, description: string): CharacterTemplateManifest {
  return {
    schema_version: 1,
    engine_version: ">=0.3.1, <0.4.0",
    id: "template-character",
    version,
    name: "Template character",
    description,
    author: "Test",
    license: "MIT",
    locale: "en",
    avatar: null,
    persona: "A template persona",
    greeting: "Hello from the template",
    example_dialogue: null,
    assets: null,
    runtime: null,
    recommendations: null,
  };
}

function renderOpenCatalog(
  character: CharacterRecord | null,
  options: {
    readonly templates?: ReadonlyArray<CharacterTemplateManifest>;
    readonly actions?: CharacterCatalogActionDependencies;
    readonly characters?: ReadonlyArray<CharacterRecord>;
    readonly catalogError?: string | null;
    readonly onRetry?: () => void;
    readonly skipOpen?: boolean;
  } = {},
) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => {
    root.render(createElement(CharacterCatalog, {
      characters: options.characters ?? (character === null ? [] : [character]),
      templates: options.templates ?? [],
      activeCharacterId: character?.id ?? "",
      actions: options.actions ?? dependencies(),
      resolveAvatarUrl: (path: string) => path,
      catalogError: options.catalogError,
      onRetry: options.onRetry,
      onRecommendations: () => undefined,
    }));
  });

  if (options.catalogError || options.skipOpen) return { container, root };

  const toggle = container.querySelector<HTMLButtonElement>('button[aria-haspopup="listbox"]');
  if (toggle === null) throw new Error("character catalog toggle was not rendered");
  act(() => {
    toggle.click();
  });

  return { container, root };
}

describe("main character catalog actions", () => {
  it("shows a catalog load error with a retry action", async () => {
    const retry = vi.fn();
    const { container, root } = renderOpenCatalog(null, {
      catalogError: "catalog unavailable",
      onRetry: retry,
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("catalog unavailable");
    const retryButton = container.querySelector<HTMLButtonElement>('button[type="button"]');
    await act(async () => {
      retryButton?.click();
    });
    expect(retry).toHaveBeenCalledTimes(1);
    root.unmount();
  });

  it("renders structured action failures as readable messages", async () => {
    const { container, root } = renderOpenCatalog({
      id: "broken",
      name: "Broken character",
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    }, {
      actions: dependencies({
        activateCharacter: vi.fn(async () => {
          throw { code: "ACTIVATION_FAILED", message: "activation failed" };
        }),
      }),
    });

    const option = container.querySelector<HTMLButtonElement>('[role="option"]');
    await act(async () => {
      option?.click();
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("activation failed");
    root.unmount();
  });

  it("shows import failures even when the catalog starts empty", async () => {
    const { container, root } = renderOpenCatalog(null, {
      skipOpen: true,
      actions: dependencies({
        importCharacter: vi.fn(async () => {
          throw { code: "IMPORT_FAILED", message: "import failed" };
        }),
      }),
    });

    const importButton = container.querySelector<HTMLButtonElement>("button");
    await act(async () => {
      importButton?.click();
    });

    const alert = container.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent ?? "").toContain("import failed");
    root.unmount();
  });

  it("shows only the newest version of a template", () => {
    const { container, root } = renderOpenCatalog({
      id: "template-instance",
      name: "Template character",
      persona: "A companion",
      user_nickname: "User",
      source_format: "template",
      created_at: 1,
      updated_at: 1,
      template_id: "template-character",
      template_version: "1.2.0",
      avatar_path: null,
    }, {
      templates: [
        template("1.2.0", "Old template version"),
        template("1.10.0", "Newest template version"),
      ],
    });

    expect(container.querySelectorAll("article")).toHaveLength(1);
    expect(container.textContent).toContain("Newest template version");
    expect(container.textContent).not.toContain("Old template version");
    root.unmount();
  });

  it("does not render template-only entries before they are provisioned as characters", () => {
    const { container, root } = renderOpenCatalog(null, {
      templates: [template("1.0.0", "Template-only character")],
      skipOpen: true,
    });

    expect(container.querySelectorAll("article")).toHaveLength(0);
    root.unmount();
  });

  it("keeps template actions for every instance of the same template", () => {
    const instance = (id: string): CharacterRecord => ({
      id,
      name: id,
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "template",
      created_at: 1,
      updated_at: 1,
      template_id: "template-character",
      template_version: "1.0.0",
      avatar_path: null,
    });
    const rendered = renderOpenCatalog(null, {
      characters: [instance("one"), instance("two")],
      templates: [template("1.0.0", "Current template")],
    });
    expect(rendered.container.querySelectorAll('button[aria-label="characterCatalog.restoreDefault"]')).toHaveLength(2);
    rendered.root.unmount();
  });

  it("does not offer conflict resolution when the instance is already current", () => {
    const { container, root } = renderOpenCatalog({
      id: "template-instance",
      name: "Template instance",
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "template",
      created_at: 1,
      updated_at: 1,
      template_id: "template-character",
      template_version: "1.0.0",
      avatar_path: null,
    }, {
      templates: [template("1.0.0", "Current template")],
    });

    expect(container.querySelector('button[aria-label="characterCatalog.resolveConflict"]')).toBeNull();
    root.unmount();
  });

  it("opens a full character preview without activating it", async () => {
    const character: CharacterRecord = {
      id: "preview-character",
      name: "Preview character",
      persona: "A detailed persona for preview",
      description: "A detailed description for preview",
      greeting: "A preview greeting",
      example_dialogue: "A preview example",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    };
    const actions = dependencies();
    const { container, root } = renderOpenCatalog(character, { actions });

    const preview = container.querySelector<HTMLButtonElement>('button[aria-label="characterCatalog.preview"]');
    expect(preview).not.toBeNull();
    await act(async () => {
      preview?.click();
    });

    const dialog = container.querySelector('[role="dialog"]');
    expect(dialog?.textContent).toContain(character.description);
    expect(dialog?.textContent).toContain(character.greeting);
    expect(document.activeElement).toBe(dialog?.querySelector("button"));
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(preview);
    expect(actions.activateCharacter).not.toHaveBeenCalled();
    root.unmount();
  });

  it("clamps long descriptions and uses the themed scrollbar", () => {
    const { container, root } = renderOpenCatalog({
      id: "long-description",
      name: "Long description",
      persona: "A companion with a long description",
      description: "Line one. Line two. Line three should not be visible in the card.",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    });

    const listbox = container.querySelector('[role="listbox"]');
    expect(listbox?.className).toContain("scrollable");

    const description = listbox?.querySelector("span.line-clamp-2");
    expect(description).not.toBeNull();
    expect(description?.className).not.toContain(" block");
    expect(description?.className).toContain("text-[var(--color-text-primary)]");
    expect(description?.className).not.toContain("text-[var(--color-text-muted)]");

    root.unmount();
  });

  it("closes the catalog dropdown when clicking outside", async () => {
    const { container, root } = renderOpenCatalog({
      id: "char-outside",
      name: "Outside Test Character",
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    });

    expect(container.querySelector('[role="listbox"]')).not.toBeNull();

    await act(async () => {
      document.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });

    expect(container.querySelector('[role="listbox"]')).toBeNull();
    root.unmount();
  });

  it("does not close the catalog dropdown when clicking inside the dropdown container", async () => {
    const { container, root } = renderOpenCatalog({
      id: "char-inside",
      name: "Inside Test Character",
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    });

    const listbox = container.querySelector('[role="listbox"]');
    expect(listbox).not.toBeNull();

    await act(async () => {
      listbox?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });

    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    root.unmount();
  });

  it("does not close the catalog dropdown on Escape key", async () => {
    const { container, root } = renderOpenCatalog({
      id: "char-esc",
      name: "Escape Test Character",
      persona: "A companion",
      description: "A companion",
      user_nickname: "User",
      source_format: "manual",
      created_at: 1,
      updated_at: 1,
      avatar_path: null,
    });

    expect(container.querySelector('[role="listbox"]')).not.toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });

    expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    root.unmount();
  });

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
