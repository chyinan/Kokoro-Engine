// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CharacterRecord, VisionConfig, SttConfig, TtsSystemConfig, BotConfig } from "@/lib/kokoro-bridge";
import { saveVisionConfig, saveSttConfig, getSttConfig, getTtsConfig, getBotConfig } from "@/lib/kokoro-bridge";
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
    listVisionScreens: vi.fn(async () => []),
    getSttConfig: vi.fn(async () => null),
    saveSttConfig: vi.fn(async () => undefined),
    getTtsConfig: vi.fn(async () => null),
    getBotConfig: vi.fn(async () => null),
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

    Object.defineProperty(globalThis.navigator, "mediaDevices", {
      value: {
        getUserMedia: vi.fn(async () => ({
          getTracks: () => [{ stop: vi.fn() }],
          getVideoTracks: () => [{ stop: vi.fn() }],
        })),
        enumerateDevices: vi.fn(async () => []),
      },
      configurable: true,
      writable: true,
    });
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

  it("retries applying pending persona on subsequent save for the same character after runtime failure", async () => {
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

    // Edit Character A's persona in the textarea
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

    expect(onCharacterRuntimeChange).toHaveBeenCalledTimes(1);
    expect(runtimeCalls[0]?.persona).toBe("Persona A Mutated");

    // Second Save on the same character without modifying draft again:
    // personaDirty is false, but pending runtime persona for char-1 must be retried
    await act(async () => {
      saveButton?.click();
    });

    expect(onCharacterRuntimeChange).toHaveBeenCalledTimes(2);
    expect(runtimeCalls[1]?.persona).toBe("Persona A Mutated");

    // Third Save: pending persona was cleared after successful runtime activation,
    // so no redundant onCharacterRuntimeChange call is made
    await act(async () => {
      saveButton?.click();
    });

    expect(onCharacterRuntimeChange).toHaveBeenCalledTimes(2);
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

  const baseVisionConfig: VisionConfig = {
    vlm_enabled: true,
    auto_vision_enabled: false,
    vision_context_history_mode: "latest",
    capture_interval_secs: 5,
    change_threshold: 0.1,
    display_id: null,
    vlm_region: null,
    proactive_vision_enabled: false,
    camera_enabled: false,
    camera_device_id: null,
    vlm_provider: "llm",
    vlm_base_url: "http://baseline.local:11434",
    vlm_model: "llava",
    vlm_api_key: "",
  };

  it("vision draft edits + toggle followed by Cancel does not write to backend or localStorage and restores baseline", async () => {
    const onClose = vi.fn();
    const onVisionConfigChange = vi.fn();
    const setItemSpy = vi.spyOn(Storage.prototype, "setItem");

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "vision",
          visionConfig: baseVisionConfig,
          onVisionConfigChange,
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Toggle auto_vision in VisionTab
    const autoToggle = container.querySelector<HTMLButtonElement>('[aria-label="auto-vision-enable-toggle"]');
    expect(autoToggle).not.toBeNull();

    await act(async () => {
      autoToggle?.click();
    });

    // Before save/cancel, nothing must be written to backend or localStorage
    expect(saveVisionConfig).not.toHaveBeenCalled();
    expect(setItemSpy).not.toHaveBeenCalledWith("kokoro_vision_config", expect.anything());

    // Click global Cancel
    const cancelButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-cancel-button"]');
    expect(cancelButton).not.toBeNull();

    await act(async () => {
      cancelButton?.click();
    });

    // Still no backend or localStorage write
    expect(saveVisionConfig).not.toHaveBeenCalled();
    expect(setItemSpy).not.toHaveBeenCalledWith("kokoro_vision_config", expect.anything());
    // Baseline is restored to App state
    expect(onVisionConfigChange).toHaveBeenCalledWith(baseVisionConfig);
    expect(onClose).toHaveBeenCalled();

    setItemSpy.mockRestore();
  });

  it("vision draft edits + toggle followed by Save commits to backend, localStorage and App state together", async () => {
    const onClose = vi.fn();
    const onVisionConfigChange = vi.fn();
    const setItemSpy = vi.spyOn(Storage.prototype, "setItem");

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "vision",
          visionConfig: baseVisionConfig,
          onVisionConfigChange,
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Toggle auto_vision in VisionTab
    const autoToggle = container.querySelector<HTMLButtonElement>('[aria-label="auto-vision-enable-toggle"]');
    expect(autoToggle).not.toBeNull();

    await act(async () => {
      autoToggle?.click();
    });

    // Click global Save
    const saveButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-save-button"]');
    expect(saveButton).not.toBeNull();

    await act(async () => {
      saveButton?.click();
    });

    // Now committed together
    expect(saveVisionConfig).toHaveBeenCalledWith(
      expect.objectContaining({ auto_vision_enabled: true }),
    );
    expect(setItemSpy).toHaveBeenCalledWith(
      "kokoro_vision_config",
      expect.stringContaining('"auto_vision_enabled":true'),
    );
    expect(onVisionConfigChange).toHaveBeenCalledWith(
      expect.objectContaining({ auto_vision_enabled: true }),
    );

    setItemSpy.mockRestore();
  });

  it("STT backend save failure prevents localStorage pollution, baseline update, and event dispatch; Cancel restores baseline", async () => {
    const setItemSpy = vi.spyOn(Storage.prototype, "setItem");
    const dispatchSpy = vi.spyOn(window, "dispatchEvent");
    const onClose = vi.fn();

    const baseSttConfig: SttConfig = {
      active_provider: "browser",
      providers: [
        {
          id: "browser",
          provider_type: "browser",
          enabled: true,
        },
      ],
      auto_send: false,
      language: "en",
      continuous_listening: false,
      wake_word_enabled: false,
      wake_word: "hey kokoro",
    };

    vi.mocked(saveSttConfig).mockRejectedValueOnce(new Error("Disk full saving STT"));
    vi.mocked(getSttConfig).mockResolvedValue(baseSttConfig);

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "stt",
          sttConfig: baseSttConfig,
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    setItemSpy.mockClear();
    dispatchSpy.mockClear();

    // Toggle continuous listening or voice interrupt in SttTab
    const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>("button"));
    const continuousBtn = buttons.find((btn) => {
      const text = btn.parentElement?.textContent || "";
      return text.includes("continuous_listening") || text.includes("settings.stt.continuous_listening");
    });
    const voiceInterruptBtn = buttons.find((btn) => {
      const text = btn.parentElement?.textContent || "";
      return text.includes("voice_interrupt") || text.includes("settings.stt.voice_interrupt");
    });

    expect(continuousBtn).toBeDefined();
    expect(voiceInterruptBtn).toBeDefined();

    await act(async () => {
      continuousBtn?.click();
      voiceInterruptBtn?.click();
    });

    // Click global Save
    const saveButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-save-button"]');
    expect(saveButton).not.toBeNull();

    await act(async () => {
      saveButton?.click();
    });

    // Verify backend call was attempted
    expect(saveSttConfig).toHaveBeenCalledTimes(1);

    // Save failed, so no STT localStorage keys should be written
    const writtenKeys = setItemSpy.mock.calls.map((call) => call[0]);
    expect(writtenKeys).not.toContain("kokoro_stt_continuous_listening");
    expect(writtenKeys).not.toContain("kokoro_stt_enabled");
    expect(writtenKeys).not.toContain("kokoro_stt_auto_send");
    expect(writtenKeys).not.toContain("kokoro_voice_interrupt");

    // kokoro-stt-settings-changed should NOT have been dispatched
    const dispatchedEvents = dispatchSpy.mock.calls.map((call) => (call[0] as Event).type);
    expect(dispatchedEvents).not.toContain("kokoro-stt-settings-changed");

    // Click Cancel
    const closeBtn = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-close-button"]');
    expect(closeBtn).not.toBeNull();

    await act(async () => {
      closeBtn?.click();
    });

    expect(onClose).toHaveBeenCalled();

    // Verify localStorage still does not contain dirty STT values
    expect(setItemSpy.mock.calls.map((c) => c[0])).not.toContain("kokoro_stt_continuous_listening");
    expect(setItemSpy.mock.calls.map((c) => c[0])).not.toContain("kokoro_voice_interrupt");

    setItemSpy.mockRestore();
    dispatchSpy.mockRestore();
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it("Vision backend save failure prevents localStorage write, baseline update, and event dispatch; Cancel restores baseline", async () => {
    const setItemSpy = vi.spyOn(Storage.prototype, "setItem");
    const dispatchSpy = vi.spyOn(window, "dispatchEvent");
    const onClose = vi.fn();
    const onVisionConfigChange = vi.fn();

    vi.mocked(saveVisionConfig).mockRejectedValueOnce(new Error("Disk error saving Vision"));

    const testVisionConfig: VisionConfig = {
      ...baseVisionConfig,
      auto_vision_enabled: false,
    };

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "vision",
          visionConfig: testVisionConfig,
          onVisionConfigChange,
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    setItemSpy.mockClear();
    dispatchSpy.mockClear();
    onVisionConfigChange.mockClear();

    // Toggle auto_vision
    const autoToggle = container.querySelector<HTMLButtonElement>('[aria-label="auto-vision-enable-toggle"]');
    expect(autoToggle).not.toBeNull();

    await act(async () => {
      autoToggle?.click();
    });

    // Click global Save
    const saveButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-save-button"]');
    expect(saveButton).not.toBeNull();

    await act(async () => {
      saveButton?.click();
    });

    expect(saveVisionConfig).toHaveBeenCalled();

    // Verify NO vision localStorage keys written
    const writtenKeys = setItemSpy.mock.calls.map((call) => call[0]);
    expect(writtenKeys).not.toContain("kokoro_vision_config");
    expect(writtenKeys).not.toContain("kokoro_vision_enabled");

    // Verify NO runtime change event dispatched
    const dispatchedEvents = dispatchSpy.mock.calls.map((call) => (call[0] as Event).type);
    expect(dispatchedEvents).not.toContain("kokoro-vision-settings-changed");

    // Click Cancel
    const closeBtn = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-close-button"]');
    expect(closeBtn).not.toBeNull();

    await act(async () => {
      closeBtn?.click();
    });

    expect(onClose).toHaveBeenCalled();
    // onVisionConfigChange was called with baseline
    expect(onVisionConfigChange).toHaveBeenCalledWith(baseVisionConfig);

    setItemSpy.mockRestore();
    dispatchSpy.mockRestore();
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it("stale open hydration R1 resolving after R2 does not overwrite R2 state or baseline", async () => {
    let resolveR1Stt!: (val: SttConfig) => void;
    let resolveR1Tts!: (val: TtsSystemConfig) => void;
    let resolveR1Bot!: (val: BotConfig) => void;

    const r1SttPromise = new Promise<SttConfig>((r) => { resolveR1Stt = r; });
    const r1TtsPromise = new Promise<TtsSystemConfig>((r) => { resolveR1Tts = r; });
    const r1BotPromise = new Promise<BotConfig>((r) => { resolveR1Bot = r; });

    const dummyTtsConfig: TtsSystemConfig = {
      cache: { enabled: true, max_entries: 100, ttl_secs: 3600 },
      queue: { max_concurrent: 2 },
      providers: [],
    };

    const dummyBotConfig: BotConfig = {
      revision: 1,
      selected_platform: "telegram",
      telegram: {
        enabled: false,
        bot_token: "",
        allowed_chat_ids: [],
        send_voice_reply: false,
      },
      qq: {
        enabled: false,
        app_id: "123",
        allow_c2c: false,
        allowed_user_openids: [],
        allowed_group_openids: [],
      },
      discord: {
        enabled: false,
        allowed_channel_ids: [],
        allow_direct_messages: true,
        send_voice_reply: false,
      },
      line: {
        enabled: false,
        webhook_path: "/webhook/line",
        allowed_user_ids: [],
      },
      webhook: {
        enabled: false,
        bind_host: "127.0.0.1",
        port: 8080,
        endpoint_path: "/webhook",
        send_voice_reply: false,
      },
    };

    const r1SttConfig: SttConfig = {
      active_provider: "r1-stt",
      providers: [{ id: "r1-stt", provider_type: "browser", enabled: true }],
      auto_send: false,
      language: "en",
      continuous_listening: false,
      wake_word_enabled: false,
      wake_word: "r1-word",
    };

    const r2SttConfig: SttConfig = {
      active_provider: "r2-stt",
      providers: [{ id: "r2-stt", provider_type: "browser", enabled: true }],
      auto_send: true,
      language: "zh",
      continuous_listening: true,
      wake_word_enabled: true,
      wake_word: "r2-word",
    };

    // First call returns pending R1 promises
    vi.mocked(getSttConfig).mockReturnValueOnce(r1SttPromise);
    vi.mocked(getTtsConfig).mockReturnValueOnce(r1TtsPromise);
    vi.mocked(getBotConfig).mockReturnValueOnce(r1BotPromise);

    // Subsequent call (R2) returns immediate resolved R2 promises
    vi.mocked(getSttConfig).mockResolvedValue(r2SttConfig);
    vi.mocked(getTtsConfig).mockResolvedValue(dummyTtsConfig);
    vi.mocked(getBotConfig).mockResolvedValue(dummyBotConfig);

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // 1. Open R1
    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose: vi.fn(),
          activeTab: "stt",
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

    // 2. Rapidly close and re-open (R2)
    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: false,
          onClose: vi.fn(),
          activeTab: "stt",
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

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose: vi.fn(),
          activeTab: "stt",
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Now resolve R1's late responses with stale data
    await act(async () => {
      resolveR1Stt(r1SttConfig);
      resolveR1Tts(dummyTtsConfig);
      resolveR1Bot(dummyBotConfig);
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // Save and verify that saveSttConfig receives R2's config, NOT stale R1!
    // Make a minor edit in R2 to mark dirty
    const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>("button"));
    const continuousBtn = buttons.find((btn) => {
      const text = btn.parentElement?.textContent || "";
      return text.includes("continuous_listening") || text.includes("settings.stt.continuous_listening");
    });
    expect(continuousBtn).toBeDefined();
    await act(async () => {
      continuousBtn?.click();
    });

    const saveButton = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-save-button"]');
    expect(saveButton).not.toBeNull();

    await act(async () => {
      saveButton?.click();
    });

    expect(saveSttConfig).toHaveBeenCalledWith(
      expect.objectContaining({ active_provider: "r2-stt", language: "zh" })
    );
    expect(saveSttConfig).not.toHaveBeenCalledWith(
      expect.objectContaining({ active_provider: "r1-stt" })
    );

    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it("in-flight manual refresh after panel close is discarded and does not pollute state", async () => {
    const initialTtsConfig: TtsSystemConfig = {
      cache: { enabled: true, max_entries: 100, ttl_secs: 3600 },
      queue: { max_concurrent: 2 },
      providers: [],
    };

    let resolveRef1!: (val: TtsSystemConfig) => void;
    const ref1Promise = new Promise<TtsSystemConfig>((r) => { resolveRef1 = r; });

    const staleTtsConfig: TtsSystemConfig = {
      cache: { enabled: true, max_entries: 100, ttl_secs: 3600 },
      queue: { max_concurrent: 2 },
      providers: [],
    };

    vi.mocked(getTtsConfig).mockResolvedValueOnce(initialTtsConfig);

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose: vi.fn(),
          activeTab: "tts",
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    const refreshBtn = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(b =>
      b.textContent?.includes("refresh") || b.textContent?.includes("settings.tts.manage_providers.refresh")
    );
    expect(refreshBtn).toBeDefined();

    // Trigger refresh returning delayed promise
    vi.mocked(getTtsConfig).mockReturnValueOnce(ref1Promise);
    await act(async () => {
      refreshBtn?.click();
    });

    // Close panel while refresh is in flight
    const closeBtn = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-close-button"]');
    expect(closeBtn).not.toBeNull();
    await act(async () => {
      closeBtn?.click();
    });

    // Now delayed refresh resolves
    await act(async () => {
      resolveRef1(staleTtsConfig);
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it("background image asset actions apply immediately while display preferences follow draft cancel isolation", async () => {
    const bgControls = {
      ...dummyBackgroundControls(),
      images: ["blob:http://localhost/bg1.png", "blob:http://localhost/bg2.png"],
      imageCount: 2,
      config: {
        enabled: true,
        blur: false,
        blurAmount: 8,
        interval: 30,
        rotation: "sequential" as const,
        mode: "slideshow" as const,
      },
      setConfig: vi.fn(),
      importFiles: vi.fn(async () => 1),
      removeImage: vi.fn(async () => undefined),
      clearImages: vi.fn(async () => undefined),
    };

    const onClose = vi.fn();
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        createElement(SettingsPanel, {
          isOpen: true,
          onClose,
          activeTab: "bg",
          activeCharacterId: "char-1",
          characters: [char1, char2],
          backgroundControls: bgControls as any,
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

    await act(async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 60));
    });

    // 1. Verify remove image executes immediately
    const removeBtn = container.querySelector<HTMLButtonElement>('button[aria-label="Remove image"]');
    expect(removeBtn).not.toBeNull();
    await act(async () => {
      removeBtn?.click();
    });
    expect(bgControls.removeImage).toHaveBeenCalledWith(0);

    // 2. Verify clear button has confirmation guard
    const clearBtn = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.includes("CLEAR ALL") || b.textContent?.includes("settings.background.import.clear")
    );
    expect(clearBtn).toBeDefined();

    // First click enters confirmation mode
    await act(async () => {
      clearBtn?.click();
    });
    expect(bgControls.clearImages).not.toHaveBeenCalled();

    // Second click confirms
    await act(async () => {
      clearBtn?.click();
    });
    expect(bgControls.clearImages).toHaveBeenCalled();

    // 3. Modify slideshow interval draft
    const rangeInput = container.querySelector<HTMLInputElement>('input[type="range"]');
    if (rangeInput) {
      await act(async () => {
        rangeInput.value = "60";
        rangeInput.dispatchEvent(new Event("change", { bubbles: true }));
      });
    }

    // 4. Cancel reverts scalar preferences to baseline without reverting asset operations
    const closeBtn = container.querySelector<HTMLButtonElement>('[data-onboarding-id="settings-close-button"]');
    expect(closeBtn).not.toBeNull();
    await act(async () => {
      closeBtn?.click();
    });

    expect(onClose).toHaveBeenCalled();
    expect(bgControls.setConfig).toHaveBeenCalledWith(
      expect.objectContaining({ interval: 30 })
    );

    act(() => {
      root.unmount();
    });
    container.remove();
  });
});
