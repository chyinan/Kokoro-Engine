// pattern: Imperative Shell

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test, vi } from "vitest";

import type { ProviderSetup } from "../../../features/onboarding/provider-setup";
import { ProviderSetupStep } from "./ProviderSetupStep";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

const setup: ProviderSetup = {
  providerType: "ollama",
  presetId: null,
  endpoint: "http://localhost:11434",
  apiKey: null,
  model: "llama3",
};

describe("ProviderSetupStep localization", () => {
  test("uses keys under the onboarding workflow provider locale path", () => {
    const html = renderToStaticMarkup(createElement(ProviderSetupStep, {
      setup,
      onChange: vi.fn(),
      onTestConnection: vi.fn(),
      onSave: vi.fn(),
    }));

    expect(html).toContain("onboarding.workflow.provider.provider_type");
    expect(html).toContain("onboarding.workflow.provider.ollama");
    expect(html).toContain("onboarding.workflow.provider.endpoint");
    expect(html).not.toContain("settings.api.provider_types.ollama");
  });
});
