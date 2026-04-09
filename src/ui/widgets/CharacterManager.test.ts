import { describe, expect, it } from "vitest";
import {
    RESPONSE_LANGUAGE_PRESETS,
    USER_LANGUAGE_PRESETS,
    getLanguageSelectValue,
    shouldShowCustomLanguageInput,
} from "./CharacterManager";

describe("character language presets", () => {
    it("includes Russian in response and user language presets", () => {
        expect(RESPONSE_LANGUAGE_PRESETS).toContain("Русский");
        expect(USER_LANGUAGE_PRESETS).toContain("Русский");
    });

    it("keeps auto mode and custom mode distinct", () => {
        expect(getLanguageSelectValue("", RESPONSE_LANGUAGE_PRESETS)).toBe("auto");
        expect(getLanguageSelectValue("__custom__", RESPONSE_LANGUAGE_PRESETS)).toBe("__custom__");
        expect(shouldShowCustomLanguageInput("", RESPONSE_LANGUAGE_PRESETS)).toBe(false);
        expect(shouldShowCustomLanguageInput("__custom__", RESPONSE_LANGUAGE_PRESETS)).toBe(true);
    });
});
