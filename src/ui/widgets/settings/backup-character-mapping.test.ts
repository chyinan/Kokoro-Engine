// pattern: Functional Core

import { describe, expect, it } from "vitest";

import {
    buildCharacterPlan,
    defaultCharacterTargets,
    IGNORE_CHARACTER_TARGET,
    NEW_CHARACTER_TARGET,
} from "./backup-character-mapping";
import type { BackupCharacterSummary } from "../../../lib/kokoro-bridge";

function backupCharacter(id: string, name: string): BackupCharacterSummary {
    return { id, name, memory_count: 3, conversation_count: 1 };
}

describe("backup character mapping", () => {
    it("suggests a merge only for an unambiguous same-name match", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-kokoro", "Kokoro")],
            [{ id: "local-kokoro", name: "Kokoro" }],
        );

        expect(targets).toEqual({ "remote-kokoro": "local-kokoro" });
    });

    it("defaults every character to a new import", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-1", "Pico")],
            [{ id: "local-1", name: "Kokoro" }],
        );

        expect(targets).toEqual({ "remote-1": NEW_CHARACTER_TARGET });
    });

    it("never guesses when no local character matches the name", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-kokoro", "Kokoro")],
            [{ id: "local-alice", name: "Alice" }],
        );

        expect(targets).toEqual({ "remote-kokoro": NEW_CHARACTER_TARGET });
    });

    it("defaults to a new character when several locals share the name", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-kokoro", "Kokoro")],
            [
                { id: "local-1", name: "Kokoro" },
                { id: "local-2", name: "kokoro" },
            ],
        );

        expect(targets).toEqual({ "remote-kokoro": NEW_CHARACTER_TARGET });
    });

    it("ignores surrounding whitespace when suggesting a match", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-kokoro", "  Kokoro ")],
            [{ id: "local-kokoro", name: "Kokoro" }],
        );

        expect(targets).toEqual({ "remote-kokoro": "local-kokoro" });
    });

    it("only sends the entries the user actually chose to merge", () => {
        const plan = buildCharacterPlan({
            "remote-kokoro": "local-kokoro",
            "remote-alice": NEW_CHARACTER_TARGET,
            "remote-bob": "local-bob",
        });

        expect(plan).toEqual({
            merges: [
                { imported_id: "remote-kokoro", target_id: "local-kokoro" },
                { imported_id: "remote-bob", target_id: "local-bob" },
            ],
            ignored: [],
        });
    });

    it("reports ignored characters instead of treating them as merges", () => {
        const plan = buildCharacterPlan({
            "remote-kokoro": "local-kokoro",
            "remote-archive": IGNORE_CHARACTER_TARGET,
            "remote-pico": NEW_CHARACTER_TARGET,
        });

        expect(plan.merges).toEqual([
            { imported_id: "remote-kokoro", target_id: "local-kokoro" },
        ]);
        expect(plan.ignored).toEqual(["remote-archive"]);
    });

    it("never guesses an ignore", () => {
        const targets = defaultCharacterTargets(
            [backupCharacter("remote-1", "Kokoro")],
            [{ id: "local-1", name: "Kokoro" }],
        );

        expect(Object.values(targets)).not.toContain(IGNORE_CHARACTER_TARGET);
    });
});
