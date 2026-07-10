import { describe, expect, it } from "vitest";

import {
    createProvider,
    getDefaultModel,
    getProviderTypeLabel,
    type SupportedProviderType,
} from "./ApiTab";

describe("ApiTab provider types", () => {
    it("creates an independent OpenAI Responses provider", () => {
        const provider = createProvider("openai_responses", []);

        expect(provider).toMatchObject({
            id: "openai-responses",
            provider_type: "openai_responses",
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o",
            api_key_env: "OPENAI_API_KEY",
        });
        expect(getProviderTypeLabel("openai_responses")).toBe("OpenAI Responses");
    });

    it("keeps the existing OpenAI-compatible defaults", () => {
        const providerType: SupportedProviderType = "openai";

        expect(getDefaultModel(providerType)).toBe("gpt-4");
        expect(createProvider(providerType, []).id).toBe("openai");
        expect(getProviderTypeLabel(providerType)).toBe("OpenAI-Compatible");
    });
});
