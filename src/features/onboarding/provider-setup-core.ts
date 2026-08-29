// pattern: Functional Core

import type { LlmConfig, LlmProviderConfig } from "../../lib/kokoro-bridge";

/** Provider types available from the focused first-run setup. */
export type SupportedProviderType = "openai" | "openai_responses" | "anthropic" | "ollama" | "llama_cpp";

/** A small, curated set of OpenAI-compatible endpoints suitable for onboarding. */
export type ProviderPreset = Readonly<{
    id: string;
    label: string;
    providerType: SupportedProviderType;
    baseUrl: string;
    model: string;
    requiresApiKey: boolean;
}>;

export const OPENAI_COMPATIBLE_PRESETS: ReadonlyArray<ProviderPreset> = [
    {
        id: "openai",
        label: "OpenAI",
        providerType: "openai",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        requiresApiKey: true,
    },
    {
        id: "deepseek",
        label: "DeepSeek",
        providerType: "openai",
        baseUrl: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
        requiresApiKey: true,
    },
    {
        id: "openrouter",
        label: "OpenRouter",
        providerType: "openai",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "openai/gpt-4o-mini",
        requiresApiKey: true,
    },
    {
        id: "groq",
        label: "Groq",
        providerType: "openai",
        baseUrl: "https://api.groq.com/openai/v1",
        model: "llama-3.3-70b-versatile",
        requiresApiKey: true,
    },
];

/** Values rendered by ProviderSetupStep. Null denotes an intentionally empty field. */
export type ProviderSetup = Readonly<{
    providerType: SupportedProviderType;
    presetId: string | null;
    endpoint: string;
    apiKey: string | null;
    model: string;
}>;

function trimTrailingSlashes(value: string): string {
    return value.trim().replace(/\/+$/, "");
}

function getPreset(presetId: string | null): ProviderPreset | null {
    if (!presetId) return null;
    return OPENAI_COMPATIBLE_PRESETS.find((preset) => preset.id === presetId) ?? null;
}

function getDefaultBaseUrl(providerType: SupportedProviderType): string {
    switch (providerType) {
        case "ollama":
            return "http://localhost:11434";
        case "llama_cpp":
            return "http://127.0.0.1:8080";
        case "anthropic":
            return "https://api.anthropic.com/v1";
        default:
            return "https://api.openai.com/v1";
    }
}

/** Returns the default model used when a focused setup has no model value. */
export function getDefaultModel(providerType: SupportedProviderType): string {
    switch (providerType) {
        case "openai_responses":
            return "gpt-4o";
        case "anthropic":
            return "claude-sonnet-4-20250514";
        case "ollama":
            return "llama3";
        case "llama_cpp":
            return "";
        default:
            return "gpt-4";
    }
}

/** Normalizes fields at the UI boundary while keeping secrets out of local providers. */
export function normalizeProviderSetup(setup: Readonly<ProviderSetup>): ProviderSetup {
    const preset = setup.providerType === "openai" || setup.providerType === "openai_responses"
        ? getPreset(setup.presetId)
        : null;
    const endpoint = trimTrailingSlashes(setup.endpoint) || preset?.baseUrl || getDefaultBaseUrl(setup.providerType);
    const model = setup.model.trim() || preset?.model || getDefaultModel(setup.providerType);
    const apiKey = setup.providerType === "ollama" || setup.providerType === "llama_cpp"
        ? null
        : setup.apiKey?.trim() || null;

    return {
        providerType: setup.providerType,
        presetId: preset?.id ?? null,
        endpoint,
        apiKey,
        model,
    };
}

/** Converts a persisted provider into the compact setup representation. */
export function providerToSetup(provider: Readonly<LlmProviderConfig>): ProviderSetup {
    const providerType = isSupportedProviderType(provider.provider_type) ? provider.provider_type : "openai";
    const preset = OPENAI_COMPATIBLE_PRESETS.find((candidate) =>
        candidate.providerType === providerType
        && candidate.baseUrl === trimTrailingSlashes(provider.base_url ?? "")
        && candidate.model === (provider.model ?? ""),
    );
    return normalizeProviderSetup({
        providerType,
        presetId: preset?.id ?? null,
        endpoint: provider.base_url ?? "",
        apiKey: provider.api_key ?? null,
        model: provider.model ?? "",
    });
}

function isSupportedProviderType(value: string): value is SupportedProviderType {
    return value === "openai"
        || value === "openai_responses"
        || value === "anthropic"
        || value === "ollama"
        || value === "llama_cpp";
}

function buildProviderId(providerType: SupportedProviderType, providers: ReadonlyArray<LlmProviderConfig>): string {
    const baseId = providerType === "llama_cpp"
        ? "llama-cpp"
        : providerType === "openai_responses"
            ? "openai-responses"
            : providerType;
    if (!providers.some((provider) => provider.id === baseId)) return baseId;

    let suffix = 2;
    while (providers.some((provider) => provider.id === `${baseId}-${suffix}`)) suffix += 1;
    return `${baseId}-${suffix}`;
}

/** Creates a provider with focused-setup defaults. */
export function createProvider(
    providerType: SupportedProviderType,
    providers: ReadonlyArray<LlmProviderConfig>,
): LlmProviderConfig {
    const setup = normalizeProviderSetup({
        providerType,
        presetId: null,
        endpoint: "",
        apiKey: null,
        model: "",
    });
    return {
        id: buildProviderId(providerType, providers),
        provider_type: providerType,
        enabled: true,
        supports_native_tools: true,
        api_key: setup.apiKey ?? undefined,
        api_key_env: providerType === "openai" || providerType === "openai_responses"
            ? "OPENAI_API_KEY"
            : providerType === "anthropic" ? "ANTHROPIC_API_KEY" : undefined,
        base_url: setup.endpoint,
        model: setup.model,
        extra: {},
    };
}

/** Human-readable provider type label used by settings. */
export function getProviderTypeLabel(providerType: string): string {
    switch (providerType) {
        case "openai_responses":
            return "OpenAI Responses";
        case "anthropic":
            return "Anthropic-Compatible";
        case "ollama":
            return "Ollama";
        case "llama_cpp":
            return "llama.cpp";
        default:
            return "OpenAI-Compatible";
    }
}

/** Applies focused fields without persistence or bridge I/O. */
export function applyProviderSetupToConfig(
    config: Readonly<LlmConfig>,
    setup: Readonly<ProviderSetup>,
): LlmConfig {
    const normalized = normalizeProviderSetup(setup);
    const activeIndex = config.providers.findIndex((provider) => provider.id === config.active_provider);
    const providerIndex = activeIndex >= 0 ? activeIndex : 0;
    const existing = config.providers[providerIndex];
    const fallback = existing ?? createProvider(normalized.providerType, config.providers);
    const provider: LlmProviderConfig = {
        ...fallback,
        provider_type: normalized.providerType,
        base_url: normalized.endpoint,
        model: normalized.model,
        api_key: normalized.apiKey ?? undefined,
        api_key_env: normalized.providerType === "openai" || normalized.providerType === "openai_responses"
            ? (fallback.api_key_env || "OPENAI_API_KEY")
            : normalized.providerType === "anthropic" ? (fallback.api_key_env || "ANTHROPIC_API_KEY") : undefined,
    };
    const providers = config.providers.length === 0
        ? [provider]
        : config.providers.map((candidate, index) => index === providerIndex ? provider : candidate);
    return {
        ...config,
        active_provider: provider.id,
        providers,
    };
}
