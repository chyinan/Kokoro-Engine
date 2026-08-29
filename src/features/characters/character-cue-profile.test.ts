// pattern: Functional Core

import { describe, expect, it } from "vitest";

import { parseCharacterCueProfile } from "./character-cue-profile";

describe("character cue profile", () => {
  it("maps validated package cue data into Live2D cue bindings", () => {
    expect(parseCharacterCueProfile({
      schema_version: 1,
      profile: "pico",
      default: "bright",
      cues: {
        bright: { expression: "smile", intensity: 0.8 },
        bounce: { motion_group: "TapBody" },
      },
    })).toEqual({
      cueMap: {
        bright: { expression: "smile" },
        bounce: { motion_group: "TapBody" },
      },
      semanticCueMap: { default: "bright" },
    });
  });
});
