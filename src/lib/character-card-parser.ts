/**
 * SillyTavern Character Card Parser
 *
 * Parses v2/v3 Character Card JSON files and PNG images with embedded
 * character data in tEXt / iTXt chunks. Zero external dependencies —
 * uses native browser APIs (DataView, TextDecoder, atob).
 */
import type { CharacterProfile } from "./db";

// ── PNG Chunk Parsing ──────────────────────────────

const PNG_SIGNATURE = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);

function isPng(buffer: ArrayBuffer): boolean {
    const header = new Uint8Array(buffer, 0, 8);
    return header.every((b, i) => b === PNG_SIGNATURE[i]);
}

/**
 * Walk through PNG chunks and extract all tEXt / iTXt entries.
 * Returns a Map of keyword → decoded text value.
 */
function extractPngTextChunks(buffer: ArrayBuffer): Map<string, string> {
    const view = new DataView(buffer);
    const decoder = new TextDecoder("latin1");
    const utf8Decoder = new TextDecoder("utf-8");
    const result = new Map<string, string>();

    let offset = 8; // skip PNG signature

    while (offset < buffer.byteLength - 4) {
        const chunkLength = view.getUint32(offset);
        const chunkTypeBytes = new Uint8Array(buffer, offset + 4, 4);
        const chunkType = decoder.decode(chunkTypeBytes);

        const dataStart = offset + 8;
        const dataEnd = dataStart + chunkLength;

        if (chunkType === "tEXt") {
            // tEXt: keyword\0value  (Latin-1 encoding)
            const data = new Uint8Array(buffer, dataStart, chunkLength);
            const nullIdx = data.indexOf(0);
            if (nullIdx !== -1) {
                const keyword = decoder.decode(data.slice(0, nullIdx));
                const value = decoder.decode(data.slice(nullIdx + 1));
                result.set(keyword, value);
            }
        } else if (chunkType === "iTXt") {
            // iTXt: keyword\0 compressionFlag(1) compressionMethod(1) languageTag\0 translatedKeyword\0 text
            const data = new Uint8Array(buffer, dataStart, chunkLength);
            const nullIdx = data.indexOf(0);
            if (nullIdx !== -1) {
                const keyword = decoder.decode(data.slice(0, nullIdx));
                const compressionFlag = data[nullIdx + 1];
                // compressionMethod = data[nullIdx + 2]; // 0 = zlib if compressed,  skip for now

                // Find language tag end (null terminated)
                let pos = nullIdx + 3;
                while (pos < data.length && data[pos] !== 0) pos++;
                pos++; // skip null

                // Find translated keyword end (null terminated)
                while (pos < data.length && data[pos] !== 0) pos++;
                pos++; // skip null

                const textBytes = data.slice(pos);

                if (compressionFlag === 0) {
                    // Uncompressed UTF-8
                    result.set(keyword, utf8Decoder.decode(textBytes));
                } else {
                    // Compressed (zlib) — use DecompressionStream if available
                    try {
                        // Fallback: try raw decode
                        result.set(keyword, utf8Decoder.decode(textBytes));
                    } catch {
                        console.warn(`[CharacterCardParser] Could not decompress iTXt chunk for keyword "${keyword}"`);
                    }
                }
            }
        }

        if (chunkType === "IEND") break;

        // Move to next chunk: length(4) + type(4) + data(chunkLength) + crc(4)
        offset = dataEnd + 4;
    }

    return result;
}

// ── JSON Parsing ───────────────────────────────────

/* eslint-disable @typescript-eslint/no-explicit-any */

/**
 * Normalize a SillyTavern character card JSON (v1, v2, or v3) into
 * our internal CharacterProfile format.
 */
function mapCardToProfile(card: any): Omit<CharacterProfile, "id" | "createdAt" | "updatedAt"> {
    // v2/v3 wraps fields inside `data`; v1 uses top-level fields
    const d = card.data ?? card;

    const name: string = d.name ?? d.char_name ?? "Unnamed Character";

    // Build persona from multiple fields
    const parts: string[] = [];

    if (d.system_prompt) {
        parts.push(d.system_prompt);
    }

    const description: string = d.description ?? d.char_persona ?? "";
    if (description) {
        parts.push(description);
    }

    if (d.personality) {
        parts.push(`Personality: ${d.personality}`);
    }

    const scenario: string = d.scenario ?? d.world_scenario ?? "";
    if (scenario) {
        parts.push(`Scenario: ${scenario}`);
    }

    const greeting: string = d.first_mes ?? d.char_greeting ?? "";
    if (greeting) {
        parts.push(`First greeting: ${greeting}`);
    }

    const examples: string = d.mes_example ?? d.example_dialogue ?? "";
    if (examples) {
        parts.push(`Example dialogue:\n${examples}`);
    }

    const persona = parts.join("\n\n");

    // Determine spec version
    let sourceFormat: "tavern-v2" | "tavern-v3" = "tavern-v2";
    if (card.spec === "chara_card_v3" || d.spec_version === "3.0") {
        sourceFormat = "tavern-v3";
    }

    return {
        name,
        persona,
        userNickname: "{{user}}",
        sourceFormat,
    };
}

/* eslint-enable @typescript-eslint/no-explicit-any */

// ── Public API ─────────────────────────────────────

/**
 * Parse a raw JSON string containing a SillyTavern character card.
 */
export function parseCharacterCardJSON(jsonStr: string): Omit<CharacterProfile, "id" | "createdAt" | "updatedAt"> {
    const card = JSON.parse(jsonStr);
    return mapCardToProfile(card);
}

/**
 * Parse a PNG file and extract the embedded SillyTavern character card.
 * Looks for a `chara` keyword in tEXt or iTXt chunks, base64-decodes
 * the value, and parses it as JSON.
 */
export async function parseCharacterCardPNG(file: File): Promise<Omit<CharacterProfile, "id" | "createdAt" | "updatedAt">> {
    const buffer = await file.arrayBuffer();

    if (!isPng(buffer)) {
        throw new Error("Not a valid PNG file");
    }

    const chunks = extractPngTextChunks(buffer);
    const charaData = chunks.get("chara");

    if (!charaData) {
        throw new Error('No "chara" metadata found in PNG. This may not be a SillyTavern character card.');
    }

    // The value is base64-encoded JSON
    let jsonStr: string;
    try {
        jsonStr = atob(charaData);
    } catch {
        // Might already be plain JSON (some tools skip base64)
        jsonStr = charaData;
    }

    // Handle potential UTF-8 encoding issues from atob
    try {
        jsonStr = decodeURIComponent(
            Array.from(jsonStr)
                .map(c => "%" + c.charCodeAt(0).toString(16).padStart(2, "0"))
                .join("")
        );
    } catch {
        // If decoding fails, use as-is
    }

    const card = JSON.parse(jsonStr);
    return mapCardToProfile(card);
}

/**
 * Detect whether a File is a character card JSON or PNG, and parse accordingly.
 */
export async function parseCharacterCard(file: File): Promise<Omit<CharacterProfile, "id" | "createdAt" | "updatedAt">> {
    const ext = file.name.toLowerCase().split(".").pop();

    if (ext === "json") {
        const text = await file.text();
        return parseCharacterCardJSON(text);
    }

    if (ext === "png") {
        return parseCharacterCardPNG(file);
    }

    throw new Error(`Unsupported file format: .${ext}. Expected .json or .png`);
}
