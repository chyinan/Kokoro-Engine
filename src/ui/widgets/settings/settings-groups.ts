// pattern: Functional Core

export const SETTINGS_TAB_IDS = [
    "api",
    "persona",
    "tts",
    "stt",
    "mods",
    "bg",
    "model",
    "imagegen",
    "memory",
    "vision",
    "mcp",
    "bot",
    "jailbreak",
    "backup",
    "pet",
    "about",
] as const;

export type SettingsTabId = typeof SETTINGS_TAB_IDS[number];
export type SettingsGroupId = "basic" | "advanced";

export interface SettingsGroup {
    id: SettingsGroupId;
    label: string;
    tabs: readonly SettingsTabId[];
}

export const SETTINGS_GROUPS: Readonly<Record<SettingsGroupId, SettingsGroup>> = {
    basic: {
        id: "basic",
        label: "settings.groups.basic",
        tabs: ["persona", "model", "tts", "stt", "bg", "imagegen"],
    },
    advanced: {
        id: "advanced",
        label: "settings.groups.advanced",
        tabs: ["vision", "memory", "mcp", "mods", "bot", "api", "jailbreak", "pet", "backup", "about"],
    },
};

export function getSettingsGroupForTab(tab: string): SettingsGroupId {
    return SETTINGS_GROUPS.basic.tabs.includes(tab as SettingsTabId) ? "basic" : "advanced";
}

export function getSettingsTabIds(): readonly SettingsTabId[] {
    return SETTINGS_TAB_IDS;
}
