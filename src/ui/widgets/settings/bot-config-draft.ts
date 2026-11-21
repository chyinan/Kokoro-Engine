// pattern: Functional Core

import type { BotConfig } from "@/lib/kokoro-bridge";

export function rebaseBotConfig(baseline: BotConfig, draft: BotConfig, saved: BotConfig) {
    let hasOverlap = false;
    function reapplyFields<T extends object>(previous: T, edited: T, latest: T): T {
        const result = { ...latest };
        for (const key of Object.keys(edited) as Array<keyof T>) {
            // Arrays, including allowlists, are edited as whole values.
            const before = JSON.stringify(previous[key]);
            const pending = JSON.stringify(edited[key]);
            const current = JSON.stringify(latest[key]);
            if (pending === before || pending === current) continue;
            if (before === current) {
                result[key] = edited[key];
            } else {
                hasOverlap = true;
            }
        }
        return result;
    }

    const selection = reapplyFields(
        { selected_platform: baseline.selected_platform },
        { selected_platform: draft.selected_platform },
        { selected_platform: saved.selected_platform },
    );
    const rebased = {
        ...saved,
        ...selection,
        telegram: reapplyFields(baseline.telegram, draft.telegram, saved.telegram),
        qq: reapplyFields(baseline.qq, draft.qq, saved.qq),
        discord: reapplyFields(baseline.discord, draft.discord, saved.discord),
        line: reapplyFields(baseline.line, draft.line, saved.line),
        webhook: reapplyFields(baseline.webhook, draft.webhook, saved.webhook),
    };
    return { draft: rebased, hasOverlap };
}
