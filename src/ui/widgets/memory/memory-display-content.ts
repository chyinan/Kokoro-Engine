// pattern: Functional Core
type StructuredMemoryContent = {
    prefix: string | null;
    text: string;
};

const STRUCTURED_MEMORY_PREFIX_PATTERN =
    /^\[((?:[a-z_]+:[^|\]]+)(?:\|[a-z_]+:[^|\]]+)*)\]\s*/i;

export function splitStructuredMemoryContent(content: string): StructuredMemoryContent {
    const match = content.match(STRUCTURED_MEMORY_PREFIX_PATTERN);
    if (!match) {
        return {
            prefix: null,
            text: content,
        };
    }

    return {
        prefix: match[0].trim(),
        text: content.slice(match[0].length),
    };
}

export function stripStructuredMemoryPrefix(content: string): string {
    return splitStructuredMemoryContent(content).text;
}

export function restoreStructuredMemoryPrefix(prefix: string | null, text: string): string {
    const trimmedText = text.trim();
    if (!prefix) {
        return trimmedText;
    }
    if (!trimmedText) {
        return prefix;
    }
    return `${prefix} ${trimmedText}`;
}
