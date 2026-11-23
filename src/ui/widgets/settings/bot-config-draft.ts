// pattern: Functional Core

import type { BotConfig } from "@/lib/kokoro-bridge";

export interface BotConfigConflictReview {
    baseline: BotConfig;
    draft: BotConfig;
    saved: BotConfig;
}

export function rebaseBotConfig(
    baseline: BotConfig,
    draft: BotConfig,
    saved: BotConfig,
    review?: BotConfigConflictReview,
) {
    let hasOverlap = false;
    const confirmed = review?.baseline === baseline && review.saved.revision === saved.revision
        ? review
        : undefined;

    function reapplyFields<T extends object>(select: (config: BotConfig) => T): T {
        const previous = select(baseline);
        const edited = select(draft);
        const latest = select(saved);
        const reviewed = confirmed && { draft: select(confirmed.draft), saved: select(confirmed.saved) };
        const result = { ...latest };
        for (const key of Object.keys(edited) as Array<keyof T>) {
            // Arrays, including allowlists, are edited as whole values.
            const before = JSON.stringify(previous[key]);
            const pending = JSON.stringify(edited[key]);
            const current = JSON.stringify(latest[key]);
            if (pending === before || pending === current) continue;
            if (before === current) {
                result[key] = edited[key];
            } else if (
                !reviewed || pending !== JSON.stringify(reviewed.draft[key])
                || current !== JSON.stringify(reviewed.saved[key])
            ) {
                hasOverlap = true;
            }
        }
        return result;
    }

    const rebased = {
        ...saved,
        ...reapplyFields(config => ({ selected_platform: config.selected_platform })),
        telegram: reapplyFields(config => config.telegram),
        qq: reapplyFields(config => config.qq),
        discord: reapplyFields(config => config.discord),
        line: reapplyFields(config => config.line),
        webhook: reapplyFields(config => config.webhook),
    };
    return { draft: rebased, hasOverlap };
}
