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
    image: Readonly<{
      zoom_in: string;
      zoom_out: string;
      rotate: string;
      reset: string;
      [key: string]: string;
    }>;
    input: Readonly<{
      drop_image_title: string;
      drop_image_hint: string;
      [key: string]: unknown;
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

  it("keeps chat image and dropzone keys translated in every supported locale", () => {
    const expectedImageKeys = ["zoom_in", "zoom_out", "rotate", "reset"];
    for (const [locale, messages] of Object.entries(locales)) {
      for (const key of expectedImageKeys) {
        expect(messages.chat.image[key], `${locale} missing chat.image.${key}`).toBeTruthy();
      }
      expect(messages.chat.input.drop_image_title, `${locale} missing chat.input.drop_image_title`).toBeTruthy();
      expect(messages.chat.input.drop_image_hint, `${locale} missing chat.input.drop_image_hint`).toBeTruthy();
    }
  });
});
