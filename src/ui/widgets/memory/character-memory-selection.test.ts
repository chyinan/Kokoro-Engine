import { describe, expect, it } from "vitest";

import { getMemoryCharacterOptions } from "./character-memory-selection";

describe("memory character selection", () => {
    it("keeps same-named character instances as separate options", () => {
        expect(getMemoryCharacterOptions(
            [
                { id: "user-kokoro", name: "Kokoro" },
                { id: "preset-kokoro", name: "Kokoro" },
                { id: "pico", name: "Pico" },
            ],
            "preset-kokoro",
            "(current)",
        )).toEqual([
            { value: "user-kokoro", label: "Kokoro" },
            { value: "preset-kokoro", label: "Kokoro (current)" },
            { value: "pico", label: "Pico" },
        ]);
    });
});
