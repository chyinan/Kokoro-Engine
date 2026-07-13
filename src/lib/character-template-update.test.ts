// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
  selectCharacterTemplateConflictValues,
  type CharacterTemplateConflict,
} from "./character-template-update";

const CONFLICTS: ReadonlyArray<CharacterTemplateConflict> = [
  {
    field: "greeting",
    old_value: "Hello",
    user_value: "Welcome back",
    new_value: "Good morning",
  },
  {
    field: "runtime.tts.voice",
    old_value: "voice-a",
    user_value: "voice-b",
    new_value: "voice-c",
  },
];

describe("selectCharacterTemplateConflictValues", () => {
  it("keeps current user values unless a template value is explicitly selected", () => {
    expect(selectCharacterTemplateConflictValues(CONFLICTS, {})).toEqual([
      { field: "greeting", value: "Welcome back" },
      { field: "runtime.tts.voice", value: "voice-b" },
    ]);
  });

  it("accepts only the selected new template values", () => {
    expect(
      selectCharacterTemplateConflictValues(CONFLICTS, {
        greeting: "accept_template",
      }),
    ).toEqual([
      { field: "greeting", value: "Good morning" },
      { field: "runtime.tts.voice", value: "voice-b" },
    ]);
  });

  it("does not mutate the conflict payload", () => {
    const before = structuredClone(CONFLICTS);

    selectCharacterTemplateConflictValues(CONFLICTS, {
      greeting: "accept_template",
      "runtime.tts.voice": "keep_user",
    });

    expect(CONFLICTS).toEqual(before);
  });
});
