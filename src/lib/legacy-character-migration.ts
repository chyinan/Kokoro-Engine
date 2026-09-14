// pattern: Imperative Shell

import { characterDb, type CharacterProfile } from "./db";
import {
    createCharacter,
    createCharacterWithAvatar,
    updateCharacter,
    type CharacterRecord,
} from "./kokoro-bridge";

export type LegacyCharacterMigrationDependencies = {
    readonly getAll: () => Promise<ReadonlyArray<CharacterProfile>>;
    readonly remove: (id: number) => Promise<void>;
    readonly createCharacter: (record: CharacterRecord) => Promise<void>;
    readonly createCharacterWithAvatar: (record: CharacterRecord, bytes: Uint8Array) => Promise<void>;
    readonly updateCharacter: (record: Omit<CharacterRecord, "created_at">) => Promise<void>;
};

const defaultDependencies: LegacyCharacterMigrationDependencies = {
    getAll: () => characterDb.getAll(),
    remove: (id) => characterDb.remove(id),
    createCharacter,
    createCharacterWithAvatar,
    updateCharacter,
};

function toCharacterRecord(character: CharacterProfile): CharacterRecord {
    return {
        id: character.stableId,
        name: character.name,
        persona: character.persona,
        user_nickname: character.userNickname,
        source_format: character.sourceFormat ?? "manual",
        created_at: character.createdAt ?? 0,
        updated_at: character.updatedAt ?? 0,
    };
}

/**
 * Migrate legacy IndexedDB character rows before any catalog or backup reads.
 * A row is removed from IndexedDB only after a SQLite create/update succeeds.
 */
export async function migrateLegacyCharactersToSqlite(
    dependencies: LegacyCharacterMigrationDependencies = defaultDependencies,
): Promise<number> {
    let migrated = 0;
    const legacyCharacters = await dependencies.getAll();
    for (const character of legacyCharacters) {
        if (!character.stableId) continue;

        const record = toCharacterRecord(character);
        let avatarBytes: Uint8Array | null = null;
        if (character.avatarBlob) {
            try {
                avatarBytes = new Uint8Array(await character.avatarBlob.arrayBuffer());
            } catch (error) {
                console.warn("[LegacyCharacterMigration] Failed to read legacy avatar:", error);
            }
        }

        let persisted = false;
        if (avatarBytes !== null) {
            try {
                await dependencies.createCharacterWithAvatar(
                    {
                        ...record,
                        avatar_path: `character-instance-resource://${character.stableId}/avatar.png`,
                    },
                    avatarBytes,
                );
                persisted = true;
            } catch (error) {
                console.warn(
                    "[LegacyCharacterMigration] Failed to migrate legacy avatar, retrying without avatar:",
                    error,
                );
            }
        }

        if (!persisted) {
            try {
                await dependencies.createCharacter(record);
                persisted = true;
            } catch (createError) {
                try {
                    const { created_at: _createdAt, ...updateRecord } = record;
                    await dependencies.updateCharacter(updateRecord);
                    persisted = true;
                } catch (updateError) {
                    console.error("[LegacyCharacterMigration] Failed to migrate character:", {
                        characterId: character.stableId,
                        createError,
                        updateError,
                    });
                }
            }
        }

        if (persisted && character.id !== undefined) {
            try {
                await dependencies.remove(character.id);
            } catch (error) {
                console.error(
                    "[LegacyCharacterMigration] Failed to remove migrated IndexedDB character:",
                    error,
                );
                throw error;
            }
            migrated += 1;
        }
    }
    return migrated;
}
