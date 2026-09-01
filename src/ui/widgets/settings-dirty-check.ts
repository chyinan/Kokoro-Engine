import type {
  BotConfig,
  CharacterRecord,
  ImageGenSystemConfig,
  LlmConfig,
  SttConfig,
  TtsSystemConfig,
  VisionConfig,
} from "../../lib/kokoro-bridge";
import type { BackgroundConfig } from "../hooks/useBackgroundSlideshow";

export type UserProfile = {
  name: string;
  persona: string;
};

/**
 * Checks if editable character fields (name, user_nickname, persona) have changed.
 */
export function isCharacterEditDirty(
  baseline: Readonly<CharacterRecord> | null | undefined,
  current: Readonly<CharacterRecord> | null | undefined,
): boolean {
  if (!current) return false;
  if (!baseline) return true;
  return (
    baseline.name !== current.name ||
    baseline.user_nickname !== current.user_nickname ||
    baseline.persona !== current.persona
  );
}

/**
 * Checks if user profile fields (name, persona) have changed.
 */
export function isUserProfileDirty(
  baseline: Readonly<UserProfile> | null | undefined,
  current: Readonly<UserProfile> | null | undefined,
): boolean {
  if (!current) return false;
  if (!baseline) return true;
  return (
    baseline.name !== current.name ||
    baseline.persona !== current.persona
  );
}

export type TtsParamSnapshot = {
  enabled: boolean;
  providerId: string;
  voice: string;
  speed: string;
  pitch: string;
};

/**
 * Checks if TTS execution parameters (enabled, providerId, voice, speed, pitch) have changed.
 */
export function isTtsParamsDirty(
  baseline: Readonly<TtsParamSnapshot> | null | undefined,
  current: Readonly<TtsParamSnapshot> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return (
    baseline.enabled !== current.enabled ||
    baseline.providerId !== current.providerId ||
    baseline.voice !== current.voice ||
    baseline.speed !== current.speed ||
    baseline.pitch !== current.pitch
  );
}

/**
 * Checks if TTS system configuration has changed.
 */
export function isTtsConfigDirty(
  baseline: Readonly<TtsSystemConfig> | null | undefined,
  current: Readonly<TtsSystemConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Checks if Vision configuration has changed.
 */
export function isVisionConfigDirty(
  baseline: Readonly<VisionConfig> | null | undefined,
  current: Readonly<VisionConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Checks if STT configuration or voice interrupt setting has changed.
 */
export function isSttConfigDirty(
  baselineConfig: Readonly<SttConfig> | null | undefined,
  currentConfig: Readonly<SttConfig> | null | undefined,
  baselineVoiceInterrupt: boolean,
  currentVoiceInterrupt: boolean,
): boolean {
  if (baselineVoiceInterrupt !== currentVoiceInterrupt) return true;
  if (!currentConfig && !baselineConfig) return false;
  if (!currentConfig || !baselineConfig) return true;
  return JSON.stringify(baselineConfig) !== JSON.stringify(currentConfig);
}

/**
 * Checks if Image Generation configuration has changed.
 */
export function isImageGenConfigDirty(
  baseline: Readonly<ImageGenSystemConfig> | null | undefined,
  current: Readonly<ImageGenSystemConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Checks if Bot configuration has changed.
 */
export function isBotConfigDirty(
  baseline: Readonly<BotConfig> | null | undefined,
  current: Readonly<BotConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Checks if LLM configuration has changed.
 */
export function isLlmConfigDirty(
  baseline: Readonly<LlmConfig> | null | undefined,
  current: Readonly<LlmConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Checks if Background configuration has changed.
 */
export function isBackgroundConfigDirty(
  baseline: Readonly<BackgroundConfig> | null | undefined,
  current: Readonly<BackgroundConfig> | null | undefined,
): boolean {
  if (!current && !baseline) return false;
  if (!current || !baseline) return true;
  return JSON.stringify(baseline) !== JSON.stringify(current);
}

/**
 * Determines whether any character-runtime-relevant setting has changed.
 * Only if this returns true should `onCharacterRuntimeChange` be invoked.
 */
export function isRuntimeDirty(params: {
  personaDirty: boolean;
  ttsDirty: boolean;
  modelDirty: boolean;
  responseLangDirty: boolean;
}): boolean {
  return (
    params.personaDirty ||
    params.ttsDirty ||
    params.modelDirty ||
    params.responseLangDirty
  );
}
