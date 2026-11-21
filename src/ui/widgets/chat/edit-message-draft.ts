import {
    applyTextareaLineEndings,
    matchTextareaLineEndings,
    normalizeTextareaLineEndings,
    readTextareaLineEndings,
    type TextareaLineEndings,
} from "./edit-line-endings";
import { EditMessageHistory, type DraftVersion, type Splice } from "./edit-message-history";

export interface TextareaEdit {
    value: string;
    start: number;
    end: number;
    inputType: string;
    data?: string | null;
}

function checkSplice(before: TextareaEdit, after: TextareaEdit, start: number, end: number): Splice | undefined {
    const inserted = after.value.length - before.value.length + end - start;
    if (start < 0 || start > end || end > before.value.length || inserted < 0 || after.start !== start + inserted) {
        return undefined;
    }
    if (!after.value.startsWith(before.value.slice(0, start)) || !after.value.endsWith(before.value.slice(end))) {
        return undefined;
    }
    return { start, end, inserted };
}

function editedRange(before: TextareaEdit, after: TextareaEdit): Splice | undefined {
    const { inputType } = after;
    if (before.inputType !== inputType || after.start !== after.end) return undefined;

    if (inputType.startsWith("delete")) {
        const removed = before.value.length - after.value.length;
        if (removed <= 0) return undefined;
        if (before.start !== before.end) {
            return removed === before.end - before.start ? checkSplice(before, after, before.start, before.end) : undefined;
        }
        // The browser may delete a grapheme, word, or line, in either direction.
        const end = after.start + removed;
        if (before.start < after.start || before.start > end) return undefined;
        if (inputType.endsWith("Backward") && before.start !== end) return undefined;
        if (inputType.endsWith("Forward") && before.start !== after.start) return undefined;
        return checkSplice(before, after, after.start, end);
    }

    if (!inputType.startsWith("insert")) return undefined;
    if (inputType === "insertCompositionText" || inputType === "insertFromComposition" || inputType === "insertReplacementText") {
        // A composition or replacement may edit previous text behind the caret.
        const data = after.data == null ? undefined : normalizeTextareaLineEndings(after.data);
        if (data !== undefined) {
            const start = after.start - data.length;
            const end = after.end - (after.value.length - before.value.length);
            if (start <= before.start && end >= before.end && after.value.startsWith(data, start)) {
                return checkSplice(before, after, start, end);
            }
        }
    }
    return checkSplice(before, after, before.start, before.end);
}

function moveEndings(endings: TextareaLineEndings, { start, end, inserted }: Splice): TextareaLineEndings {
    const shift = inserted - (end - start);
    let affected = false;
    for (const offset of endings.keys()) {
        if (offset >= start && (offset < end || shift !== 0)) {
            affected = true;
            break;
        }
    }
    if (!affected) return endings;

    const moved = new Map<number, string>();
    for (const [offset, separator] of endings) {
        if (offset < start) moved.set(offset, separator);
        else if (offset >= end) moved.set(offset + shift, separator);
    }
    return moved;
}

export class EditMessageDraft {
    private text: string;
    private readonly initial: DraftVersion;
    private tracked: DraftVersion;
    private readonly history = new EditMessageHistory();

    constructor(source: string) {
        this.text = normalizeTextareaLineEndings(source);
        this.initial = { text: this.text, endings: readTextareaLineEndings(source) };
        this.tracked = this.initial;
    }

    get value(): string {
        return this.text;
    }

    private version(): DraftVersion {
        if (this.text === this.tracked.text) return this.tracked;
        return { text: this.text, endings: matchTextareaLineEndings(this.tracked.text, this.text, this.tracked.endings) };
    }

    input(after: TextareaEdit, before?: TextareaEdit) {
        if (this.initial.endings.size === 0) {
            this.text = after.value;
            return;
        }
        const undo = after.inputType === "historyUndo";
        const redo = after.inputType === "historyRedo";
        if (undo || redo) {
            if (this.text !== this.tracked.text && after.value === this.tracked.text) {
                this.text = after.value;
                return;
            }
            const endings = this.history.restore(this.tracked, after.value, undo);
            if (endings) {
                this.tracked = { text: after.value, endings };
                this.text = after.value;
                return;
            }
            if (undo && after.value === this.initial.text) {
                const previous = this.version();
                this.history.reconnectToOriginal(this.initial, previous, previous === this.tracked);
                this.tracked = this.initial;
                this.text = this.initial.text;
                return;
            }
        }

        const splice = before?.value === this.text ? editedRange(before, after) : undefined;
        if (splice) {
            const previous = this.version();
            const endings = moveEndings(previous.endings, splice);
            if (after.value !== previous.text || endings !== previous.endings) {
                if (previous !== this.tracked) this.history.clear();
                this.history.record(previous, after.value, splice);
                this.tracked = { text: after.value, endings };
            }
        } else if (after.value !== this.text && !undo && !redo) {
            this.history.discardRedo();
        }
        // Without a verified edit range, keep the last known separators until
        // another native edit or a save needs to reconcile the current value.
        this.text = after.value;
    }

    serialize(): string {
        return applyTextareaLineEndings(this.text, this.version().endings);
    }
}
