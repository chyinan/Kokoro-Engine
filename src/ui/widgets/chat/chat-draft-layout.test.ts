// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    CHAT_DRAFT_KEY_PREFIX,
    CHAT_DRAFT_IMAGES_KEY_PREFIX,
    clearCharacterDraft,
    clearCharacterDraftImages,
    combineDraftWithTranscription,
    getCharacterDraftImagesStorageKey,
    getCharacterDraftStorageKey,
    loadSavedCharacterDraft,
    loadSavedCharacterDraftImages,
    saveCharacterDraft,
    saveCharacterDraftImages,
} from "./chat-draft-layout";

describe("chat-draft-layout", () => {
    describe("combineDraftWithTranscription", () => {
        it("returns transcription when base draft is empty or whitespace", () => {
            expect(combineDraftWithTranscription("", "Hello")).toBe("Hello");
            expect(combineDraftWithTranscription("   ", "Hello")).toBe("Hello");
        });

        it("returns base draft when transcription is empty or whitespace", () => {
            expect(combineDraftWithTranscription("Draft", "")).toBe("Draft");
            expect(combineDraftWithTranscription("Draft", "   ")).toBe("Draft");
        });

        it("combines Latin words with space", () => {
            expect(combineDraftWithTranscription("Hello", "world")).toBe("Hello world");
            expect(combineDraftWithTranscription("Hello ", "world")).toBe("Hello world");
        });

        it("combines CJK characters directly without space unless user typed space", () => {
            expect(combineDraftWithTranscription("今天下午", "开会")).toBe("今天下午开会");
            expect(combineDraftWithTranscription("今天下午 ", "开会")).toBe("今天下午 开会");
        });

        it("handles CJK and Western punctuation naturally", () => {
            expect(combineDraftWithTranscription("你好，", "世界")).toBe("你好，世界");
            expect(combineDraftWithTranscription("Hello,", "world")).toBe("Hello, world");
            expect(combineDraftWithTranscription("任务列表：\n", "第一项")).toBe("任务列表：\n第一项");
        });

        it("handles alphanumeric with CJK transition gracefully", () => {
            expect(combineDraftWithTranscription("版本 2.0", "已发布")).toBe("版本 2.0 已发布");
        });
    });

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

    describe("image draft storage helpers", () => {
        it("returns key with prefix and character id", () => {
            expect(getCharacterDraftImagesStorageKey("kiana")).toBe(`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`);
            expect(getCharacterDraftImagesStorageKey("  bronya  ")).toBe(`${CHAT_DRAFT_IMAGES_KEY_PREFIX}bronya`);
            expect(getCharacterDraftImagesStorageKey("")).toBe(`${CHAT_DRAFT_IMAGES_KEY_PREFIX}default`);
        });

        it("saves and loads image array from storage", () => {
            const store: Record<string, string> = {};
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: (k: string, v: string) => { store[k] = v; },
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            saveCharacterDraftImages("kiana", ["http://test/1.png", "http://test/2.png"], mockStorage);
            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual([
                "http://test/1.png",
                "http://test/2.png",
            ]);
        });

        it("filters out invalid entries when loading image draft", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`]: JSON.stringify(["http://test/1.png", "", 123, null, "http://test/2.png"]),
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: () => {},
                removeItem: () => {},
            } as unknown as Storage;

            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual([
                "http://test/1.png",
                "http://test/2.png",
            ]);
        });

        it("removes storage key when saving empty image array", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`]: '["http://test/1.png"]',
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: (k: string, v: string) => { store[k] = v; },
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            saveCharacterDraftImages("kiana", [], mockStorage);
            expect(store[`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`]).toBeUndefined();
        });

        it("clears character image draft immediately", () => {
            const store: Record<string, string> = {
                [`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`]: '["http://test/1.png"]',
            };
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: () => {},
                removeItem: (k: string) => { delete store[k]; },
            } as unknown as Storage;

            clearCharacterDraftImages("kiana", mockStorage);
            expect(store[`${CHAT_DRAFT_IMAGES_KEY_PREFIX}kiana`]).toBeUndefined();
        });

        it("handles storage exceptions gracefully", () => {
            const throwingStorage = {
                setItem: () => {
                    throw new Error("Quota exceeded");
                },
                getItem: () => {
                    throw new Error("Storage disabled");
                },
                removeItem: () => {
                    throw new Error("Storage disabled");
                },
            } as unknown as Storage;

            expect(() => saveCharacterDraftImages("kiana", ["http://test.png"], throwingStorage)).not.toThrow();
            expect(loadSavedCharacterDraftImages("kiana", throwingStorage)).toEqual([]);
            expect(() => clearCharacterDraftImages("kiana", throwingStorage)).not.toThrow();
        });
    });
});
