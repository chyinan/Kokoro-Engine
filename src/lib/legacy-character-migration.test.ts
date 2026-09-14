// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";
import type { CharacterProfile } from "./db";
import type { CharacterRecord } from "./kokoro-bridge";
import {
    migrateLegacyCharactersToSqlite,
    type LegacyCharacterMigrationDependencies,
} from "./legacy-character-migration";

function createLegacyCharacter(overrides: Partial<CharacterProfile> = {}): CharacterProfile {
    return {
        id: 7,
        stableId: "legacy-character",
        name: "Legacy Character",
        persona: "A saved persona",
        userNickname: "User",
        sourceFormat: "manual",
        createdAt: 10,
        updatedAt: 20,
        ...overrides,
    };
}

function createDependencies(
    legacyCharacters: ReadonlyArray<CharacterProfile>,
): LegacyCharacterMigrationDependencies {
    return {
        getAll: vi.fn().mockResolvedValue(legacyCharacters),
        remove: vi.fn().mockResolvedValue(undefined),
        createCharacter: vi.fn().mockResolvedValue(undefined),
        createCharacterWithAvatar: vi.fn().mockResolvedValue(undefined),
        updateCharacter: vi.fn().mockResolvedValue(undefined),
    };
}

describe("migrateLegacyCharactersToSqlite", () => {
    it("copies the stable identity and removes the legacy row after persistence", async () => {
        const dependencies = createDependencies([createLegacyCharacter()]);

        const migrated = await migrateLegacyCharactersToSqlite(dependencies);

        expect(migrated).toBe(1);
        expect(dependencies.createCharacter).toHaveBeenCalledWith({
            id: "legacy-character",
            name: "Legacy Character",
            persona: "A saved persona",
            user_nickname: "User",
            source_format: "manual",
            created_at: 10,
            updated_at: 20,
        } satisfies CharacterRecord);
        expect(dependencies.remove).toHaveBeenCalledWith(7);
    });

    it("fails instead of silently succeeding when the legacy row cannot be removed", async () => {
        const dependencies = createDependencies([createLegacyCharacter()]);
        vi.mocked(dependencies.remove).mockRejectedValue(new Error("IndexedDB is unavailable"));

        await expect(migrateLegacyCharactersToSqlite(dependencies)).rejects.toThrow(
            "IndexedDB is unavailable",
        );
    });
});
