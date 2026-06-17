import { describe, expect, it, vi } from "vitest";
import {
  APP_SETTING_KEYS,
  readBooleanSetting,
  readNumberSetting,
  readOptionalStringSetting,
  readJsonSetting,
  readStringSetting,
  removeSetting,
  writeBooleanSetting,
  writeJsonSetting,
  writeNumberSetting,
  writeStringSetting,
  dispatchRuntimeSettingsChanged,
} from "./app-settings";

function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (key: string) => data.get(key) ?? null,
    key: (index: number) => [...data.keys()][index] ?? null,
    removeItem: (key: string) => {
      data.delete(key);
    },
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
}

describe("app-settings", () => {
  it("reads boolean settings with defaults", () => {
    const storage = memoryStorage();
    expect(readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false, storage)).toBe(false);
    storage.setItem(APP_SETTING_KEYS.ttsEnabled, "true");
    expect(readBooleanSetting(APP_SETTING_KEYS.ttsEnabled, false, storage)).toBe(true);
  });

  it("writes string, number, and boolean settings", () => {
    const storage = memoryStorage();
    writeStringSetting(APP_SETTING_KEYS.ttsVoice, "alice", storage);
    writeNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1.25, storage);
    writeBooleanSetting(APP_SETTING_KEYS.visionEnabled, true, storage);

    expect(readStringSetting(APP_SETTING_KEYS.ttsVoice, "", storage)).toBe("alice");
    expect(readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1, storage)).toBe(1.25);
    expect(readBooleanSetting(APP_SETTING_KEYS.visionEnabled, false, storage)).toBe(true);
  });

  it("includes the STT language key used by runtime settings writes", () => {
    const storage = memoryStorage();
    expect(APP_SETTING_KEYS.sttLanguage).toBe("kokoro_stt_language");

    writeStringSetting(APP_SETTING_KEYS.sttLanguage, "ja", storage);

    expect(readStringSetting(APP_SETTING_KEYS.sttLanguage, "", storage)).toBe("ja");
  });

  it("reads optional strings as null when unset", () => {
    const storage = memoryStorage();
    expect(readOptionalStringSetting(APP_SETTING_KEYS.activeCharacterId, storage)).toBeNull();

    storage.setItem(APP_SETTING_KEYS.activeCharacterId, "default");
    expect(readOptionalStringSetting(APP_SETTING_KEYS.activeCharacterId, storage)).toBe("default");
  });

  it("falls back for invalid numbers and invalid json", () => {
    const storage = memoryStorage();
    storage.setItem(APP_SETTING_KEYS.ttsSpeed, "fast");
    storage.setItem(APP_SETTING_KEYS.visionConfig, "{");

    expect(readNumberSetting(APP_SETTING_KEYS.ttsSpeed, 1, storage)).toBe(1);
    expect(readJsonSetting(APP_SETTING_KEYS.visionConfig, { camera_enabled: false }, storage)).toEqual({
      camera_enabled: false,
    });
  });

  it("writes json settings and removes settings", () => {
    const storage = memoryStorage();
    writeJsonSetting(APP_SETTING_KEYS.visionConfig, { camera_enabled: true }, storage);

    expect(readJsonSetting(APP_SETTING_KEYS.visionConfig, {}, storage)).toEqual({
      camera_enabled: true,
    });

    removeSetting(APP_SETTING_KEYS.visionConfig, storage);
    expect(readJsonSetting(APP_SETTING_KEYS.visionConfig, { camera_enabled: false }, storage)).toEqual({
      camera_enabled: false,
    });
  });

  it("dispatches stable sync events", () => {
    const target = new EventTarget();
    const vision = vi.fn();
    const stt = vi.fn();
    target.addEventListener("kokoro-vision-settings-changed", vision);
    target.addEventListener("kokoro-stt-settings-changed", stt);

    dispatchRuntimeSettingsChanged("vision", target);
    dispatchRuntimeSettingsChanged("stt", target);

    expect(vision).toHaveBeenCalledTimes(1);
    expect(stt).toHaveBeenCalledTimes(1);
  });
});
