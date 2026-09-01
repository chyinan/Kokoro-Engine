// pattern: Functional Core

import { describe, expect, it } from "vitest";
import { normalizeCharacterPersona } from "./character-persona";

describe("character persona normalization", () => {
    const character = {
        name: "Kokoro",
        userNickname: "枫",
    };

    it("removes repeated generated identity, nickname, and user-name segments", () => {
        const corrupted = [
            "Your name is Kokoro.",
            "Your name is Kokoro.",
            "<character_persona>",
            "A warm companion.",
            "</character_persona>",
            "Address the user as \"枫\".",
            "The user's name is 浅川枫.",
        ].join(" ");

        const normalized = normalizeCharacterPersona(corrupted, character, "浅川枫");

        expect(normalized).toBe("A warm companion.");
        expect(normalizeCharacterPersona(normalized, character, "浅川枫")).toBe(normalized);
    });

    it("keeps ordinary user-authored persona text unchanged", () => {
        const persona = "A quiet companion who likes astronomy.";

        expect(normalizeCharacterPersona(persona, character, "浅川枫")).toBe(persona);
    });
});
