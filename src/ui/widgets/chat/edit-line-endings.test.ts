import { describe, expect, it } from "vitest";
import { normalizeTextareaLineEndings, restoreEditedLineEndings } from "./edit-line-endings";

describe("restoreEditedLineEndings", () => {
    it.each([
        {
            name: "an insertion at the start and replacement at the end",
            source: "A\r\nB\r\nC\nD\r\nE",
            draft: "X\nA\nB\nC\nD\nZ",
            expected: "X\nA\r\nB\r\nC\nD\r\nZ",
        },
        {
            name: "two separated edits",
            source: "one\r\nalpha\r\nkeep\nsection\rbeta\r\nend",
            draft: "one\nALPHA\nkeep\nsection\nBETA\nend",
            expected: "one\r\nALPHA\r\nkeep\nsection\rBETA\r\nend",
        },
        {
            name: "an insertion and separated deletion with the same final line count",
            source: "A\r\nB\rC\nD\r\nE",
            draft: "X\nA\nC\nD\nE",
            expected: "X\nA\r\nC\nD\r\nE",
        },
        {
            name: "a deleted line at the beginning",
            source: "first\rkeep\r\nlast",
            draft: "keep\nlast",
            expected: "keep\r\nlast",
        },
        {
            name: "duplicate and empty lines",
            source: "A\r\nsame\rsame\n\nlast\r\n",
            draft: "A\nsame\n\nlast\n",
            expected: "A\r\nsame\r\r\nlast\r\n",
        },
        {
            name: "inserting an empty line after a lone carriage return",
            source: "A\rB",
            draft: "A\n\nB",
            expected: "A\r\r\nB",
        },
        {
            name: "an unchanged mix and unchanged trailing breaks",
            source: "\r\nA\nB\rC\r\n\n",
            draft: "\nA\nB\nC\n\n",
            expected: "\r\nA\nB\rC\r\n\n",
        },
        {
            name: "LF-only text and a new trailing break",
            source: "A\nB",
            draft: "Z\nB\n\n",
            expected: "Z\nB\n\n",
        },
        {
            name: "CRLF text with one character changed",
            source: "A\r\nB\r\n",
            draft: "Z\nB\n",
            expected: "Z\r\nB\r\n",
        },
        {
            name: "lone-CR text with edits and a retained trailing break",
            source: "A\rB\r",
            draft: "X\nY\n",
            expected: "X\rY\r",
        },
        {
            name: "a single-character body",
            source: "界",
            draft: "文",
            expected: "文",
        },
    ])("preserves the intended separators for $name", ({ source, draft, expected }) => {
        const saved = restoreEditedLineEndings(source, draft);
        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it.each([96, 97, 4_096])("keeps a CRLF between two %i-character pastes", length => {
        const source = "A\r\nB";
        const expected = "x".repeat(length) + source + "y".repeat(length);
        const draft = normalizeTextareaLineEndings(expected);
        const saved = restoreEditedLineEndings(source, draft);

        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it("keeps the only break when two large pastes touch it from either side", () => {
        const source = "A\r\nB";
        const expected = `A${"x".repeat(97)}\r\n${"y".repeat(97)}B`;
        const draft = normalizeTextareaLineEndings(expected);
        const saved = restoreEditedLineEndings(source, draft);

        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it("follows retained content across large pastes with unequal line counts", () => {
        const source = "first original row\r\nkeep this row\nsection start\r\nobsolete entry\r"
            + "left side with a retained tail\r\nright side with the old ending\rfinal row";
        const expected = "p".repeat(400) + "first original row\r\ninserted row\nkeep this row\nextra entry\nsection start\r\n"
            + "q".repeat(400) + "LEFT side with a retained tail\r\nright side with a replacement ending" + "r".repeat(400) + "\rfinal row";
        const draft = normalizeTextareaLineEndings(expected);
        const saved = restoreEditedLineEndings(source, draft);

        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it("does not treat a shared punctuation mark and initial letter as evidence of a retained break", () => {
        const source = "The old address.\r\nWelcome back to the previous workspace.";
        const draft = "x".repeat(400) + "An entirely different statement.\nWhile you wait, make tea";

        expect(restoreEditedLineEndings(source, draft)).toBe(draft);
    });

    it("recovers an identifiable break after the detailed diff exhausts its step budget", () => {
        const interior = "a".repeat(2_000_000) + "\r\nretained final line";
        const source = `A${interior}Z`;
        const expected = `x${interior}y`;
        const draft = normalizeTextareaLineEndings(expected);
        const saved = restoreEditedLineEndings(source, draft);

        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it("preserves distinct internal boundaries and repeated lines in a 295,575-byte assistant message", () => {
        let line = 0;
        const paragraph = "这是一行超过模型预算的中文正文。";
        const original = `  \n长文原字-编辑回归专用前缀\n${`${paragraph}\n`.repeat(6_000)}\n编辑回归唯一尾段-Q38V\n  `
            .replace(/\n/g, () => ["\r\n", "\n", "\r", "\n"][line++ % 4]);
        expect(new TextEncoder().encode(original)).toHaveLength(295_575);

        // Change both lines around two internal breaks, with thousands of
        // identical lines between them.
        const laterBreak = [...original.matchAll(/\r\n?|\n/g)][3_002];
        const laterLineStart = laterBreak.index - paragraph.length;
        const laterLineEnd = laterBreak.index + laterBreak[0].length + paragraph.length;
        expect(laterBreak[0]).toBe("\r");
        const editedLater = original.slice(0, laterLineStart) + "p".repeat(4_096)
            + original.slice(laterLineStart, laterLineEnd) + "q".repeat(4_096) + original.slice(laterLineEnd);
        const editedStart = `${"x".repeat(97)}${paragraph}\r\n${paragraph}${"y".repeat(97)}`;
        const expected = editedLater.replace(`${paragraph}\r\n${paragraph}`, editedStart);
        const draft = normalizeTextareaLineEndings(expected);
        const saved = restoreEditedLineEndings(original, draft);

        expect(saved).toBe(expected);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });

    it("keeps unchanged interior anchors across a rewrite beyond the detailed diff limit", () => {
        const retained = "保留的第一行\r\n" + "重复的保留行\r\n重复的保留行\r".repeat(500);
        const source = "旧内容\r\n".repeat(6_000) + retained + "原结尾\r\n".repeat(6_000);
        const draft = "新内容\n".repeat(7_000) + normalizeTextareaLineEndings(retained) + "新结尾\n".repeat(5_000);
        const saved = restoreEditedLineEndings(source, draft);
        expect(saved).toContain(retained);
        expect(normalizeTextareaLineEndings(saved)).toBe(draft);
    });
});
