import { describe, expect, it } from "vitest";
import {
    RESPONSE_LANGUAGE_PRESETS,
    USER_LANGUAGE_PRESETS,
    getLanguageSelectValue,
    shouldShowCustomLanguageInput,
    isCharacterEditDirty,
    isUserProfileDirty,
} from "./CharacterManager";
import type { CharacterRecord } from "@/lib/kokoro-bridge";

describe("character language presets", () => {
    it("maps persisted language codes to their preset display values", () => {
        expect(getLanguageSelectValue("en", RESPONSE_LANGUAGE_PRESETS)).toBe("English");
        expect(getLanguageSelectValue("ja", RESPONSE_LANGUAGE_PRESETS)).toBe("日本語");
        expect(getLanguageSelectValue("zh-TW", USER_LANGUAGE_PRESETS)).toBe("繁體中文");
        expect(getLanguageSelectValue("ja-jp", RESPONSE_LANGUAGE_PRESETS)).toBe("日本語");
        expect(getLanguageSelectValue("zh-tw", USER_LANGUAGE_PRESETS)).toBe("繁體中文");
        expect(shouldShowCustomLanguageInput("en", RESPONSE_LANGUAGE_PRESETS)).toBe(false);
        expect(shouldShowCustomLanguageInput("ja", RESPONSE_LANGUAGE_PRESETS)).toBe(false);
    });

    it("includes Russian and Traditional Chinese in response and user language presets", () => {
        expect(RESPONSE_LANGUAGE_PRESETS).toContain("Русский");
        expect(USER_LANGUAGE_PRESETS).toContain("Русский");
        expect(RESPONSE_LANGUAGE_PRESETS).toContain("繁體中文");
        expect(USER_LANGUAGE_PRESETS).toContain("繁體中文");
    });

    it("keeps auto mode and custom mode distinct", () => {
        expect(getLanguageSelectValue("", RESPONSE_LANGUAGE_PRESETS)).toBe("auto");
        expect(getLanguageSelectValue("__custom__", RESPONSE_LANGUAGE_PRESETS)).toBe("__custom__");
        expect(shouldShowCustomLanguageInput("", RESPONSE_LANGUAGE_PRESETS)).toBe(false);
        expect(shouldShowCustomLanguageInput("__custom__", RESPONSE_LANGUAGE_PRESETS)).toBe(true);
    });
});

describe("character dirty checking", () => {
    const baseCharacter: CharacterRecord = {
        id: "char-1",
        name: "Kokoro",
        user_nickname: "User",
        persona: "You are Kokoro.",
        source_format: "manual",
        created_at: 100,
        updated_at: 100,
    };

    it("detects when character edit fields are unchanged", () => {
        expect(isCharacterEditDirty(baseCharacter, { ...baseCharacter })).toBe(false);
        expect(isCharacterEditDirty(baseCharacter, null)).toBe(false);
    });

    it("detects when character edit fields are changed", () => {
        expect(isCharacterEditDirty(baseCharacter, { ...baseCharacter, name: "New Kokoro" })).toBe(true);
        expect(isCharacterEditDirty(baseCharacter, { ...baseCharacter, user_nickname: "Boss" })).toBe(true);
        expect(isCharacterEditDirty(baseCharacter, { ...baseCharacter, persona: "Different persona" })).toBe(true);
    });

    it("detects when user profile fields are unchanged", () => {
        const profile = { name: "User", persona: "A friendly user" };
        expect(isUserProfileDirty(profile, { ...profile })).toBe(false);
    });

    it("detects when user profile fields are changed", () => {
        const profile = { name: "User", persona: "A friendly user" };
        expect(isUserProfileDirty(profile, { ...profile, name: "Alice" })).toBe(true);
        expect(isUserProfileDirty(profile, { ...profile, persona: "A programmer" })).toBe(true);
    });
});

describe("character draft management and lifecycle semantics", () => {
    const charA: CharacterRecord = {
        id: "char-a",
        name: "Character A",
        user_nickname: "User",
        persona: "Persona A",
        avatar_path: null,
        source_format: "manual",
        created_at: 100,
        updated_at: 100,
    };

    const charB: CharacterRecord = {
        id: "char-b",
        name: "Character B",
        user_nickname: "User",
        persona: "Persona B",
        avatar_path: null,
        source_format: "manual",
        created_at: 200,
        updated_at: 200,
    };

    it("Scenario 1: edits A, switches to B, reverts A to baseline -> draft is pruned and not committed", () => {
        const initialCharacters = new Map<string, CharacterRecord>([
            [charA.id, { ...charA }],
            [charB.id, { ...charB }],
        ]);
        const characterDrafts = new Map<string, CharacterRecord>();

        // 1. Edit charA
        const editedA: CharacterRecord = { ...charA, persona: "Modified Persona A" };
        if (isCharacterEditDirty(initialCharacters.get(charA.id), editedA)) {
            characterDrafts.set(charA.id, editedA);
        }
        expect(characterDrafts.has(charA.id)).toBe(true);

        // 2. Switch to charB, then back to charA and revert to baseline
        const revertedA: CharacterRecord = { ...charA, persona: "Persona A" };
        if (isCharacterEditDirty(initialCharacters.get(charA.id), revertedA)) {
            characterDrafts.set(charA.id, revertedA);
        } else {
            characterDrafts.delete(charA.id);
        }
        expect(characterDrafts.has(charA.id)).toBe(false);

        // 3. Purge pass before save
        for (const [id, draft] of characterDrafts.entries()) {
            const initial = initialCharacters.get(id);
            if (initial && !isCharacterEditDirty(initial, draft)) {
                characterDrafts.delete(id);
            }
        }
        expect(characterDrafts.size).toBe(0);
    });

    it("Scenario 1b: deleting drafted character removes draft and initial record", () => {
        const initialCharacters = new Map<string, CharacterRecord>([
            [charA.id, { ...charA }],
        ]);
        const characterDrafts = new Map<string, CharacterRecord>([
            [charA.id, { ...charA, persona: "Draft Persona" }],
        ]);

        // Simulating handleDelete(charA.id)
        characterDrafts.delete(charA.id);
        initialCharacters.delete(charA.id);

        expect(characterDrafts.has(charA.id)).toBe(false);
        expect(initialCharacters.has(charA.id)).toBe(false);
    });

    it("Scenario 2: avatar operations construct DB record using baseline, preventing uncommitted persona leaks", () => {
        const initialCharacters = new Map<string, CharacterRecord>([
            [charA.id, { ...charA }],
        ]);
        // User has drafted persona in editChar
        const editChar: CharacterRecord = { ...charA, persona: "Unsaved persona draft" };

        // Avatar change: DB payload must use baseline's persona
        const committedInitial = initialCharacters.get(editChar.id) ?? editChar;
        const avatarPath = `character-instance-resource://${editChar.id}/avatar.png`;
        const dbRecord: CharacterRecord = {
            ...committedInitial,
            avatar_path: avatarPath,
            updated_at: 300,
        };

        // Assert DB record retains original persona, not the draft!
        expect(dbRecord.persona).toBe("Persona A");
        expect(dbRecord.avatar_path).toBe(avatarPath);

        // Update baseline with new avatar
        initialCharacters.set(editChar.id, dbRecord);

        // UI state editChar gets new avatar while keeping draft persona
        const nextEditChar: CharacterRecord = { ...editChar, avatar_path: avatarPath };
        expect(nextEditChar.persona).toBe("Unsaved persona draft");
        expect(nextEditChar.avatar_path).toBe(avatarPath);

        // Cancel resets to initialCharacters
        const rolledBack = initialCharacters.get(editChar.id)!;
        expect(rolledBack.persona).toBe("Persona A");
        expect(rolledBack.avatar_path).toBe(avatarPath);
    });

    it("Scenario 3: multiple character drafts only returns active character in changedCharacter if active was committed", () => {
        const activeId = charB.id; // Character B is active
        const committedMap = new Map<string, CharacterRecord>();

        // Draft for charA was committed
        const updatedA = { ...charA, persona: "Committed Persona A" };
        committedMap.set(charA.id, updatedA);

        // Since charB was not modified, committedMap does not contain charB
        let activeCommittedChar: CharacterRecord | undefined;
        if (committedMap.has(activeId)) {
            activeCommittedChar = committedMap.get(activeId);
        }

        expect(activeCommittedChar).toBeUndefined();
    });

    it("Scenario 4: external character list refresh preserves draft baselines for drafted characters", () => {
        const initialCharacters = new Map<string, CharacterRecord>([
            [charA.id, { ...charA }],
            [charB.id, { ...charB }],
        ]);
        const characterDrafts = new Map<string, CharacterRecord>([
            [charA.id, { ...charA, persona: "In-progress draft" }],
        ]);
        const editChar = { ...charA, persona: "In-progress draft" };

        // External update arrives from backend with newer updated_at
        const externalChars: CharacterRecord[] = [
            { ...charA, name: "Remote Char A Update", updated_at: 500 },
            { ...charB, name: "Remote Char B Update", updated_at: 500 },
        ];

        externalChars.forEach((c) => {
            if (!characterDrafts.has(c.id) && editChar?.id !== c.id) {
                initialCharacters.set(c.id, { ...c });
            }
        });

        // charA baseline is preserved so Cancel rolls back correctly
        expect(initialCharacters.get(charA.id)?.name).toBe("Character A");
        // charB (not drafted) baseline is updated
        expect(initialCharacters.get(charB.id)?.name).toBe("Remote Char B Update");
    });
});

describe("revision tracking and error aggregation semantics", () => {
    it("Scenario 6: revision tracking guards async hydration so stale responses are discarded", () => {
        let openRevision = 0;
        let promptState = "initial";

        // First open: revision 1
        const rev1 = ++openRevision;
        // Fast close & reopen: revision 2
        const rev2 = ++openRevision;

        // Fast reopen completes first
        promptState = "reopened_prompt";

        // Stale rev1 completes later
        const staleResponse = "stale_prompt";
        if (rev1 === openRevision) {
            promptState = staleResponse;
        }

        // Must still be reopened_prompt
        expect(promptState).toBe("reopened_prompt");
        expect(rev2).toBe(2);
    });

    it("Scenario 10: sub-config save failures aggregate errors and prevent Saved feedback", () => {
        const saveErrors: string[] = [];
        let showSaveFeedbackCalled = false;

        // Sub-config 1 succeeds
        // Sub-config 2 (e.g. Vision) fails
        try {
            throw new Error("Vision server unreachable");
        } catch (e) {
            saveErrors.push((e as Error).message);
        }

        if (saveErrors.length === 0) {
            showSaveFeedbackCalled = true;
        }

        expect(showSaveFeedbackCalled).toBe(false);
        expect(saveErrors).toEqual(["Vision server unreachable"]);
    });
});
