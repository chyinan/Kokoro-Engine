// pattern: Functional Core

import { describe, expect, test } from "vitest";
import type { LlmConfig, LlmProviderConfig } from "../../lib/kokoro-bridge";
import { applyProviderSetupToConfig, createProvider, type ProviderSetup } from "./provider-setup-core";

function provider(): LlmProviderConfig {
  return {
    id: "openai",
    provider_type: "openai",
    enabled: true,
    supports_native_tools: true,
    base_url: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    api_key: "sk-old",
    extra: { temperature: 0.2 },
  };
}

describe("provider setup functional core", () => {
  test("applies focused fields without performing persistence or bridge I/O", () => {
    const config: LlmConfig = { active_provider: "openai", providers: [provider()], presets: [] };
    const setup: ProviderSetup = {
      providerType: "openai",
      presetId: "deepseek",
      endpoint: "https://api.deepseek.com/v1/",
      apiKey: "sk-new",
      model: "deepseek-chat",
    };

    expect(applyProviderSetupToConfig(config, setup)).toMatchObject({
      active_provider: "openai",
      providers: [{ base_url: "https://api.deepseek.com/v1", model: "deepseek-chat", api_key: "sk-new", extra: { temperature: 0.2 } }],
    });
  });

  test("creates a Codex runtime provider without API-key fields", () => {
    const runtime = createProvider("codex_runtime", []);

    expect(runtime).toMatchObject({
      id: "codex-runtime",
      provider_type: "codex_runtime",
      enabled: true,
      supports_native_tools: true,
    });
    expect(runtime.model).toBeUndefined();
    expect(runtime).not.toHaveProperty("api_key");
    expect(runtime).not.toHaveProperty("base_url");
  });
});
