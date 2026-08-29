// pattern: Functional Core

import { describe, expect, test, vi } from "vitest";
import type { LlmConfig, LlmConnectionTestResult, LlmProviderConfig } from "../../lib/kokoro-bridge";
import {
    OPENAI_COMPATIBLE_PRESETS,
    discoverProviderModels,
    normalizeProviderSetup,
    saveProviderSetup,
    testProviderSetup,
    type ProviderSetup,
} from "./provider-setup";

function createConfig(provider: LlmProviderConfig): LlmConfig {
    return {
        active_provider: provider.id,
        providers: [provider],
        presets: [],
    };
}

function createProvider(overrides: Partial<LlmProviderConfig> = {}): LlmProviderConfig {
    return {
        id: "openai",
        provider_type: "openai",
        enabled: true,
        supports_native_tools: true,
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        api_key: "sk-test",
        extra: { temperature: 0.2 },
        ...overrides,
    };
}

describe("focused provider setup", () => {
    test("exposes OpenAI-compatible presets with endpoint and model defaults", () => {
        const ids = OPENAI_COMPATIBLE_PRESETS.map((preset) => preset.id);

        expect(ids).toEqual(expect.arrayContaining(["openai", "deepseek", "openrouter"]));
        expect(OPENAI_COMPATIBLE_PRESETS.find((preset) => preset.id === "deepseek")).toMatchObject({
            providerType: "openai",
            baseUrl: "https://api.deepseek.com/v1",
            model: "deepseek-chat",
            requiresApiKey: true,
        });
    });

    test("normalizes a preset setup without changing advanced provider options", () => {
        const setup: ProviderSetup = {
            providerType: "openai",
            presetId: "deepseek",
            endpoint: "  ",
            apiKey: "  sk-deepseek  ",
            model: "  ",
        };

        expect(normalizeProviderSetup(setup)).toEqual({
            providerType: "openai",
            presetId: "deepseek",
            endpoint: "https://api.deepseek.com/v1",
            apiKey: "sk-deepseek",
            model: "deepseek-chat",
        });
    });

    test("normalizes Ollama setup without requiring or persisting a key", () => {
        const setup = normalizeProviderSetup({
            providerType: "ollama",
            presetId: null,
            endpoint: "http://localhost:11434/",
            apiKey: "secret-that-must-not-be-used",
            model: " llama3 ",
        });

        expect(setup).toEqual({
            providerType: "ollama",
            presetId: null,
            endpoint: "http://localhost:11434",
            apiKey: null,
            model: "llama3",
        });
    });

    test("discovers models through Ollama or OpenAI-compatible endpoints", async () => {
        const listOllamaModels = vi.fn().mockResolvedValue([{ name: "llama3" }, { name: "qwen2.5" }]);
        const fetchModels = vi.fn().mockResolvedValue(["gpt-4o-mini"]);

        const ollamaModels = await discoverProviderModels(
            normalizeProviderSetup({
                providerType: "ollama",
                presetId: null,
                endpoint: "http://localhost:11434/",
                apiKey: null,
                model: "llama3",
            }),
            { listOllamaModels, fetchModels },
        );
        const openAiModels = await discoverProviderModels(
            normalizeProviderSetup({
                providerType: "openai",
                presetId: "openai",
                endpoint: "",
                apiKey: "sk-test",
                model: "",
            }),
            { listOllamaModels, fetchModels },
        );

        expect(ollamaModels).toEqual(["llama3", "qwen2.5"]);
        expect(openAiModels).toEqual(["gpt-4o-mini"]);
        expect(listOllamaModels).toHaveBeenCalledWith("http://localhost:11434");
        expect(fetchModels).toHaveBeenCalledWith("https://api.openai.com/v1", "sk-test");
    });

    test("saves normalized provider setup while preserving advanced options", async () => {
        const provider = createProvider();
        const config = createConfig(provider);
        const saveLlmConfig = vi.fn().mockResolvedValue(undefined);

        const saved = await saveProviderSetup(
            config,
            {
                providerType: "openai",
                presetId: "deepseek",
                endpoint: "https://api.deepseek.com/v1/",
                apiKey: " sk-updated ",
                model: "deepseek-chat",
            },
            { saveLlmConfig },
        );

        expect(saved.active_provider).toBe("openai");
        expect(saved.providers[0]).toMatchObject({
            id: "openai",
            base_url: "https://api.deepseek.com/v1",
            api_key: "sk-updated",
            model: "deepseek-chat",
            extra: { temperature: 0.2 },
        });
        expect(saveLlmConfig).toHaveBeenCalledWith(saved);
    });

    test("returns the connection test result for the normalized setup", async () => {
        const result: LlmConnectionTestResult = {
            tested_targets: [{ role: "active", provider_id: "openai", model: "gpt-4o-mini" }],
        };
        const testLlmConnection = vi.fn().mockResolvedValue(result);

        const tested = await testProviderSetup(
            createConfig(createProvider()),
            {
                providerType: "openai",
                presetId: "openai",
                endpoint: "https://api.openai.com/v1",
                apiKey: "sk-test",
                model: "gpt-4o-mini",
            },
            { testLlmConnection },
        );

        expect(tested).toEqual(result);
        expect(testLlmConnection).toHaveBeenCalledTimes(1);
        expect(testLlmConnection.mock.calls[0]?.[0].providers[0]).toMatchObject({
            base_url: "https://api.openai.com/v1",
            api_key: "sk-test",
            model: "gpt-4o-mini",
        });
    });
});
