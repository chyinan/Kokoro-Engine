// pattern: Functional Core

import { describe, expect, it } from "vitest";

import {
  SETTINGS_GROUPS,
  getSettingsGroupForTab,
  getSettingsTabIds,
} from "./settings-groups";

describe("settings groups", () => {
  it("keeps primary controls in the basic group", () => {
    expect(getSettingsGroupForTab("persona")).toBe("basic");
    expect(getSettingsGroupForTab("model")).toBe("basic");
    expect(getSettingsGroupForTab("tts")).toBe("basic");
    expect(getSettingsGroupForTab("stt")).toBe("basic");
    expect(getSettingsGroupForTab("bg")).toBe("basic");
  });

  it("keeps capability and integration tabs in advanced without removing tabs", () => {
    expect(getSettingsGroupForTab("vision")).toBe("advanced");
    expect(getSettingsGroupForTab("memory")).toBe("advanced");
    expect(getSettingsGroupForTab("mcp")).toBe("advanced");
    expect(getSettingsGroupForTab("bot")).toBe("advanced");

    const allTabs = getSettingsTabIds();
    const groupedTabs = Object.values(SETTINGS_GROUPS).flatMap(group => group.tabs);
    expect(groupedTabs).toEqual(expect.arrayContaining([...allTabs]));
    expect(new Set(groupedTabs)).toEqual(new Set(allTabs));
  });
});
