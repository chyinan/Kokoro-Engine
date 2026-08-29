// pattern: Imperative Shell

import {
    fetchModels,
    listOllamaModels,
    saveLlmConfig,
    testLlmConnection,
    type LlmConfig,
    type LlmConnectionTestResult,
    type OllamaModelInfo,
} from "../../lib/kokoro-bridge";
import {
    applyProviderSetupToConfig,
    normalizeProviderSetup,
    type ProviderSetup,
} from "./provider-setup-core";

export {
    OPENAI_COMPATIBLE_PRESETS,
    createProvider,
    getDefaultModel,
    getProviderTypeLabel,
    normalizeProviderSetup,
    providerToSetup,
} from "./provider-setup-core";
export type {
    ProviderPreset,
    ProviderSetup,
    SupportedProviderType,
} from "./provider-setup-core";

export type ProviderSetupDependencies = Readonly<{
    fetchModels?: (endpoint: string, apiKey: string) => Promise<Array<string>>;
    listOllamaModels?: (baseUrl: string) => Promise<Array<OllamaModelInfo>>;
    saveLlmConfig?: (config: LlmConfig) => Promise<void>;
    testLlmConnection?: (config: LlmConfig) => Promise<LlmConnectionTestResult>;
}>;

/** Discovers models through the bridge after the pure setup has been normalized. */
export async function discoverProviderModels(
    setup: Readonly<ProviderSetup>,
    dependencies: ProviderSetupDependencies = {},
): Promise<Array<string>> {
    const normalized = normalizeProviderSetup(setup);
    if (normalized.providerType === "ollama") {
        const discover = dependencies.listOllamaModels ?? listOllamaModels;
        const models = await discover(normalized.endpoint);
        return models.map((model) => model.name.trim()).filter((name) => name.length > 0);
    }
    if (normalized.providerType === "openai" || normalized.providerType === "openai_responses") {
        const discover = dependencies.fetchModels ?? fetchModels;
        return discover(normalized.endpoint, normalized.apiKey ?? "");
    }
    return [];
}

/** Persists a pure setup transformation through the bridge and returns its result. */
export async function saveProviderSetup(
    config: Readonly<LlmConfig>,
    setup: Readonly<ProviderSetup>,
    dependencies: ProviderSetupDependencies = {},
): Promise<LlmConfig> {
    const updated = applyProviderSetupToConfig(config, setup);
    const persist = dependencies.saveLlmConfig ?? saveLlmConfig;
    await persist(updated);
    return updated;
}

/** Tests a pure setup transformation through the bridge without persisting it. */
export async function testProviderSetup(
    config: Readonly<LlmConfig>,
    setup: Readonly<ProviderSetup>,
    dependencies: ProviderSetupDependencies = {},
): Promise<LlmConnectionTestResult> {
    const test = dependencies.testLlmConnection ?? testLlmConnection;
    return test(applyProviderSetupToConfig(config, setup));
}
