// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CharacterRecord } from "@/lib/kokoro-bridge";
import SettingsPanel from "./SettingsPanel";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
window.scrollTo = vi.fn();

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

const char1: CharacterRecord = {
  id: "char-1",
  name: "Character One",
  user_nickname: "User",
  persona: "Persona of Character One",
  source_format: "manual",
  created_at: 100,
  updated_at: 100,
};

const char2: CharacterRecord = {
  id: "char-2",
  name: "Character Two",
  user_nickname: "User",
  persona: "Persona of Character Two",
  source_format: "manual",
  created_at: 200,
  updated_at: 200,
};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({})),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/kokoro-bridge", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>();
  return {
    ...actual,
    listCharacters: vi.fn(async () => [char1, char2]),
    updateCharacter: vi.fn(async (c: CharacterRecord) => c),
    createCharacter: vi.fn(async (c: CharacterRecord) => c),
    getProactiveEnabled: vi.fn(async () => true),
    setUserName: vi.fn(async () => undefined),
    setUserPersona: vi.fn(async () => undefined),
    getJailbreakPrompt: vi.fn(async () => ""),
    getAutoBackupConfig: vi.fn(async () => null),
    getVisionConfig: vi.fn(async () => ({})),
    listTtsProviders: vi.fn(async () => []),
    listTtsVoices: vi.fn(async () => []),
    getKokoroErrorMessage: (err: unknown) => (err instanceof Error ? err.message : String(err)),
    saveVisionConfig: vi.fn(async () => undefined),
  };
});

vi.mock("@/lib/character-db", () => ({
  characterDb: {
    getAll: vi.fn(async () => []),
    remove: vi.fn(async () => undefined),
  },
}));

function dummyBackgroundControls() {
  return {
    config: {
      type: "solid" as const,
      color: "#000000",
      image_path: null,
      slideshow_paths: [],
      slideshow_interval: 30,
      opacity: 1,
    },
    setConfig: vi.fn(),
    currentBg: null,
    nextBg: null,
    isCrossFading: false,
    reload: vi.fn(),
    selectImage: vi.fn(),
    addSlideImages: vi.fn(),
    removeSlideImage: vi.fn(),
    clearSlideImages: vi.fn(),
    images: [],
    importFiles: vi.fn(),
    removeImage: vi.fn(),
    clearImages: vi.fn(),
    imageCount: 0,
  };
}

describe("SettingsPanel persona lifecycle and isolation", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    localStorage.clear();
  });

  afterEach(() => {
    act(() => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it("runtime failure on character A followed by character switch to B does NOT carry over A's persona on save", async () => {
    let runtimeCallCount = 0;
    const runtimeCalls: any[] = [];
    const onCharacterRuntimeChange = vi.fn(async (overrides: any) => {
      runtimeCallCount++;
      runtimeCalls.push(overrides);
      if (runtimeCallCount === 1) {
        throw new Error("Runtime activation failed for Character A");
      }
    });

    const onActivateCharacter = vi.fn(async () => undefined);
    const onClose = vi.fn();

    // 1. Mount SettingsPanel with Character A as active
    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "persona",
          activeCharacterId: "char-1",
          characters: [char1, char2],
          backgroundControls: dummyBackgroundControls() as any,
          displayMode: "full",
          onDisplayModeChange: vi.fn(),
          customModelPath: null,
          onCustomModelChange: vi.fn(),
          renderFps: 60,
          onRenderFpsChange: vi.fn(),
          onActivateCharacter,
          onCharacterRuntimeChange,
          responseLanguage: "en",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 50));
    });

    // Edit Character A's persona in the textarea (second textarea is character persona)
    const textareas = container.querySelectorAll("textarea");
    const personaTextarea = textareas.length > 1 ? textareas[1] : textareas[0];
    expect(personaTextarea).not.toBeNull();
    await act(async () => {
      if (personaTextarea) {
        const nativeSetter = Object.getOwnPropertyDescriptor(
          window.HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        nativeSetter?.call(personaTextarea, "Persona A Mutated");
        personaTextarea.dispatchEvent(new Event("input", { bubbles: true }));
        personaTextarea.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    // First Save: Character A's persona draft is committed to SQLite, but runtime reload fails
    const saveButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-save-button"]');
    expect(saveButton).not.toBeNull();

    await act(async () => {
      saveButton?.click();
    });

    // Verify first runtime change was attempted with Mutated Persona and failed
    expect(onCharacterRuntimeChange).toHaveBeenCalledTimes(1);
    expect(runtimeCalls[0]?.persona).toBe("Persona A Mutated");

    // 2. User switches to Character B
    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "persona",
          activeCharacterId: "char-2", // Switched to Character B!
          characters: [char1, char2],
          backgroundControls: dummyBackgroundControls() as any,
          displayMode: "full",
          onDisplayModeChange: vi.fn(),
          customModelPath: null,
          onCustomModelChange: vi.fn(),
          renderFps: 60,
          onRenderFpsChange: vi.fn(),
          onActivateCharacter,
          onCharacterRuntimeChange,
          responseLanguage: "en",
        }),
      );
    });

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Edit Character B's persona in the textarea
    const textareasAfterSwitch = container.querySelectorAll("textarea");
    const charBTextarea = textareasAfterSwitch.length > 1 ? textareasAfterSwitch[1] : textareasAfterSwitch[0];
    expect(charBTextarea).not.toBeNull();
    await act(async () => {
      if (charBTextarea) {
        const nativeSetter = Object.getOwnPropertyDescriptor(
          window.HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        nativeSetter?.call(charBTextarea, "Persona B Dedicated");
        charBTextarea.dispatchEvent(new Event("input", { bubbles: true }));
        charBTextarea.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    // 3. User clicks Save again for Character B
    await act(async () => {
      saveButton?.click();
    });

    // CRITICAL: Character B must receive ONLY Character B's persona, NOT Character A's old failed persona!
    expect(onCharacterRuntimeChange).toHaveBeenCalledTimes(2);
    expect(runtimeCalls[1]?.persona).toBe("Persona B Dedicated");
    expect(runtimeCalls[1]?.persona).not.toBe("Persona A Mutated");
  });

  it("header close button calls cancel and cleans up state", async () => {
    const onClose = vi.fn();
    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "persona",
          activeCharacterId: "char-1",
          characters: [char1, char2],
          backgroundControls: dummyBackgroundControls() as any,
          displayMode: "full",
          onDisplayModeChange: vi.fn(),
          customModelPath: null,
          onCustomModelChange: vi.fn(),
          renderFps: 60,
          onRenderFpsChange: vi.fn(),
          onActivateCharacter: vi.fn(),
          onCharacterRuntimeChange: vi.fn(),
          responseLanguage: "en",
        }),
      );
    });

    const closeButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-close-button"]');
    expect(closeButton).not.toBeNull();

    await act(async () => {
      closeButton?.click();
    });

    expect(onClose).toHaveBeenCalled();
  });
});
