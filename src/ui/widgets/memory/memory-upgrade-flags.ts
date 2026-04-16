// pattern: Functional Core
import type { MemoryUpgradeConfig } from "../../../lib/kokoro-bridge";

export type MemoryUpgradeFlagKey =
    | "observability_enabled"
    | "event_trigger_enabled"
    | "structured_memory_enabled"
    | "intent_routing_enabled"
    | "retrieval_eval_enabled";

export function updateMemoryUpgradeFlag(
    config: Readonly<MemoryUpgradeConfig>,
    key: MemoryUpgradeFlagKey,
    enabled: boolean,
): MemoryUpgradeConfig {
    const nextObservability =
        key === "retrieval_eval_enabled" && enabled
            ? true
            : key === "observability_enabled"
                ? enabled
                : config.observability_enabled;
    const nextRetrievalEval =
        key === "observability_enabled" && !enabled
            ? false
            : key === "retrieval_eval_enabled"
                ? enabled
                : config.retrieval_eval_enabled;

    return {
        ...config,
        [key]: enabled,
        observability_enabled: nextObservability,
        retrieval_eval_enabled: nextRetrievalEval,
    };
}
