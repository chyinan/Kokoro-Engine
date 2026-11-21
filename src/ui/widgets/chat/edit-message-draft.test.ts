import { describe, expect, it } from "vitest";
import { normalizeTextareaLineEndings } from "./edit-line-endings";
import { EditMessageDraft } from "./edit-message-draft";

function replace(draft: EditMessageDraft, start: number, end: number, text: string, inputType: string) {
    const before = { value: draft.value, start, end, inputType };
    const caret = start + text.length;
    draft.input({ value: draft.value.slice(0, start) + text + draft.value.slice(end), start: caret, end: caret, inputType }, before);
}

function expectValue(draft: EditMessageDraft, expected: string) {
    expect(draft.serialize()).toBe(expected);
    expect(draft.value).toBe(normalizeTextareaLineEndings(expected));
}

describe("EditMessageDraft", () => {
    it.each([
        { afterA: false, afterB: false, size: 97 },
        { afterA: false, afterB: true, size: 97 },
        { afterA: true, afterB: false, size: 97 },
        { afterA: true, afterB: true, size: 97 },
        { afterA: true, afterB: true, size: 4_096 },
    ])("keeps the break with $size-character pastes (after A: $afterA, after B: $afterB)", ({ afterA, afterB, size }) => {
        const draft = new EditMessageDraft("A\r\nB");
        const first = "x".repeat(size);
        const second = "y".repeat(size);
        const firstCaret = Number(afterA);
        replace(draft, firstCaret, firstCaret, first, "insertFromPaste");
        const secondCaret = draft.value.length - Number(!afterB);
        replace(draft, secondCaret, secondCaret, second, "insertFromPaste");

        const firstLine = afterA ? `A${first}` : `${first}A`;
        const secondLine = afterB ? `B${second}` : `${second}B`;
        expectValue(draft, `${firstLine}\r\n${secondLine}`);
    });

    it.each([
        { start: 2, end: 7, text: "", inputType: "deleteByCut", expected: "A\r\nsame\nZ" },
        { start: 7, end: 12, text: "", inputType: "deleteContentBackward", expected: "A\r\nsame\rZ" },
        { start: 2, end: 2, text: "same\n", inputType: "insertFromPaste", expected: "A\r\nsame\nsame\rsame\nZ" },
        { start: 2, end: 7, text: "same\n", inputType: "insertFromPaste", expected: "A\r\nsame\nsame\nZ" },
    ])("uses the actual $inputType range $start..$end among duplicate lines", ({ start, end, text, inputType, expected }) => {
        const draft = new EditMessageDraft("A\r\nsame\rsame\nZ");
        replace(draft, start, end, text, inputType);
        expectValue(draft, expected);
    });

    it("keeps a lone CR and reverses the serialization adjustment when a blank line is split again", () => {
        const draft = new EditMessageDraft("A\rB\nC");
        replace(draft, 2, 3, "", "deleteContentBackward");
        expectValue(draft, "A\r\r\nC");
        replace(draft, 2, 2, "B", "insertText");
        expectValue(draft, "A\rB\nC");
    });

    it("uses UTF-16 caret offsets for a collapsed Backspace that removes a grapheme", () => {
        const draft = new EditMessageDraft("A\r\nB🙂\rC\r\n");
        const inputType = "deleteContentBackward";
        draft.input({ value: "A\nB\nC\n", start: 3, end: 3, inputType }, { value: draft.value, start: 5, end: 5, inputType });
        expectValue(draft, "A\r\nB\rC\r\n");
    });

    it("retires the correct break on collapsed forward Delete", () => {
        const draft = new EditMessageDraft("A\r\nB\rC");
        const inputType = "deleteContentForward";
        draft.input({ value: "AB\nC", start: 1, end: 1, inputType }, { value: draft.value, start: 1, end: 1, inputType });
        expectValue(draft, "AB\rC");
    });

    it("tracks recomposition behind the caret and grouped browser undo and redo", () => {
        const original = "A\r\nB\rC\n";
        const draft = new EditMessageDraft(original);
        const inputType = "insertCompositionText";
        replace(draft, 1, 1, "拼", inputType);
        draft.input({ value: "A中文\nB\nC\n", start: 3, end: 3, inputType, data: "中文" }, {
            value: draft.value, start: 2, end: 2, inputType,
        });
        expectValue(draft, "A中文\r\nB\rC\n");

        draft.input({ value: normalizeTextareaLineEndings(original), start: 1, end: 1, inputType: "historyUndo" });
        expectValue(draft, original);
        draft.input({ value: "A中文\nB\nC\n", start: 3, end: 3, inputType: "historyRedo" });
        expectValue(draft, "A中文\r\nB\rC\n");
    });

    it("undoes and redoes a replacement even when the visible textarea value is identical", () => {
        const original = "A\r\nsame\rsame\nZ";
        const draft = new EditMessageDraft(original);
        replace(draft, 2, 7, "same\n", "insertFromPaste");
        expectValue(draft, "A\r\nsame\nsame\nZ");

        draft.input({ value: draft.value, start: 2, end: 7, inputType: "historyUndo" });
        expectValue(draft, original);
        draft.input({ value: draft.value, start: 7, end: 7, inputType: "historyRedo" });
        expectValue(draft, "A\r\nsame\nsame\nZ");
    });

    it.each([3, 13])("preserves the same 295,576-byte intermediate draft after %i undos", steps => {
        let line = 0;
        const source = `  \n长文原字-编辑回归专用前缀\n${"这是一行超过模型预算的中文正文。\n".repeat(6_000)}\n编辑回归唯一尾段-Q38V\n  `
            .replace(/\n/g, () => ["\r\n", "\n", "\r", "\n"][line++ % 4]);
        const draft = new EditMessageDraft(source);
        const versions = [draft.value];
        replace(draft, draft.value.length, draft.value.length, "X", "insertText");
        versions.push(draft.value);
        replace(draft, 0, 0, "  \n", "insertFromPaste");
        versions.push(draft.value);
        for (let i = 0; i < steps - 1; i++) {
            const caret = i % 2 === 0 ? 9 : draft.value.length - 3;
            replace(draft, caret, caret, "x", "insertText");
            versions.push(draft.value);
        }

        for (let i = versions.length - 2; i >= 1; i--) {
            draft.input({ value: versions[i], start: 0, end: 0, inputType: "historyUndo" });
        }
        const saved = draft.serialize();
        expect(draft.value === normalizeTextareaLineEndings(`${source}X`)).toBe(true);
        expect(new TextEncoder().encode(saved)).toHaveLength(295_576);
        expect(saved === `${source}X`).toBe(true);

        draft.input({ value: versions[2], start: 3, end: 3, inputType: "historyRedo" });
        expect(draft.serialize() === `  \n${source}X`).toBe(true);
        draft.input({ value: versions[1], start: 0, end: 0, inputType: "historyUndo" });

        // Insert the same duplicate on the opposite side of the first break.
        // Both branches have the same textarea value and distinct saved bytes.
        const branch = `  \r\n  \n${source.slice(4)}X`;
        replace(draft, 3, 3, "  \n", "insertFromPaste");
        expect(draft.value === versions[2]).toBe(true);
        expect(draft.serialize() === branch).toBe(true);
        draft.input({ value: versions[1], start: 3, end: 3, inputType: "historyUndo" });
        expect(draft.serialize() === `${source}X`).toBe(true);
        draft.input({ value: versions[2], start: 6, end: 6, inputType: "historyRedo" });
        expect(draft.serialize() === branch).toBe(true);
    });

    it("keeps reversible separator positions after the changed-text buffers exceed their budget", () => {
        const source = "same\r\nsame\n尾";
        const draft = new EditMessageDraft(source);
        replace(draft, draft.value.length, draft.value.length, "X", "insertText");
        const intermediate = draft.value;
        const replacement = "甲".repeat(600_000) + "\n";
        replace(draft, 4, 5, replacement, "insertFromPaste");
        const afterReplacement = draft.value;
        const secondPaste = "乙".repeat(600_000);
        replace(draft, draft.value.length, draft.value.length, secondPaste, "insertFromPaste");
        const afterTwoPastes = draft.value;

        draft.input({ value: intermediate, start: 4, end: 5, inputType: "historyUndo" });
        expectValue(draft, `${source}X`);

        draft.input({ value: afterReplacement, start: 4 + replacement.length, end: 4 + replacement.length, inputType: "historyRedo" });
        expect(draft.serialize() === `same${replacement}same\n尾X`).toBe(true);
        draft.input({ value: afterTwoPastes, start: afterTwoPastes.length, end: afterTwoPastes.length, inputType: "historyRedo" });
        expect(draft.serialize() === `same${replacement}same\n尾X${secondPaste}`).toBe(true);
    });

    it("recovers the original bytes on Undo when the document alone exceeds the patch text budget", () => {
        const original = `A\r\n${"界".repeat(1_000_000)}`;
        const draft = new EditMessageDraft(original);
        replace(draft, 1, 2, "", "deleteContentBackward");

        draft.input({ value: normalizeTextareaLineEndings(original), start: 1, end: 2, inputType: "historyUndo" });
        expect(draft.serialize() === original).toBe(true);
        expect(draft.value === normalizeTextareaLineEndings(original)).toBe(true);
    });

    it("uses bounded alignment for programmatic input without discarding the original on interim changes", () => {
        const original = "A\r\nB\rC\n";
        const draft = new EditMessageDraft(original);
        draft.input({ value: " ", start: 1, end: 1, inputType: "" });
        draft.input({ value: "A\nB\nC\n", start: 6, end: 6, inputType: "" });
        expectValue(draft, original);
        draft.input({ value: "Z\nB\nC\n", start: 6, end: 6, inputType: "" });
        expectValue(draft, "Z\r\nB\rC\n");
    });

    it("keeps proven Redo versions after a grouped Undo crosses an untracked input to the original", () => {
        const original = "A\r\nsame\rsame\nZ";
        const draft = new EditMessageDraft(original);
        draft.input({ value: `${draft.value}X`, start: 14, end: 14, inputType: "" });
        replace(draft, 2, 7, "same\n", "insertFromPaste");
        const intermediate = draft.value;
        replace(draft, draft.value.length, draft.value.length, "Y", "insertText");
        const later = draft.value;

        draft.input({ value: intermediate, start: intermediate.length, end: intermediate.length, inputType: "historyUndo" });
        expectValue(draft, "A\r\nsame\nsame\nZX");
        draft.input({ value: normalizeTextareaLineEndings(original), start: 1, end: 1, inputType: "historyUndo" });
        expectValue(draft, original);

        draft.input({ value: intermediate, start: 7, end: 7, inputType: "historyRedo" });
        expectValue(draft, "A\r\nsame\nsame\nZX");
        draft.input({ value: later, start: later.length, end: later.length, inputType: "historyRedo" });
        expectValue(draft, "A\r\nsame\nsame\nZXY");

        draft.input({ value: normalizeTextareaLineEndings(original), start: 1, end: 1, inputType: "historyUndo" });
        expectValue(draft, original);
        draft.input({ value: later, start: later.length, end: later.length, inputType: "historyRedo" });
        expectValue(draft, "A\r\nsame\nsame\nZXY");
    });
});
