// pattern: Functional Core

import type { CharacterRecord } from "../../../lib/kokoro-bridge";

export type MemoryCharacterOption = {
    readonly value: string;
    readonly label: string;
};

/** Builds memory selector options without collapsing characters that share a name. */
export function getMemoryCharacterOptions(
    characters: ReadonlyArray<Pick<CharacterRecord, "id" | "name">>,
    activeCharacterId: string,
    activeLabel: string,
): Array<MemoryCharacterOption> {
    return characters.map((character) => ({
        value: character.id,
        label: `${character.name}${character.id === activeCharacterId ? ` ${activeLabel}` : ""}`,
    }));
}
