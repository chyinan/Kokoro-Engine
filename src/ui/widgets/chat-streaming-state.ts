export type StreamingBubbleMessage = {
    readonly role: "user" | "kokoro" | "tool";
};

export type TypingIndicatorStateOptions = {
    readonly activeMessageIndex: number | null;
    readonly isThinking: boolean;
    readonly messages: ReadonlyArray<StreamingBubbleMessage>;
};

export type LiveTurnToolVisibilityOptions = {
    readonly activeMessageIndex: number | null;
    readonly approvalStatus?: "requested" | "approved" | "rejected";
    readonly messages: ReadonlyArray<StreamingBubbleMessage>;
};

export type StreamingRevealOptions = {
    readonly accumulatedText: string;
    readonly delta: string;
    readonly hasVisibleTextStarted: boolean;
};

export function hasActiveKokoroBubble(
    messages: ReadonlyArray<StreamingBubbleMessage>,
    index: number | null
): boolean {
    return (
        index !== null
        && index >= 0
        && index < messages.length
        && messages[index]?.role === "kokoro"
    );
}

export function shouldRenderTypingIndicator(options: TypingIndicatorStateOptions): boolean {
    if (!options.isThinking) {
        return false;
    }

    return !hasActiveKokoroBubble(options.messages, options.activeMessageIndex);
}

export function shouldRevealLiveTurnToolTrace(options: LiveTurnToolVisibilityOptions): boolean {
    if (options.approvalStatus === "requested") {
        return true;
    }

    return hasActiveKokoroBubble(options.messages, options.activeMessageIndex);
}

export function hasVisibleAssistantContent(text: string): boolean {
    return /\S/u.test(text);
}

export function getStreamingRevealText(options: StreamingRevealOptions): string | null {
    if (options.hasVisibleTextStarted) {
        return options.delta;
    }

    if (!hasVisibleAssistantContent(options.accumulatedText)) {
        return null;
    }

    return options.accumulatedText;
}
