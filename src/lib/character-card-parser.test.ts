// pattern: Functional Core
import { describe, expect, it } from "vitest";
import {
    parseCharacterCardJSON,
    parseCharacterCardPNG,
} from "./character-card-parser";

function createPngCard(card: Readonly<Record<string, unknown>>): File {
    const encodedJson = new TextEncoder().encode(JSON.stringify(card));
    let jsonBytes = "";
    for (const byte of encodedJson) {
        jsonBytes += String.fromCharCode(byte);
    }

    const keyword = new TextEncoder().encode("chara\0");
    const payload = new TextEncoder().encode(btoa(jsonBytes));
    const textData = new Uint8Array(keyword.length + payload.length);
    textData.set(keyword);
    textData.set(payload, keyword.length);

    const signature = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
    const textChunk = createPngChunk("tEXt", textData);
    const endChunk = createPngChunk("IEND", new Uint8Array());
    const png = new Uint8Array(signature.length + textChunk.length + endChunk.length);
    png.set(signature);
    png.set(textChunk, signature.length);
    png.set(endChunk, signature.length + textChunk.length);

    return new File([png], "card.png", { type: "image/png" });
}

function createPngChunk(type: string, data: Uint8Array): Uint8Array {
    const chunk = new Uint8Array(12 + data.length);
    const view = new DataView(chunk.buffer);
    view.setUint32(0, data.length);
    chunk.set(new TextEncoder().encode(type), 4);
    chunk.set(data, 8);
    return chunk;
}

describe("parseCharacterCardJSON", () => {
    it("keeps v1 description, greeting, and example dialogue separate", () => {
        const profile = parseCharacterCardJSON(JSON.stringify({
            name: "V1 Character",
            description: "V1 description",
            personality: "Curious",
            scenario: "A quiet library",
            first_mes: "V1 greeting",
            mes_example: "<START>\n{{char}}: V1 example",
        }));

        expect(profile).toMatchObject({
            name: "V1 Character",
            description: "V1 description",
            greeting: "V1 greeting",
            example_dialogue: "<START>\n{{char}}: V1 example",
            source_format: "tavern-v1",
        });
        expect(profile.persona).not.toContain("V1 greeting");
        expect(profile.persona).not.toContain("V1 example");
    });

    it("maps v2 first_mes and mes_example without flattening them into persona", () => {
        const profile = parseCharacterCardJSON(JSON.stringify({
            spec: "chara_card_v2",
            spec_version: "2.0",
            data: {
                name: "V2 Character",
                description: "V2 description",
                personality: "Practical",
                scenario: "A workshop",
                first_mes: "V2 greeting",
                mes_example: "<START>\n{{char}}: V2 example",
            },
        }));

        expect(profile).toMatchObject({
            name: "V2 Character",
            description: "V2 description",
            greeting: "V2 greeting",
            example_dialogue: "<START>\n{{char}}: V2 example",
            source_format: "tavern-v2",
        });
        expect(profile.persona).not.toContain("V2 greeting");
        expect(profile.persona).not.toContain("V2 example");
    });

    it("maps v3 first_mes and mes_example from the data envelope", () => {
        const profile = parseCharacterCardJSON(JSON.stringify({
            spec: "chara_card_v3",
            spec_version: "3.0",
            data: {
                name: "V3 Character",
                description: "V3 description",
                first_mes: "V3 greeting",
                mes_example: "<START>\n{{char}}: V3 example",
            },
        }));

        expect(profile).toMatchObject({
            name: "V3 Character",
            description: "V3 description",
            greeting: "V3 greeting",
            example_dialogue: "<START>\n{{char}}: V3 example",
            source_format: "tavern-v3",
        });
    });

    it("rejects malformed or non-object card input", () => {
        expect(() => parseCharacterCardJSON("{"))
            .toThrow();
        expect(() => parseCharacterCardJSON("null"))
            .toThrow("invalid character card");
        expect(() => parseCharacterCardJSON(JSON.stringify({ data: "invalid" })))
            .toThrow("invalid character card");
    });
});

describe("parseCharacterCardPNG", () => {
    it("returns PNG bytes for managed-resource persistence instead of a data URL", async () => {
        const file = createPngCard({
            spec: "chara_card_v2",
            data: {
                name: "PNG Character",
                description: "PNG description",
                first_mes: "PNG greeting",
                mes_example: "PNG example",
            },
        });

        const profile = await parseCharacterCardPNG(file);

        expect(profile.avatar_path).toBeNull();
        expect(Array.from(profile.avatar_bytes?.slice(0, 8) ?? [])).toEqual([
            137, 80, 78, 71, 13, 10, 26, 10,
        ]);
        expect(profile).toMatchObject({
            greeting: "PNG greeting",
            example_dialogue: "PNG example",
        });
    });
});
