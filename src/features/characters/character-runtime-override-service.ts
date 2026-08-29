// pattern: Imperative Shell

import type { CharacterRecord } from "@/lib/kokoro-bridge";

import {
  buildCharacterRuntimeOverrideUpdate,
  type CharacterRuntimeOverrides,
} from "./character-runtime-overrides";

export type CharacterRuntimeOverrideServiceDependencies = {
  readonly getCharacter: (characterId: string) => Promise<CharacterRecord>;
  readonly updateCharacter: (character: Readonly<CharacterRecord>) => Promise<void>;
  readonly activateCharacter: (characterId: string) => Promise<unknown>;
  readonly now: () => number;
};

export type CharacterRuntimeOverrideService = {
  readonly update: (
    characterId: string,
    overrides: Readonly<CharacterRuntimeOverrides>,
  ) => Promise<CharacterRecord>;
};

/** Persists instance overrides before re-entering the sole activation owner. */
export function createCharacterRuntimeOverrideService(
  dependencies: Readonly<CharacterRuntimeOverrideServiceDependencies>,
): CharacterRuntimeOverrideService {
  return {
    async update(
      characterId: string,
      overrides: Readonly<CharacterRuntimeOverrides>,
    ): Promise<CharacterRecord> {
      const character = await dependencies.getCharacter(characterId);
      const updated = buildCharacterRuntimeOverrideUpdate({
        character,
        overrides,
        updatedAt: dependencies.now(),
      });
      await dependencies.updateCharacter(updated);
      await dependencies.activateCharacter(characterId);
      return updated;
    },
  };
}
