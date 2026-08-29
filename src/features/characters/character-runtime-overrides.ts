// pattern: Functional Core

import type { CharacterRecord } from "@/lib/kokoro-bridge";

export type CharacterTtsRuntimeOverride = {
  readonly enabled: boolean;
  readonly providerId: string | null;
  readonly providerType: string | null;
  readonly voice: string | null;
  readonly speed: number;
  readonly pitch: number;
};

export type CharacterRuntimeOverrides = {
  readonly persona?: string;
  readonly responseLanguage?: string;
  readonly proactiveEnabled?: boolean;
  readonly live2dModel?: string | null;
  readonly background?: string | null;
  readonly cueProfile?: string | null;
  readonly tts?: Readonly<CharacterTtsRuntimeOverride>;
};

export type BuildCharacterRuntimeOverrideUpdateOptions = {
  readonly character: Readonly<CharacterRecord>;
  readonly overrides: Readonly<CharacterRuntimeOverrides>;
  readonly updatedAt: number;
};

export type CharacterRuntimeProfileObject = {
  readonly response_language?: string | null;
  readonly proactive_enabled?: boolean | null;
  readonly live2d_model?: string | null;
  readonly background?: string | null;
  readonly cue_profile?: string | null;
  readonly tts?: Readonly<Record<string, unknown>> | null;
};

export function readCharacterRuntimeProfile(raw: string | undefined): CharacterRuntimeProfileObject {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
    const record = parsed as Record<string, unknown>;
    return {
      ...(typeof record.response_language === "string"
        ? { response_language: record.response_language }
        : {}),
      ...(typeof record.proactive_enabled === "boolean"
        ? { proactive_enabled: record.proactive_enabled }
        : {}),
      ...(typeof record.live2d_model === "string" ? { live2d_model: record.live2d_model } : {}),
      ...(typeof record.background === "string" ? { background: record.background } : {}),
      ...(typeof record.cue_profile === "string" ? { cue_profile: record.cue_profile } : {}),
      ...(typeof record.tts === "object" && record.tts !== null && !Array.isArray(record.tts)
        ? { tts: record.tts as Readonly<Record<string, unknown>> }
        : {}),
    };
  } catch {
    return {};
  }
}

function ownOverride<TKey extends keyof CharacterRuntimeOverrides>(
  overrides: Readonly<CharacterRuntimeOverrides>,
  key: TKey,
): boolean {
  return Object.prototype.hasOwnProperty.call(overrides, key);
}

/** Builds a credential-free SQLite character update for runtime-owned fields. */
export function buildCharacterRuntimeOverrideUpdate(
  options: Readonly<BuildCharacterRuntimeOverrideUpdateOptions>,
): CharacterRecord {
  const { character, overrides, updatedAt } = options;
  const current = readCharacterRuntimeProfile(character.runtime_profile_json);
  const runtime: CharacterRuntimeProfileObject = {
    ...current,
    ...(ownOverride(overrides, "responseLanguage")
      ? { response_language: overrides.responseLanguage }
      : {}),
    ...(ownOverride(overrides, "proactiveEnabled")
      ? { proactive_enabled: overrides.proactiveEnabled }
      : {}),
    ...(ownOverride(overrides, "live2dModel")
      ? { live2d_model: overrides.live2dModel }
      : {}),
    ...(ownOverride(overrides, "background")
      ? { background: overrides.background }
      : {}),
    ...(ownOverride(overrides, "cueProfile")
      ? { cue_profile: overrides.cueProfile }
      : {}),
    ...(overrides.tts
      ? {
          tts: {
            enabled: overrides.tts.enabled,
            provider_id: overrides.tts.providerId,
            provider_type: overrides.tts.providerType,
            voice: overrides.tts.voice,
            speed: overrides.tts.speed,
            pitch: overrides.tts.pitch,
          },
        }
      : {}),
  };
  return {
    ...character,
    persona: overrides.persona ?? character.persona,
    updated_at: updatedAt,
    runtime_profile_json: JSON.stringify(runtime),
  };
}

/** Resolves the exact catalog edit target without altering the active character. */
export function selectCharacterForEditing(
  characters: ReadonlyArray<CharacterRecord>,
  characterId: string,
): CharacterRecord {
  const selected = characters.find((character) => character.id === characterId);
  if (!selected) throw new Error(`character '${characterId}' is unavailable`);
  return selected;
}
