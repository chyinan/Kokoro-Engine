// pattern: Functional Core

import { describe, expect, it } from "vitest";
import { hasPinnedConversationState } from "../../../lib/kokoro-bridge";
import type { Conversation } from "../../../lib/kokoro-bridge";

describe("chat-conversation-pin", () => {
    describe("hasPinnedConversationState", () => {
        it("returns false for empty or default empty JSON states", () => {
            expect(hasPinnedConversationState("")).toBe(false);
            expect(hasPinnedConversationState("{}")).toBe(false);
            expect(hasPinnedConversationState("   ")).toBe(false);
        });

        it("returns true when pinned property is true", () => {
            expect(hasPinnedConversationState(JSON.stringify({ pinned: true }))).toBe(true);
            expect(hasPinnedConversationState(JSON.stringify({ pinned: true, pinned_at: "2026-01-01T00:00:00Z" }))).toBe(true);
        });

        it("returns false when pinned property is false or missing", () => {
            expect(hasPinnedConversationState(JSON.stringify({ pinned: false }))).toBe(false);
            expect(hasPinnedConversationState(JSON.stringify({ topic: "science" }))).toBe(false);
        });

        it("returns false on invalid JSON", () => {
            expect(hasPinnedConversationState("not-json")).toBe(false);
            expect(hasPinnedConversationState("{invalid")).toBe(false);
        });
    });

    describe("conversation pinned-priority sorting", () => {
        const sortConversations = (list: Conversation[]): Conversation[] => {
            return [...list].sort((a, b) => {
                const aPinned = hasPinnedConversationState(a.pinned_state) ? 1 : 0;
                const bPinned = hasPinnedConversationState(b.pinned_state) ? 1 : 0;
                if (aPinned !== bPinned) {
                    return bPinned - aPinned;
                }
                return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
            });
        };

        it("places pinned conversations at the top regardless of updated_at", () => {
            const list: Conversation[] = [
                {
                    id: "conv-1",
                    character_id: "char-1",
                    title: "Older Unpinned",
                    topic: "",
                    pinned_state: "",
                    created_at: "2026-01-01T00:00:00Z",
                    updated_at: "2026-01-01T00:00:00Z",
                },
                {
                    id: "conv-2",
                    character_id: "char-1",
                    title: "Newest Unpinned",
                    topic: "",
                    pinned_state: "{}",
                    created_at: "2026-01-05T00:00:00Z",
                    updated_at: "2026-01-05T00:00:00Z",
                },
                {
                    id: "conv-3",
                    character_id: "char-1",
                    title: "Pinned Middle",
                    topic: "",
                    pinned_state: JSON.stringify({ pinned: true }),
                    created_at: "2026-01-02T00:00:00Z",
                    updated_at: "2026-01-02T00:00:00Z",
                },
            ];

            const sorted = sortConversations(list);
            expect(sorted.map(c => c.id)).toEqual(["conv-3", "conv-2", "conv-1"]);
        });

        it("sorts multiple pinned items by updated_at descending", () => {
            const list: Conversation[] = [
                {
                    id: "conv-pin-1",
                    character_id: "char-1",
                    title: "Pinned Older",
                    topic: "",
                    pinned_state: JSON.stringify({ pinned: true }),
                    created_at: "2026-01-01T00:00:00Z",
                    updated_at: "2026-01-02T00:00:00Z",
                },
                {
                    id: "conv-pin-2",
                    character_id: "char-1",
                    title: "Pinned Newer",
                    topic: "",
                    pinned_state: JSON.stringify({ pinned: true }),
                    created_at: "2026-01-01T00:00:00Z",
                    updated_at: "2026-01-04T00:00:00Z",
                },
                {
                    id: "conv-unpin",
                    character_id: "char-1",
                    title: "Unpinned Newest",
                    topic: "",
                    pinned_state: "{}",
                    created_at: "2026-01-01T00:00:00Z",
                    updated_at: "2026-01-10T00:00:00Z",
                },
            ];

            const sorted = sortConversations(list);
            expect(sorted.map(c => c.id)).toEqual(["conv-pin-2", "conv-pin-1", "conv-unpin"]);
        });
    });

    describe("pin toggle state transition", () => {
        it("toggles from unpinned to pinned with JSON payload", () => {
            const conv: Conversation = {
                id: "conv-1",
                character_id: "char-1",
                title: "Test",
                topic: "",
                pinned_state: "{}",
                created_at: "2026-01-01T00:00:00Z",
                updated_at: "2026-01-01T00:00:00Z",
            };

            const isCurrentlyPinned = hasPinnedConversationState(conv.pinned_state);
            const nextPinnedState = isCurrentlyPinned
                ? "{}"
                : JSON.stringify({ pinned: true, pinned_at: new Date().toISOString() });

            expect(isCurrentlyPinned).toBe(false);
            expect(hasPinnedConversationState(nextPinnedState)).toBe(true);
        });

        it("toggles from pinned to unpinned with empty object", () => {
            const conv: Conversation = {
                id: "conv-1",
                character_id: "char-1",
                title: "Test",
                topic: "",
                pinned_state: JSON.stringify({ pinned: true }),
                created_at: "2026-01-01T00:00:00Z",
                updated_at: "2026-01-01T00:00:00Z",
            };

            const isCurrentlyPinned = hasPinnedConversationState(conv.pinned_state);
            const nextPinnedState = isCurrentlyPinned
                ? "{}"
                : JSON.stringify({ pinned: true, pinned_at: new Date().toISOString() });

            expect(isCurrentlyPinned).toBe(true);
            expect(nextPinnedState).toBe("{}");
            expect(hasPinnedConversationState(nextPinnedState)).toBe(false);
        });
    });
});
