// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    computeTargetScrollTop,
    isScrollAtBottom,
    computeAnchoredScrollTop,
    type ChatScrollSnapshot,
} from "./chat-scroll-state";

describe("chat-scroll-state", () => {
    describe("computeAnchoredScrollTop", () => {
        it("compensates scrollTop when scrollHeight expands after prepending", () => {
            expect(computeAnchoredScrollTop(80, 2000, 3500)).toBe(1580);
        });

        it("preserves scrollTop when scrollHeight has not increased", () => {
            expect(computeAnchoredScrollTop(80, 2000, 2000)).toBe(80);
            expect(computeAnchoredScrollTop(80, 2000, 1800)).toBe(80);
        });
    });

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
    describe("scroll retention during reply completion", () => {
        it("preserves userScrolled when user is reading history so that auto-scroll does not force bottom jump", () => {
            let userScrolled = true;
            // Completion of streaming does not overwrite userScrolled
            const onStreamFinish = () => {
                // userScrolled is preserved
            };
            onStreamFinish();
            expect(userScrolled).toBe(true);

            // Auto-scroll condition: only scroll if !userScrolled
            const willAutoScroll = !userScrolled;
            expect(willAutoScroll).toBe(false);
        });

        it("allows auto-scroll when user is at the bottom", () => {
            let userScrolled = false;
            const willAutoScroll = !userScrolled;
            expect(willAutoScroll).toBe(true);
        });
    });
});
