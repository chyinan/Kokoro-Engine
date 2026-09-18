// pattern: Functional Core

import type {
    BackupCharacterSummary,
    CharacterMerge,
} from "../../../lib/kokoro-bridge";

/** Selector value meaning "import this character as a new instance". */
export const NEW_CHARACTER_TARGET = "";
/** Selector value meaning "do not restore this character at all". */
export const IGNORE_CHARACTER_TARGET = "__ignore__";

/** Selected plan per backup character id. */
export type CharacterTargets = Readonly<Record<string, string>>;

export type LocalCharacterOption = Readonly<{ id: string; name: string }>;

export type CharacterPlan = Readonly<{
    merges: CharacterMerge[];
    ignored: string[];
}>;

function normalizedName(name: string): string {
    return name.trim().toLocaleLowerCase();
}

/**
 * Preselects a target for every character in the backup.
 *
 * Instance ids are generated per machine, so a backup never matches a local id.
 * A single same-named local character is offered as a merge — that is the common
 * migration case — while anything ambiguous (no match, or several characters
 * sharing the name) defaults to importing a new instance. The user can change
 * every entry before confirming; nothing is merged or dropped implicitly.
 */
export function defaultCharacterTargets(
    backupCharacters: ReadonlyArray<BackupCharacterSummary>,
    localCharacters: ReadonlyArray<LocalCharacterOption>,
): CharacterTargets {
    const targets: Record<string, string> = {};
    for (const backup of backupCharacters) {
        const expected = normalizedName(backup.name);
        const sameName = localCharacters.filter(
            (local) => normalizedName(local.name) === expected,
        );
        targets[backup.id] =
            sameName.length === 1 ? sameName[0].id : NEW_CHARACTER_TARGET;
    }
    return targets;
}

/**
 * Splits the selector state into the IPC payload.
 *
 * Both sentinels are recognised here so a merge list can never contain one of
 * them by accident.
 */
export function buildCharacterPlan(targets: CharacterTargets): CharacterPlan {
    const merges: CharacterMerge[] = [];
    const ignored: string[] = [];
    for (const [importedId, targetId] of Object.entries(targets)) {
        if (targetId === IGNORE_CHARACTER_TARGET) {
            ignored.push(importedId);
        } else if (targetId !== NEW_CHARACTER_TARGET && targetId.length > 0) {
            merges.push({ imported_id: importedId, target_id: targetId });
        }
    }
    return { merges, ignored };
}
