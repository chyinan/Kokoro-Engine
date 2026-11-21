import type { TextareaLineEndings } from "./edit-line-endings";

export type DraftVersion = { text: string; endings: TextareaLineEndings };
export type Splice = { start: number; end: number; inserted: number };

type Ending = [offset: number, separator: string];
type TextMark = { length: number; first: number; second: number };
type Patch = { length: number; endings: Ending[]; units?: Uint16Array };
type Edit = { start: number; before: TextMark; after: TextMark; removed: Patch; inserted: Patch };
// A piece without a patch refers to the current textarea text.
type Piece = { offset: number; length: number; patch?: Patch };

const MAX_RETAINED_PATCH_UNITS = 1_000_000;

function markText(text: string): TextMark {
    let first = 0x811c9dc5;
    let second = 0x9e3779b9;
    for (let i = 0; i < text.length; i++) {
        const unit = text.charCodeAt(i);
        first = Math.imul(first ^ unit, 0x01000193);
        second = Math.imul(second + unit, 0x85ebca77) ^ (second >>> 13);
    }
    return { length: text.length, first, second };
}

function sameMark(a: TextMark, b: TextMark): boolean {
    return a.length === b.length && a.first === b.first && a.second === b.second;
}

function copyUnits(text: string, start: number, length: number): Uint16Array {
    // A small string slice can retain the entire previous textarea string.
    const units = new Uint16Array(length);
    for (let i = 0; i < length; i++) units[i] = text.charCodeAt(start + i);
    return units;
}

function splitAt(pieces: Piece[], offset: number): number {
    for (let i = 0; i < pieces.length; i++) {
        if (offset === 0) return i;
        const piece = pieces[i];
        if (offset < piece.length) {
            pieces.splice(i, 1,
                { ...piece, length: offset },
                { ...piece, offset: piece.offset + offset, length: piece.length - offset });
            return i + 1;
        }
        offset -= piece.length;
    }
    return pieces.length;
}

function applyEdit(pieces: Piece[], edit: Edit, undo: boolean) {
    const replaced = undo ? edit.inserted : edit.removed;
    const patch = undo ? edit.removed : edit.inserted;
    const start = splitAt(pieces, edit.start);
    const end = splitAt(pieces, edit.start + replaced.length);
    if (patch.length) pieces.splice(start, end - start, { offset: 0, length: patch.length, patch });
    else pieces.splice(start, end - start);
}

function matchesPieces(current: string, target: string, pieces: Piece[]): boolean {
    let offset = 0;
    for (const piece of pieces) {
        if (offset + piece.length > target.length) return false;
        if (!piece.patch) {
            if (!target.startsWith(current.slice(piece.offset, piece.offset + piece.length), offset)) return false;
        } else if (piece.patch.units) {
            for (let i = 0; i < piece.length; i++) {
                if (target.charCodeAt(offset + i) !== piece.patch.units[piece.offset + i]) return false;
            }
        }
        offset += piece.length;
    }
    return offset === target.length;
}

function firstEndingAtOrAfter(endings: Ending[], offset: number): number {
    let low = 0;
    let high = endings.length;
    while (low < high) {
        const middle = Math.floor((low + high) / 2);
        if (endings[middle][0] < offset) low = middle + 1;
        else high = middle;
    }
    return low;
}

function orderedEndings(endings: TextareaLineEndings): Ending[] {
    return [...endings].sort(([left], [right]) => left - right);
}

function collectEndings(pieces: Piece[], currentEndings: TextareaLineEndings, target: string): TextareaLineEndings | undefined {
    const original = orderedEndings(currentEndings);
    const result = new Map<number, string>();
    let offset = 0;
    for (const piece of pieces) {
        const endings = piece.patch?.endings ?? original;
        for (let i = firstEndingAtOrAfter(endings, piece.offset); i < endings.length; i++) {
            const [position, separator] = endings[i];
            if (position >= piece.offset + piece.length) break;
            const targetOffset = offset + position - piece.offset;
            if (target[targetOffset] !== "\n") return undefined;
            result.set(targetOffset, separator);
        }
        offset += piece.length;
    }
    return result;
}

export class EditMessageHistory {
    private edits: Edit[] = [];
    private cursor = 0;
    private firstRetainedText = 0;
    private retainedUnits = 0;

    clear() {
        this.edits = [];
        this.cursor = 0;
        this.firstRetainedText = 0;
        this.retainedUnits = 0;
    }

    private dropText(edit: Edit) {
        this.retainedUnits -= (edit.removed.units?.length ?? 0) + (edit.inserted.units?.length ?? 0);
        edit.removed.units = undefined;
        edit.inserted.units = undefined;
    }

    discardRedo() {
        for (let i = this.cursor; i < this.edits.length; i++) this.dropText(this.edits[i]);
        this.edits.length = this.cursor;
        this.firstRetainedText = Math.min(this.firstRetainedText, this.cursor);
    }

    record(previous: DraftVersion, next: string, { start, end, inserted }: Splice) {
        this.discardRedo();
        const before = this.edits[this.cursor - 1]?.after ?? markText(previous.text);
        const after = next === previous.text ? before : markText(next);
        const removedEndings: Ending[] = [];
        if (start !== end) {
            for (const [offset, separator] of previous.endings) {
                if (offset >= start && offset < end) removedEndings.push([offset - start, separator]);
            }
            removedEndings.sort(([left], [right]) => left - right);
        }
        const edit: Edit = {
            start, before, after,
            removed: { length: end - start, endings: removedEndings },
            inserted: { length: inserted, endings: [] },
        };
        const changedUnits = edit.removed.length + inserted;
        if (changedUnits <= MAX_RETAINED_PATCH_UNITS) {
            while (this.retainedUnits + changedUnits > MAX_RETAINED_PATCH_UNITS) {
                this.dropText(this.edits[this.firstRetainedText++]);
            }
            if (edit.removed.length) edit.removed.units = copyUnits(previous.text, start, edit.removed.length);
            if (inserted) edit.inserted.units = copyUnits(next, start, inserted);
            this.retainedUnits += changedUnits;
        }
        // Keep the compact splice and its separator provenance for the whole
        // edit session, even when the optional text buffer exceeds its budget.
        this.edits.push(edit);
        this.cursor++;
    }

    reconnectToOriginal(original: DraftVersion, previous: DraftVersion, keepRedo: boolean) {
        // An undo can cross an untracked input and land on the original.
        // Keep one reversible transition to the previous state and its known redo suffix.
        if (keepRedo) {
            for (const edit of this.edits.splice(0, this.cursor)) this.dropText(edit);
            this.firstRetainedText = Math.max(0, this.firstRetainedText - this.cursor);
            this.cursor = 0;
        } else {
            this.clear();
        }
        if (original.text === previous.text && original.endings.size === previous.endings.size
            && [...original.endings].every(([offset, ending]) => previous.endings.get(offset) === ending)) return;

        const bridge: Edit = {
            start: 0,
            before: markText(original.text),
            after: this.edits[0]?.before ?? markText(previous.text),
            removed: { length: original.text.length, endings: orderedEndings(original.endings) },
            inserted: { length: previous.text.length, endings: orderedEndings(previous.endings) },
        };
        const changedUnits = bridge.removed.length + bridge.inserted.length;
        if (this.retainedUnits + changedUnits <= MAX_RETAINED_PATCH_UNITS) {
            bridge.removed.units = copyUnits(original.text, 0, bridge.removed.length);
            bridge.inserted.units = copyUnits(previous.text, 0, bridge.inserted.length);
            this.retainedUnits += changedUnits;
            this.firstRetainedText = 0;
        } else {
            this.firstRetainedText++;
        }
        this.edits.unshift(bridge);
    }

    restore(current: DraftVersion, target: string, undo: boolean): TextareaLineEndings | undefined {
        if (undo ? this.cursor === 0 : this.cursor === this.edits.length) return undefined;
        const mark = markText(target);
        const pieces: Piece[] = current.text ? [{ offset: 0, length: current.text.length }] : [];
        const step = undo ? -1 : 1;
        let applied = this.cursor;
        for (let cursor = this.cursor + step; cursor >= 0 && cursor <= this.edits.length; cursor += step) {
            const boundary = undo ? this.edits[cursor].before : this.edits[cursor - 1].after;
            if (!sameMark(mark, boundary)) continue;
            // A native history action can group multiple input notifications.
            // Replay the candidate with slices of existing text and compact
            // patches, keeping the textarea untouched until it matches.
            while (applied !== cursor) {
                applyEdit(pieces, this.edits[undo ? applied - 1 : applied], undo);
                applied += step;
            }
            if (!matchesPieces(current.text, target, pieces)) continue;
            const endings = collectEndings(pieces, current.endings, target);
            if (!endings) continue;
            this.cursor = cursor;
            return endings;
        }
        return undefined;
    }
}
