export const APP_SETTING_KEYS = {
  activeCharacterId: "kokoro_active_character_id",
  appLanguage: "kokoro_app_language",
  bgConfig: "kokoro_bg_config",
  customModelPath: "kokoro_custom_model_path",
  displayMode: "kokoro_display_mode",
  gazeTracking: "kokoro_gaze_tracking",
  persona: "kokoro_persona",
  proactiveEnabled: "kokoro_proactive_enabled",
  responseLanguage: "kokoro_response_language",
  settingsActiveTab: "kokoro_settings_active_tab",
  sttAutoSend: "kokoro_stt_auto_send",
  sttContinuousListening: "kokoro_stt_continuous_listening",
  sttEnabled: "kokoro_stt_enabled",
  sttLanguage: "kokoro_stt_language",
  ttsEnabled: "kokoro_tts_enabled",
  ttsPitch: "kokoro_tts_pitch",
  ttsProvider: "kokoro_tts_provider",
  ttsSpeed: "kokoro_tts_speed",
  ttsVoice: "kokoro_tts_voice",
  userLanguage: "kokoro_user_language",
  userName: "kokoro_user_name",
  userPersona: "kokoro_user_persona",
  visionConfig: "kokoro_vision_config",
  visionEnabled: "kokoro_vision_enabled",
  voiceInterrupt: "kokoro_voice_interrupt",
  wakeWord: "kokoro_wake_word",
  wakeWordEnabled: "kokoro_wake_word_enabled",
} as const;

export type AppSettingKey = (typeof APP_SETTING_KEYS)[keyof typeof APP_SETTING_KEYS];
export type RuntimeSettingsKind = "vision" | "stt";

const browserStorage = () => window.localStorage;

export function readStringSetting(
  key: AppSettingKey,
  fallback: string,
  storage: Storage = browserStorage(),
): string {
  return storage.getItem(key) ?? fallback;
}

export function readOptionalStringSetting(
  key: AppSettingKey,
  storage: Storage = browserStorage(),
): string | null {
  return storage.getItem(key);
}

export function writeStringSetting(
  key: AppSettingKey,
  value: string,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, value);
}

export function readBooleanSetting(
  key: AppSettingKey,
  fallback: boolean,
  storage: Storage = browserStorage(),
): boolean {
  const raw = storage.getItem(key);
  if (raw === null) return fallback;
  return raw === "true";
}

export function writeBooleanSetting(
  key: AppSettingKey,
  value: boolean,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, value ? "true" : "false");
}

export function readNumberSetting(
  key: AppSettingKey,
  fallback: number,
  storage: Storage = browserStorage(),
): number {
  const raw = storage.getItem(key);
  if (raw === null) return fallback;
  const parsed = Number.parseFloat(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

export function writeNumberSetting(
  key: AppSettingKey,
  value: number,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, String(value));
}

export function readJsonSetting<T>(
  key: AppSettingKey,
  fallback: T,
  storage: Storage = browserStorage(),
): T {
  const raw = storage.getItem(key);
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function writeJsonSetting<T>(
  key: AppSettingKey,
  value: T,
  storage: Storage = browserStorage(),
): void {
  storage.setItem(key, JSON.stringify(value));
}

export function removeSetting(
  key: AppSettingKey,
  storage: Storage = browserStorage(),
): void {
  storage.removeItem(key);
}

export function dispatchRuntimeSettingsChanged(
  kind: RuntimeSettingsKind,
  target: Pick<EventTarget, "dispatchEvent"> = window,
): void {
  const eventName =
    kind === "vision" ? "kokoro-vision-settings-changed" : "kokoro-stt-settings-changed";
  target.dispatchEvent(new Event(eventName));
}
