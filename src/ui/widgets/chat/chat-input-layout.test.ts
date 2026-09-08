// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    clampChatInputHeight,
    toggleChatInputResetHeight,
    computeResizedInputHeight,
    loadSavedChatInputHeight,
    saveChatInputHeight,
    DEFAULT_CHAT_INPUT_HEIGHT,
    MIN_CHAT_INPUT_HEIGHT,
    MAX_CHAT_INPUT_HEIGHT,
    EXPANDED_CHAT_INPUT_HEIGHT,
    CHAT_INPUT_HEIGHT_STORAGE_KEY,
} from "./chat-input-layout";

describe("chat-input-layout", () => {
    describe("clampChatInputHeight", () => {
        it("keeps height within range unchanged", () => {
            expect(clampChatInputHeight(150)).toBe(150);
            expect(clampChatInputHeight(88)).toBe(88);
        });

        it("clamps below minimum to MIN_CHAT_INPUT_HEIGHT", () => {
            expect(clampChatInputHeight(50)).toBe(MIN_CHAT_INPUT_HEIGHT);
            expect(clampChatInputHeight(0)).toBe(MIN_CHAT_INPUT_HEIGHT);
            expect(clampChatInputHeight(-20)).toBe(MIN_CHAT_INPUT_HEIGHT);
        });

        it("clamps above maximum to MAX_CHAT_INPUT_HEIGHT", () => {
            expect(clampChatInputHeight(350)).toBe(MAX_CHAT_INPUT_HEIGHT);
            expect(clampChatInputHeight(999)).toBe(MAX_CHAT_INPUT_HEIGHT);
        });

        it("handles NaN or invalid inputs with default height", () => {
            expect(clampChatInputHeight(Number.NaN)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
            expect(clampChatInputHeight(undefined as unknown as number)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
        });
    });

    describe("toggleChatInputResetHeight", () => {
        it("resets to DEFAULT when currently expanded", () => {
            expect(toggleChatInputResetHeight(150)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
            expect(toggleChatInputResetHeight(89)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
        });

        it("expands to EXPANDED when currently default or below", () => {
            expect(toggleChatInputResetHeight(DEFAULT_CHAT_INPUT_HEIGHT)).toBe(EXPANDED_CHAT_INPUT_HEIGHT);
            expect(toggleChatInputResetHeight(82)).toBe(EXPANDED_CHAT_INPUT_HEIGHT);
        });
    });

    describe("computeResizedInputHeight", () => {
        it("increases height when dragging upwards (clientY decreases)", () => {
            // startHeight 88, startY 500, currentY 450 (moved up 50px) -> 88 + 50 = 138
            expect(computeResizedInputHeight(88, 500, 450)).toBe(138);
        });

        it("decreases height when dragging downwards (clientY increases)", () => {
            // startHeight 180, startY 400, currentY 460 (moved down 60px) -> 180 - 60 = 120
            expect(computeResizedInputHeight(180, 400, 460)).toBe(120);
        });

        it("respects min and max bounds", () => {
            expect(computeResizedInputHeight(88, 500, 700)).toBe(MIN_CHAT_INPUT_HEIGHT);
            expect(computeResizedInputHeight(88, 500, 100)).toBe(MAX_CHAT_INPUT_HEIGHT);
        });
    });

    describe("storage persistence", () => {
        it("loads default height when storage is empty", () => {
            const mockStorage = {
                getItem: () => null,
                setItem: () => {},
            } as unknown as Storage;
            expect(loadSavedChatInputHeight(mockStorage)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
        });

        it("loads and clamps saved height from storage", () => {
            const mockStorage = {
                getItem: (key: string) => (key === CHAT_INPUT_HEIGHT_STORAGE_KEY ? "165" : null),
                setItem: () => {},
            } as unknown as Storage;
            expect(loadSavedChatInputHeight(mockStorage)).toBe(165);
        });

        it("clamps invalid or out-of-range storage values", () => {
            const outOfRange = {
                getItem: () => "500",
                setItem: () => {},
            } as unknown as Storage;
            expect(loadSavedChatInputHeight(outOfRange)).toBe(MAX_CHAT_INPUT_HEIGHT);

            const invalid = {
                getItem: () => "not-a-number",
                setItem: () => {},
            } as unknown as Storage;
            expect(loadSavedChatInputHeight(invalid)).toBe(DEFAULT_CHAT_INPUT_HEIGHT);
        });

        it("saves clamped height to storage", () => {
            const store: Record<string, string> = {};
            const mockStorage = {
                getItem: (k: string) => store[k] ?? null,
                setItem: (k: string, v: string) => { store[k] = v; },
            } as unknown as Storage;

            saveChatInputHeight(190, mockStorage);
            expect(store[CHAT_INPUT_HEIGHT_STORAGE_KEY]).toBe("190");

            saveChatInputHeight(400, mockStorage);
            expect(store[CHAT_INPUT_HEIGHT_STORAGE_KEY]).toBe(String(MAX_CHAT_INPUT_HEIGHT));
        });
    });
});
