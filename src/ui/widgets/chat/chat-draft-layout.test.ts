// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    CHAT_DRAFT_KEY_PREFIX,
    clearCharacterDraft,
    getCharacterDraftStorageKey,
    loadSavedCharacterDraft,
    saveCharacterDraft,
} from "./chat-draft-layout";

describe("chat-draft-layout", () => {
    describe("getCharacterDraftStorageKey", () => {
        it("returns key with prefix and character id", () => {
            expect(getCharacterDraftStorageKey("kiana")).toBe(`${CHAT_DRAFT_KEY_PREFIX}kiana`);
            expect(getCharacterDraftStorageKey("  bronya  ")).toBe(`${CHAT_DRAFT_KEY_PREFIX}bronya`);
        });

        it("falls back to default if character id is empty or blank", () => {
            expect(getCharacterDraftStorageKey("")).toBe(`${CHAT_DRAFT_KEY_PREFIX}default`);
            expect(getCharacterDraftStorageKey("   ")).toBe(`${CHAT_DRAFT_KEY_PREFIX}default`);
        });

        it("encodes special characters safely", () => {
            expect(getCharacterDraftStorageKey("user/char:1")).toBe(
                `${CHAT_DRAFT_KEY_PREFIX}user%2Fchar%3A1`
            );
        });
    });

    describe("loadSavedCharacterDraft", () => {
        it("loads draft string from storage", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_KEY_PREFIX}kiana`]: "Hello Kiana!",
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: () => {},
                removeItem: () => {},
            } as unknown as Storage;

            expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Hello Kiana!");
            expect(loadSavedCharacterDraft("bronya", mockStorage)).toBe("");
        });

        it("returns empty string if storage is unavailable or throws", () => {
            const throwingStorage = {
                getItem: () => {
                    throw new Error("Quota or security error");
                },
            } as unknown as Storage;

            expect(loadSavedCharacterDraft("kiana", throwingStorage)).toBe("");
            expect(loadSavedCharacterDraft("kiana", undefined)).toBe("");
        });
    });

    describe("saveCharacterDraft", () => {
        it("saves non-empty text to storage", () => {
            const store: Record<string, string> = {};
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: (k: string, v: string) => { store[k] = v; },
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            saveCharacterDraft("kiana", "Testing draft", mockStorage);
            expect(store[`${CHAT_DRAFT_KEY_PREFIX}kiana`]).toBe("Testing draft");
        });

        it("removes the storage key if draft is empty or whitespace only", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_KEY_PREFIX}kiana`]: "Existing text",
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: (k: string, v: string) => { store[k] = v; },
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            saveCharacterDraft("kiana", "", mockStorage);
            expect(store[`${CHAT_DRAFT_KEY_PREFIX}kiana`]).toBeUndefined();

            store[`${CHAT_DRAFT_KEY_PREFIX}kiana`] = "More text";
            saveCharacterDraft("kiana", "   \n\t  ", mockStorage);
            expect(store[`${CHAT_DRAFT_KEY_PREFIX}kiana`]).toBeUndefined();
        });

        it("handles storage exceptions gracefully", () => {
            const throwingStorage = {
                setItem: () => {
                    throw new Error("Disk full");
                },
                removeItem: () => {},
            } as unknown as Storage;

            expect(() => saveCharacterDraft("kiana", "some text", throwingStorage)).not.toThrow();
        });
    });

    describe("clearCharacterDraft", () => {
        it("removes the key from storage", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_KEY_PREFIX}kiana`]: "Draft to delete",
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: () => {},
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            clearCharacterDraft("kiana", mockStorage);
            expect(store[`${CHAT_DRAFT_KEY_PREFIX}kiana`]).toBeUndefined();
        });
    });
});
