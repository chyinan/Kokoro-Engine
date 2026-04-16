import { describe, expect, it } from "vitest";
import type { MemoryUpgradeConfig } from "../../../lib/kokoro-bridge";
import { updateMemoryUpgradeFlag } from "./memory-upgrade-flags";

function createConfig(overrides: Partial<MemoryUpgradeConfig> = {}): MemoryUpgradeConfig {
    return {
        observability_enabled: false,
        event_trigger_enabled: false,
        event_cooldown_secs: 120,
        structured_memory_enabled: false,
        intent_routing_enabled: false,
        retrieval_eval_enabled: false,
        ...overrides,
    };
}

describe("updateMemoryUpgradeFlag", () => {
    it("enables observability when retrieval eval is turned on", () => {
        const next = updateMemoryUpgradeFlag(
            createConfig(),
            "retrieval_eval_enabled",
            true,
        );

        expect(next.retrieval_eval_enabled).toBe(true);
        expect(next.observability_enabled).toBe(true);
    });

    it("turns off retrieval eval when observability is disabled", () => {
        const next = updateMemoryUpgradeFlag(
            createConfig({
                observability_enabled: true,
                retrieval_eval_enabled: true,
            }),
            "observability_enabled",
            false,
        );

        expect(next.observability_enabled).toBe(false);
        expect(next.retrieval_eval_enabled).toBe(false);
    });
});
