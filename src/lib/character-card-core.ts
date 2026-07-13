// pattern: Functional Core

const PNG_SIGNATURE = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);

type CardRecord = Record<string, unknown>;

export type ParsedCharacterCard = {
    readonly name: string;
    readonly description: string;
    readonly persona: string;
    readonly avatar_path: string | null;
    readonly avatar_bytes: Uint8Array | null;
    readonly greeting: string;
    readonly example_dialogue: string;
    readonly user_nickname: string;
    readonly source_format: "tavern-v1" | "tavern-v2" | "tavern-v3";
};

function isCardRecord(value: unknown): value is CardRecord {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(record: Readonly<CardRecord>, keys: ReadonlyArray<string>): string {
    for (const key of keys) {
        const value = record[key];
        if (typeof value === "string" && value.length > 0) return value;
    }
    return "";
}

function getCardData(card: Readonly<CardRecord>): CardRecord {
    if (!("data" in card)) return card;
    if (!isCardRecord(card.data)) {
        throw new Error("invalid character card: data must be an object");
    }
    return card.data;
}

function getSourceFormat(
    card: Readonly<CardRecord>,
    data: Readonly<CardRecord>,
): ParsedCharacterCard["source_format"] {
    const spec = readString(card, ["spec"]);
    const specVersion = readString(card, ["spec_version"]) || readString(data, ["spec_version"]);
    if (spec === "chara_card_v3" || specVersion.startsWith("3.")) return "tavern-v3";
    if (spec === "chara_card_v2" || specVersion.startsWith("2.") || data !== card) {
        return "tavern-v2";
    }
    return "tavern-v1";
}

export function normalizeCharacterCard(cardValue: unknown): ParsedCharacterCard {
    if (!isCardRecord(cardValue)) {
        throw new Error("invalid character card: expected an object");
    }
    const data = getCardData(cardValue);
    return {
        name: readString(data, ["name", "char_name"]) || "Unnamed Character",
        description: readString(data, ["description", "char_persona"]),
        persona: readString(data, ["persona", "char_persona", "description", "personality"]),
        avatar_path: null,
        avatar_bytes: null,
        greeting: readString(data, ["first_mes", "char_greeting"]),
        example_dialogue: readString(data, ["mes_example", "example_dialogue"]),
        user_nickname: "{{user}}",
        source_format: getSourceFormat(cardValue, data),
    };
}

function isPng(buffer: ArrayBuffer): boolean {
    if (buffer.byteLength < PNG_SIGNATURE.length) return false;
    const header = new Uint8Array(buffer, 0, PNG_SIGNATURE.length);
    return header.every((byte, index) => byte === PNG_SIGNATURE[index]);
}

function extractPngTextChunks(buffer: ArrayBuffer): Map<string, string> {
    const view = new DataView(buffer);
    const latin1 = new TextDecoder("latin1");
    const utf8 = new TextDecoder("utf-8");
    const result = new Map<string, string>();
    let offset = PNG_SIGNATURE.length;
    while (offset + 12 <= buffer.byteLength) {
        const chunkLength = view.getUint32(offset);
        const chunkType = latin1.decode(new Uint8Array(buffer, offset + 4, 4));
        const dataStart = offset + 8;
        const dataEnd = dataStart + chunkLength;
        if (dataEnd + 4 > buffer.byteLength) throw new Error("invalid PNG chunk length");
        const data = new Uint8Array(buffer, dataStart, chunkLength);
        if (chunkType === "tEXt") {
            const nullIndex = data.indexOf(0);
            if (nullIndex >= 0) {
                result.set(latin1.decode(data.slice(0, nullIndex)), latin1.decode(data.slice(nullIndex + 1)));
            }
        } else if (chunkType === "iTXt") {
            const nullIndex = data.indexOf(0);
            if (nullIndex >= 0 && data[nullIndex + 1] === 0) {
                let position = nullIndex + 3;
                while (position < data.length && data[position] !== 0) position += 1;
                position += 1;
                while (position < data.length && data[position] !== 0) position += 1;
                result.set(latin1.decode(data.slice(0, nullIndex)), utf8.decode(data.slice(position + 1)));
            }
        }
        if (chunkType === "IEND") break;
        offset = dataEnd + 4;
    }
    return result;
}

function decodeBase64Utf8(value: string): string {
    const binary = atob(value);
    const bytes = Uint8Array.from(binary, character => character.charCodeAt(0));
    return new TextDecoder("utf-8").decode(bytes);
}

export function parseCharacterCardJsonText(json: string): ParsedCharacterCard {
    const card: unknown = JSON.parse(json);
    return normalizeCharacterCard(card);
}

export function parseCharacterCardPngBuffer(buffer: ArrayBuffer): ParsedCharacterCard {
    if (!isPng(buffer)) throw new Error("Not a valid PNG file");
    const charaData = extractPngTextChunks(buffer).get("chara");
    if (!charaData) {
        throw new Error('No "chara" metadata found in PNG. This may not be a SillyTavern character card.');
    }
    let json = charaData;
    try {
        json = decodeBase64Utf8(charaData);
    } catch {
        // Some card writers store plain JSON in the metadata chunk.
    }
    const normalized = normalizeCharacterCard(JSON.parse(json));
    return {...normalized, avatar_bytes: new Uint8Array(buffer.slice(0))};
}
