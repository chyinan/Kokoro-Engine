// pattern: Imperative Shell

import {
    parseCharacterCardJsonText,
    parseCharacterCardPngBuffer,
    type ParsedCharacterCard,
} from "./character-card-core";

export type { ParsedCharacterCard } from "./character-card-core";

export function parseCharacterCardJSON(json: string): ParsedCharacterCard {
    return parseCharacterCardJsonText(json);
}

export async function parseCharacterCardPNG(file: File): Promise<ParsedCharacterCard> {
    return parseCharacterCardPngBuffer(await file.arrayBuffer());
}

export async function parseCharacterCard(file: File): Promise<ParsedCharacterCard> {
    const extension = file.name.toLowerCase().split(".").pop();
    if (extension === "json") return parseCharacterCardJSON(await file.text());
    if (extension === "png") return parseCharacterCardPNG(file);
    throw new Error(`Unsupported file format: .${extension}. Expected .json or .png`);
}
