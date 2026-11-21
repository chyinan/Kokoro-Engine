// pattern: Functional Core

export const normalizeTextareaLineEndings = (text: string) => text.replace(/\r\n?/g, "\n");

type Match = { source: number; draft: number; length: number };
type Range = { sourceStart: number; sourceEnd: number; draftStart: number; draftEnd: number };

const MAX_EDIT_DISTANCE = 192;
const MAX_DIFF_STEPS = 2_000_000;

function followsInsertion(frontier: Int32Array, origin: number, distance: number, diagonal: number) {
    return diagonal === -distance
        || (diagonal !== distance && frontier[origin + diagonal - 1] < frontier[origin + diagonal + 1]);
}

// Myers' shortest edit path. The frontier is indexed by the difference between
// source and draft positions. Only equal ranges along that path carry separators.
function matchChangedRange(
    source: ArrayLike<string>,
    draft: ArrayLike<string>,
    range: Range,
    budget: { steps: number },
): Match[] | undefined {
    const sourceLength = range.sourceEnd - range.sourceStart;
    const draftLength = range.draftEnd - range.draftStart;
    if (sourceLength === 0 || draftLength === 0) return [];
    if (budget.steps <= 0 || Math.abs(sourceLength - draftLength) > MAX_EDIT_DISTANCE) return undefined;

    const origin = MAX_EDIT_DISTANCE + 1;
    const frontier = new Int32Array(2 * origin + 1).fill(-1);
    frontier[origin + 1] = 0;
    const trace: Int32Array[] = [];
    for (let distance = 0; distance <= MAX_EDIT_DISTANCE; distance++) {
        for (let diagonal = -distance; diagonal <= distance; diagonal += 2) {
            if (--budget.steps < 0) return undefined;
            let x = followsInsertion(frontier, origin, distance, diagonal)
                ? frontier[origin + diagonal + 1]
                : frontier[origin + diagonal - 1] + 1;
            let y = x - diagonal;
            while (x < sourceLength && y < draftLength && source[range.sourceStart + x] === draft[range.draftStart + y]) {
                if (--budget.steps < 0) return undefined;
                x++;
                y++;
            }
            frontier[origin + diagonal] = x;
            if (x === sourceLength && y === draftLength) {
                const matches: Match[] = [];
                for (let edits = distance; edits > 0; edits--) {
                    const previous = trace[edits - 1];
                    const diagonal = x - y;
                    const priorDiagonal = followsInsertion(previous, origin, edits, diagonal)
                        ? diagonal + 1
                        : diagonal - 1;
                    const priorX = previous[origin + priorDiagonal];
                    const priorY = priorX - priorDiagonal;
                    const length = Math.min(x - priorX, y - priorY);
                    if (length > 0) {
                        matches.push({ source: range.sourceStart + x - length, draft: range.draftStart + y - length, length });
                    }
                    x = priorX;
                    y = priorY;
                }
                return matches;
            }
        }
        trace.push(frontier.slice());
    }
    return undefined;
}

function uniqueRuns(lines: Array<string | undefined>, start: number, end: number) {
    const runs = new Map<string, { index: number; length: number } | null>();
    for (let index = start; index < end;) {
        const line = lines[index];
        let next = index + 1;
        while (next < end && lines[next] === line) next++;
        if (line !== undefined) runs.set(line, runs.has(line) ? null : { index, length: next - index });
        index = next;
    }
    return runs;
}

// Use keys that occur in one run in each changed span. The longest increasing
// subsequence keeps the anchors in document order.
function findOrderedAnchors(source: Array<string | undefined>, draft: Array<string | undefined>, range: Range): Match[] {
    if (range.sourceStart === range.sourceEnd || range.draftStart === range.draftEnd) return [];
    const sourceRuns = uniqueRuns(source, range.sourceStart, range.sourceEnd);
    const draftRuns = uniqueRuns(draft, range.draftStart, range.draftEnd);
    const anchors: Match[] = [];
    const tails: number[] = [];
    const predecessors: number[] = [];
    for (const [line, sourceRun] of sourceRuns) {
        const draftRun = draftRuns.get(line);
        if (!sourceRun || !draftRun) continue;
        let low = 0;
        let high = tails.length;
        while (low < high) {
            const middle = Math.floor((low + high) / 2);
            if (anchors[tails[middle]].draft < draftRun.index) low = middle + 1;
            else high = middle;
        }
        predecessors.push(low === 0 ? -1 : tails[low - 1]);
        tails[low] = anchors.length;
        anchors.push({
            source: sourceRun.index,
            draft: draftRun.index,
            length: Math.min(sourceRun.length, draftRun.length),
        });
    }
    const ordered: Match[] = [];
    for (let index = tails[tails.length - 1]; index >= 0; index = predecessors[index]) {
        ordered.push(anchors[index]);
    }
    return ordered.reverse();
}

function matchEdges(
    source: ArrayLike<string>,
    draft: ArrayLike<string>,
    range: Range,
    match: (match: Match) => void,
): Range {
    let { sourceStart, sourceEnd, draftStart, draftEnd } = range;
    // Match the equal prefix first, then the suffix. Repeated lines at the
    // beginning of an edit keep their original position and separator.
    while (sourceStart < sourceEnd && draftStart < draftEnd && source[sourceStart] === draft[draftStart]) {
        sourceStart++;
        draftStart++;
    }
    if (sourceStart > range.sourceStart) {
        match({ source: range.sourceStart, draft: range.draftStart, length: sourceStart - range.sourceStart });
    }
    while (sourceEnd > sourceStart && draftEnd > draftStart && source[sourceEnd - 1] === draft[draftEnd - 1]) {
        sourceEnd--;
        draftEnd--;
    }
    if (sourceEnd < range.sourceEnd) {
        match({ source: sourceEnd, draft: draftEnd, length: range.sourceEnd - sourceEnd });
    }
    return { sourceStart, sourceEnd, draftStart, draftEnd };
}

function splitLines(text: string) {
    let offset = 0;
    const lines = text.split("\n");
    const offsets = lines.map(line => {
        const start = offset;
        offset += line.length + 1;
        return start;
    });
    offsets.push(text.length);
    return { lines, offsets };
}

function recognizeWrappedLines(source: string[], draft: string[], range: Range): string[] {
    if (range.sourceStart === range.sourceEnd || range.draftStart === range.draftEnd) return draft;
    const width = 8;
    const starts = new Map<string, string | null>();
    const ends = new Map<string, string | null>();
    const unchanged = new Set<string>();
    function index(map: Map<string, string | null>, key: string, line: string) {
        const previous = map.get(key);
        map.set(key, previous === undefined || previous === line ? line : null);
    }
    for (let i = range.sourceStart; i < range.sourceEnd; i++) {
        const line = source[i];
        unchanged.add(line);
        if (line.length < width || !line.trim()) continue;
        index(starts, line.slice(0, width), line);
        index(ends, line.slice(-width), line);
    }

    // An otherwise identical line with pasted text at either end can still
    // align with its original. This also keeps repeated lines in order without
    // searching every source line for each draft line.
    let keys = draft;
    for (let i = range.draftStart; i < range.draftEnd; i++) {
        const line = draft[i];
        if (line.length <= width || unchanged.has(line)) continue;
        const start = starts.get(line.slice(0, width));
        const end = ends.get(line.slice(-width));
        if (start === null || end === null) continue;
        const startingLine = start && line.startsWith(start) ? start : undefined;
        const endingLine = end && line.endsWith(end) ? end : undefined;
        if (startingLine && endingLine && startingLine !== endingLine) continue;
        const original = startingLine ?? endingLine;
        if (original === undefined) continue;
        if (keys === draft) keys = draft.slice();
        keys[i] = original;
    }
    return keys;
}

function lineBreaks(text: string, start: number, end: number): number[] {
    const offsets: number[] = [];
    for (let offset = text.indexOf("\n", start); offset !== -1 && offset < end; offset = text.indexOf("\n", offset + 1)) {
        offsets.push(offset);
    }
    return offsets;
}

function surroundingText(text: string, offset: number, before: number, after: number): string | undefined {
    // The newline is at a fixed position in the key, even beside empty lines.
    if (offset < before || offset + after >= text.length) return undefined;
    const context = text.slice(offset - before, offset + after + 1);
    return context.trim() ? context : undefined;
}

function precedingLineEnd(text: string, offset: number): string | undefined {
    const width = 8;
    if (offset < width) return undefined;
    const context = text.slice(offset - width, offset);
    return !context.includes("\n") && context.trim() ? context : undefined;
}

// A short, fully retained span can distinguish a break even when there are
// too few characters on each side for the fixed-width contexts.
function matchShortRetainedText(source: string, draft: string, range: Range): Match | undefined {
    const sourceLength = range.sourceEnd - range.sourceStart;
    const draftLength = range.draftEnd - range.draftStart;
    const sourceIsShorter = sourceLength <= draftLength;
    const length = Math.min(sourceLength, draftLength);
    if (length > 32) return undefined;
    const [shorter, longer] = sourceIsShorter
        ? [source.slice(range.sourceStart, range.sourceEnd), draft.slice(range.draftStart, range.draftEnd)]
        : [draft.slice(range.draftStart, range.draftEnd), source.slice(range.sourceStart, range.sourceEnd)];
    if (!shorter.trim() || !shorter.includes("\n")) return undefined;
    const start = longer.indexOf(shorter);
    if (start === -1 || longer.indexOf(shorter, start + 1) !== -1) return undefined;
    return {
        source: range.sourceStart + (sourceIsShorter ? 0 : start),
        draft: range.draftStart + (sourceIsShorter ? start : 0),
        length,
    };
}

const breakContexts = [
    (text: string, offset: number) => surroundingText(text, offset, 16, 16),
    (text: string, offset: number) => surroundingText(text, offset, 4, 4),
    (text: string, offset: number) => surroundingText(text, offset, 8, 1),
    (text: string, offset: number) => surroundingText(text, offset, 1, 8),
    precedingLineEnd,
];

function matchBreaksByContext(source: string, draft: string, range: Range, preserve: (match: Match) => void) {
    const contained = matchShortRetainedText(source, draft, range);
    if (contained !== undefined) {
        preserve(contained);
        return;
    }
    const sourceBreaks = lineBreaks(source, range.sourceStart, range.sourceEnd);
    const draftBreaks = lineBreaks(draft, range.draftStart, range.draftEnd);
    if (!sourceBreaks.length || !draftBreaks.length) {
        matchEdges(source, draft, range, preserve);
        return;
    }
    if (sourceBreaks.length === 1 && draftBreaks.length === 1) {
        const [sourceBreak] = sourceBreaks;
        const [draftBreak] = draftBreaks;
        const sourcePrefix = source.slice(range.sourceStart, sourceBreak);
        const draftPrefix = draft.slice(range.draftStart, draftBreak);
        const sourceSuffix = source.slice(sourceBreak + 1, range.sourceEnd);
        const draftSuffix = draft.slice(draftBreak + 1, range.draftEnd);
        // Pasting on both sides of the break can change its immediate context.
        // Exact text retained on both outer edges still identifies the break.
        if (sourcePrefix && draftPrefix && sourceSuffix && draftSuffix
            && (sourcePrefix.startsWith(draftPrefix) || draftPrefix.startsWith(sourcePrefix))
            && (sourceSuffix.endsWith(draftSuffix) || draftSuffix.endsWith(sourceSuffix))) {
            preserve({ source: sourceBreak, draft: draftBreak, length: 1 });
            return;
        }
    }

    // Compare a fixed number of characters near each break. Longer, more
    // distinctive contexts constrain the gaps searched with shorter contexts.
    // If the following line changed, the preceding line can still own the break.
    let anchors: Match[] = [];
    for (const context of breakContexts) {
        const sourceKeys = sourceBreaks.map(offset => context(source, offset));
        const draftKeys = draftBreaks.map(offset => context(draft, offset));
        const refined: Match[] = [];
        let sourceStart = 0;
        let draftStart = 0;
        function refine(sourceEnd: number, draftEnd: number) {
            for (const match of findOrderedAnchors(sourceKeys, draftKeys, { sourceStart, sourceEnd, draftStart, draftEnd })) {
                refined.push(match);
            }
        }
        for (const anchor of anchors) {
            refine(anchor.source, anchor.draft);
            refined.push(anchor);
            sourceStart = anchor.source + anchor.length;
            draftStart = anchor.draft + anchor.length;
        }
        refine(sourceBreaks.length, draftBreaks.length);
        anchors = refined;
    }

    // Apply prefix and suffix matches only between confirmed breaks so a bare
    // newline at a gap edge cannot take precedence over matching content.
    let { sourceStart, draftStart } = range;
    for (const anchor of anchors) {
        for (let i = 0; i < anchor.length; i++) {
            const sourceEnd = sourceBreaks[anchor.source + i];
            const draftEnd = draftBreaks[anchor.draft + i];
            matchEdges(source, draft, { sourceStart, sourceEnd, draftStart, draftEnd }, preserve);
            preserve({ source: sourceEnd, draft: draftEnd, length: 1 });
            sourceStart = sourceEnd + 1;
            draftStart = draftEnd + 1;
        }
    }
    matchEdges(source, draft, { ...range, sourceStart, draftStart }, preserve);
}

export type TextareaLineEndings = ReadonlyMap<number, string>;

export function readTextareaLineEndings(source: string): TextareaLineEndings {
    const endings = new Map<number, string>();
    let removed = 0;
    for (const match of source.matchAll(/\r\n?/g)) {
        endings.set(match.index! - removed, match[0]);
        removed += match[0].length - 1;
    }
    return endings;
}

export function matchTextareaLineEndings(normalized: string, draft: string, originalEndings: TextareaLineEndings): TextareaLineEndings {
    if (originalEndings.size === 0 || normalized === draft) return originalEndings;

    const preserved = new Map<number, string>();
    function preserveCharacters({ source, draft, length }: Match) {
        const end = source + length;
        for (let offset = normalized.indexOf("\n", source); offset !== -1 && offset < end; offset = normalized.indexOf("\n", offset + 1)) {
            const ending = originalEndings.get(offset);
            if (ending !== undefined) preserved.set(draft + offset - source, ending);
        }
    }

    const budget = { steps: MAX_DIFF_STEPS };
    const before = splitLines(normalized);
    const after = splitLines(draft);
    const lines: Match[] = [];
    const changedLines = matchEdges(before.lines, after.lines, {
        sourceStart: 0, sourceEnd: before.lines.length,
        draftStart: 0, draftEnd: after.lines.length,
    }, match => lines.push(match));
    const draftKeys = recognizeWrappedLines(before.lines, after.lines, changedLines);
    const lineRange = draftKeys === after.lines ? changedLines
        : matchEdges(before.lines, draftKeys, changedLines, match => lines.push(match));
    const matchedLines = matchChangedRange(before.lines, draftKeys, lineRange, budget)
        ?? findOrderedAnchors(before.lines, draftKeys, lineRange);
    for (const match of matchedLines) lines.push(match);
    lines.sort((a, b) => a.source - b.source);

    // A retained line keeps the separator immediately after it. Match smaller
    // text edits inside the gaps between lines paired by their original text.
    let sourceStart = 0;
    let draftStart = 0;
    function matchGap(sourceEnd: number, draftEnd: number) {
        const gap = {
            sourceStart: before.offsets[sourceStart], sourceEnd: before.offsets[sourceEnd],
            draftStart: after.offsets[draftStart], draftEnd: after.offsets[draftEnd],
        };
        const edges: Match[] = [];
        const range = matchEdges(normalized, draft, gap, match => edges.push(match));
        const changes = matchChangedRange(normalized, draft, range, budget);
        if (changes === undefined) {
            matchBreaksByContext(normalized, draft, gap, preserveCharacters);
            return;
        }
        for (const match of edges) preserveCharacters(match);
        for (const match of changes) preserveCharacters(match);
    }
    for (const match of lines) {
        matchGap(match.source, match.draft);
        for (let i = 0; i < match.length; i++) {
            const oldLine = match.source + i;
            const newLine = match.draft + i;
            const ending = originalEndings.get(before.offsets[oldLine] + before.lines[oldLine].length);
            if (ending !== undefined && newLine < after.lines.length - 1) {
                preserved.set(after.offsets[newLine] + after.lines[newLine].length, ending);
            }
        }
        sourceStart = match.source + match.length;
        draftStart = match.draft + match.length;
    }
    matchGap(before.lines.length, after.lines.length);
    return preserved;
}

export function applyTextareaLineEndings(draft: string, preserved: TextareaLineEndings): string {
    if (preserved.size === 0) return draft;
    let loneCarriageReturn = -2;
    return draft.replace(/\n/g, (_, offset: number) => {
        const ending = preserved.get(offset) ?? "\n";
        // A lone CR followed by LF would collapse two textarea breaks into one.
        const separator = ending === "\n" && loneCarriageReturn === offset - 1 ? "\r\n" : ending;
        if (separator === "\r") loneCarriageReturn = offset;
        return separator;
    });
}

export function restoreEditedLineEndings(source: string, draft: string): string {
    if (!source.includes("\r")) return draft;
    const normalized = normalizeTextareaLineEndings(source);
    const endings = matchTextareaLineEndings(normalized, draft, readTextareaLineEndings(source));
    return applyTextareaLineEndings(draft, endings);
}
