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

