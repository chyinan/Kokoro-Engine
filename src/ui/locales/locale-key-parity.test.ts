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
  chat: Readonly<{
    history: Readonly<{
      pinned: string;
      pin: string;
      unpin: string;
      [key: string]: string;
    }>;
  }>;
}>;

const locales: Readonly<Record<string, Locale>> = { en, zh, zhTw, ja, ko, ru };

describe("locale keys parity", () => {
  it("keeps provider discovery errors translated in every supported locale", () => {
    const expected = Object.keys(en.onboarding.workflow.errors).sort();

    expect(expected).toContain("model_discovery");
    for (const [locale, messages] of Object.entries(locales)) {
      expect(Object.keys(messages.onboarding.workflow.errors).sort(), locale).toEqual(expected);
    }
  });

  it("keeps conversation pin keys translated in every supported locale", () => {
    const expectedKeys = ["pinned", "pin", "unpin"];
    for (const [locale, messages] of Object.entries(locales)) {
      for (const key of expectedKeys) {
        expect(messages.chat.history[key], `${locale} missing chat.history.${key}`).toBeTruthy();
      }
    }
  });
});
