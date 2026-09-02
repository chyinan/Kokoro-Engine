// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CharacterRecord } from "@/lib/kokoro-bridge";
import CharacterManager, { type CharacterManagerRef } from "./CharacterManager";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("react-i18next", () => ({
  Trans: ({ children, i18nKey }: any) => children ?? i18nKey ?? null,
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: {
      language: "en",
      resolvedLanguage: "en",
      changeLanguage: vi.fn(),
    },
  } as any),
}));

vi.mock("@/lib/db", () => ({
  characterDb: {
    getAll: vi.fn(async () => []),
    remove: vi.fn(async () => undefined),
  },
}));

const mockUpdateCharacter = vi.fn();
const mockCreateCharacter = vi.fn();

vi.mock("@/lib/kokoro-bridge", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>();
  return {
    ...actual,
    listCharacters: vi.fn(async () => [char1, char2, char3]),
    updateCharacter: (c: CharacterRecord) => mockUpdateCharacter(c),
    createCharacter: (c: CharacterRecord) => mockCreateCharacter(c),
    getProactiveEnabled: vi.fn(async () => true),
    setUserName: vi.fn(async () => undefined),
    setUserPersona: vi.fn(async () => undefined),
    getJailbreakPrompt: vi.fn(async () => ""),
    getKokoroErrorMessage: (err: unknown) => (err instanceof Error ? err.message : String(err)),
  };
});

const char1: CharacterRecord = {
  id: "char-1",
  name: "Char 1 Baseline",
  user_nickname: "User",
  persona: "Persona 1 Baseline",
  source_format: "manual",
  created_at: 100,
  updated_at: 100,
};

const char2: CharacterRecord = {
  id: "char-2",
  name: "Char 2 Baseline",
  user_nickname: "User",
  persona: "Persona 2 Baseline",
  source_format: "manual",
  created_at: 200,
  updated_at: 200,
};

const char3: CharacterRecord = {
  id: "char-3",
  name: "Char 3 Baseline",
  user_nickname: "User",
  persona: "Persona 3 Baseline",
  source_format: "manual",
  created_at: 300,
  updated_at: 300,
};

describe("CharacterManager lifecycle and draft management", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    localStorage.clear();
    mockUpdateCharacter.mockReset();
  });

  afterEach(() => {
    act(() => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it("retains failed draft when one write fails in multi-character edits and allows retry", async () => {
    const managerRef = createRef<CharacterManagerRef>();

    await act(async () => {
      root.render(
        createElement(CharacterManager as any, {
          ref: managerRef,
          characters: [char1, char2, char3],
          activeCharacterId: "char-1",
          characterToEditId: "char-2",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // 1. Edit char-2's name (dirty draft in char-2)
    const nameInput = container.querySelector<HTMLInputElement>(
      'input[placeholder="settings.persona.edit.name_placeholder"]',
    );
    expect(nameInput).not.toBeNull();
    await act(async () => {
      if (nameInput) {
        const nativeSetter = Object.getOwnPropertyDescriptor(
          window.HTMLInputElement.prototype,
          "value",
        )?.set;
        nativeSetter?.call(nameInput, "Char 2 Draft Edited");
        nameInput.dispatchEvent(new Event("input", { bubbles: true }));
        nameInput.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    // 2. Switch editor to char-3 (storing char-2 in characterDraftsRef)
    await act(async () => {
      root.render(
        createElement(CharacterManager as any, {
          ref: managerRef,
          characters: [char1, char2, char3],
          activeCharacterId: "char-1",
          characterToEditId: "char-3",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Edit char-3's persona (now both char-2 and char-3 have dirty drafts)
    const personaTextarea = container.querySelector<HTMLTextAreaElement>(
      'textarea[placeholder="settings.persona.edit.persona_placeholder"]',
    );
    expect(personaTextarea).not.toBeNull();
    await act(async () => {
      if (personaTextarea) {
        const nativeSetter = Object.getOwnPropertyDescriptor(
          window.HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        nativeSetter?.call(personaTextarea, "Char 3 Persona Draft");
        personaTextarea.dispatchEvent(new Event("input", { bubbles: true }));
        personaTextarea.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    // 3. Setup mock: char-2 update fails, char-3 update succeeds
    mockUpdateCharacter.mockImplementation(async (c: CharacterRecord) => {
      if (c.id === "char-2") {
        throw new Error("Disk error writing char-2");
      }
      return c;
    });

    // Call saveDraft()
    let saveResult1: any;
    await act(async () => {
      saveResult1 = await managerRef.current?.saveDraft();
    });

    // Verify char-2 failed, char-3 succeeded
    expect(saveResult1.failedDraftIds).toContain("char-2");
    expect(saveResult1.errors?.length).toBeGreaterThan(0);
    // Draft for char-2 MUST still be retained (characterDirty true)
    expect(saveResult1.characterDirty).toBe(true);

    // 4. Next attempt: char-2 update now succeeds
    mockUpdateCharacter.mockImplementation(async (c: CharacterRecord) => c);

    let saveResult2: any;
    await act(async () => {
      saveResult2 = await managerRef.current?.saveDraft();
    });

    // char-2 was retried and now succeeded!
    expect(saveResult2.failedDraftIds?.length ?? 0).toBe(0);
    expect(saveResult2.errors?.length ?? 0).toBe(0);
    // Char 2's edited name was persisted
    expect(mockUpdateCharacter).toHaveBeenCalledWith(
      expect.objectContaining({ id: "char-2", name: "Char 2 Draft Edited" }),
    );
  });

  it("external characters prop refresh does NOT overwrite currently edited fields", async () => {
    const managerRef = createRef<CharacterManagerRef>();

    await act(async () => {
      root.render(
        createElement(CharacterManager as any, {
          ref: managerRef,
          characters: [char1, char2],
          activeCharacterId: "char-1",
          characterToEditId: "char-1",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // User is editing char-1's name
    const nameInput = container.querySelector<HTMLInputElement>(
      'input[placeholder="settings.persona.edit.name_placeholder"]',
    );
    expect(nameInput).not.toBeNull();
    await act(async () => {
      if (nameInput) {
        const nativeSetter = Object.getOwnPropertyDescriptor(
          window.HTMLInputElement.prototype,
          "value",
        )?.set;
        nativeSetter?.call(nameInput, "Char 1 Local In-Progress Typing");
        nameInput.dispatchEvent(new Event("input", { bubbles: true }));
        nameInput.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    expect(nameInput?.value).toBe("Char 1 Local In-Progress Typing");

    // External characters update arrives from network or external event with stale name
    const staleExternalCharacters: CharacterRecord[] = [
      { ...char1, name: "Stale Remote Name" },
      char2,
    ];

    await act(async () => {
      root.render(
        createElement(CharacterManager as any, {
          ref: managerRef,
          characters: staleExternalCharacters,
          activeCharacterId: "char-1",
          characterToEditId: "char-1",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // CRITICAL: The input field must retain the user's local edits and NOT be overwritten!
    const updatedInput = container.querySelector<HTMLInputElement>(
      'input[placeholder="settings.persona.edit.name_placeholder"]',
    );
    expect(updatedInput?.value).toBe("Char 1 Local In-Progress Typing");
  });
});
