// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    computeTargetScrollTop,
    isScrollAtBottom,
    type ChatScrollSnapshot,
} from "./chat-scroll-state";

describe("chat-scroll-state", () => {
    describe("isScrollAtBottom", () => {
        it("returns true when exact bottom", () => {
            // scrollHeight: 1000, clientHeight: 400, scrollTop: 600
            expect(isScrollAtBottom(600, 1000, 400)).toBe(true);
        });

        it("returns true when within threshold (e.g. 30px from bottom)", () => {
            // distance to bottom: 1000 - 570 - 400 = 30 < 40
            expect(isScrollAtBottom(570, 1000, 400)).toBe(true);
        });

        it("returns false when beyond threshold (e.g. 50px from bottom)", () => {
            // distance to bottom: 1000 - 550 - 400 = 50 >= 40
            expect(isScrollAtBottom(550, 1000, 400)).toBe(false);
        });

        it("respects custom threshold", () => {
            expect(isScrollAtBottom(550, 1000, 400, 60)).toBe(true);
            expect(isScrollAtBottom(550, 1000, 400, 20)).toBe(false);
        });
    });

    describe("computeTargetScrollTop", () => {
        it("returns bottom scroll when snapshot is null", () => {
            const result = computeTargetScrollTop(null, 1500, 500);
            expect(result.scrollTop).toBe(1500);
            expect(result.userScrolled).toBe(false);
        });

        it("returns bottom scroll when snapshot was at bottom", () => {
            const snapshot: ChatScrollSnapshot = {
                scrollTop: 800,
                scrollHeight: 1200,
                clientHeight: 400,
                isAtBottom: true,
            };
            // Even if content grew while collapsed (e.g. 1200 -> 1600)
            const result = computeTargetScrollTop(snapshot, 1600, 400);
            expect(result.scrollTop).toBe(1600);
            expect(result.userScrolled).toBe(false);
        });

        it("restores exact scrollTop when user was scrolled up reading history", () => {
            const snapshot: ChatScrollSnapshot = {
                scrollTop: 350,
                scrollHeight: 1200,
                clientHeight: 400,
                isAtBottom: false,
            };
            const result = computeTargetScrollTop(snapshot, 1600, 400);
            expect(result.scrollTop).toBe(350);
            expect(result.userScrolled).toBe(true);
        });

        it("clamps scrollTop if content shrank below previous scrollTop", () => {
            const snapshot: ChatScrollSnapshot = {
                scrollTop: 900,
                scrollHeight: 1500,
                clientHeight: 400,
                isAtBottom: false,
            };
            // Max scroll is 800 - 400 = 400
            const result = computeTargetScrollTop(snapshot, 800, 400);
            expect(result.scrollTop).toBe(400);
            expect(result.userScrolled).toBe(true);
        });
    });
});
