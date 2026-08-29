// pattern: Functional Core

import { describe, expect, it } from "vitest";

import en from "./en.json";
import ja from "./ja.json";
import ko from "./ko.json";
import ru from "./ru.json";
import zh from "./zh.json";
import zhTw from "./zh-TW.json";

type Locale = Readonly<{
  onboarding: Readonly<{
    workflow: Readonly<{
      errors: Readonly<Record<string, string>>;
    }>;
  }>;
}>;

const locales: Readonly<Record<string, Locale>> = { en, zh, zhTw, ja, ko, ru };

describe("onboarding locale keys", () => {
  it("keeps provider discovery errors translated in every supported locale", () => {
    const expected = Object.keys(en.onboarding.workflow.errors).sort();

    expect(expected).toContain("model_discovery");
    for (const [locale, messages] of Object.entries(locales)) {
      expect(Object.keys(messages.onboarding.workflow.errors).sort(), locale).toEqual(expected);
    }
  });
});
