// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ImportOptions, ImportPreview } from "@/lib/kokoro-bridge";
import { importData, previewImport } from "@/lib/kokoro-bridge";
import { BackupTab } from "./BackupTab";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** `t` keeps interpolation visible so assertions can see which target is selected. */
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { count?: number; name?: string; memories?: number; conversations?: number }) => {
      if (options?.name !== undefined) return `${key}:${options.name}`;
      return key;
    },
    i18n: { language: "en", resolvedLanguage: "en", changeLanguage: vi.fn() },
  } as any),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/tmp/backup.kokoro"),
  save: vi.fn(async () => "/tmp/out.kokoro"),
}));

vi.mock("@/lib/legacy-character-migration", () => ({
  migrateLegacyCharactersToSqlite: vi.fn(async () => undefined),
}));

const previewWithCharacters: ImportPreview = {
  manifest: { version: "2", created_at: "2026-09-18T00:00:00Z", app_version: "0.4.0", includes_character_resources: false },
  has_database: true,
  has_configs: false,
  config_files: [],
  stats: { memories: 4, conversations: 2, messages: 9, configs: 0 },
  characters: [
    { id: "remote-kokoro", name: "Kokoro", memory_count: 3, conversation_count: 1 },
    { id: "remote-pico", name: "Pico", memory_count: 1, conversation_count: 1 },
  ],
};

vi.mock("@/lib/kokoro-bridge", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>();
  return {
    ...actual,
    previewImport: vi.fn(async () => previewWithCharacters),
    listCharacters: vi.fn(async () => [
      { id: "local-kokoro", name: "Kokoro" },
      { id: "local-alice", name: "Alice" },
    ]),
    importData: vi.fn(async () => ({
      imported_memories: 4,
      imported_conversations: 2,
      imported_configs: 0,
      imported_characters: 1,
      merged_characters: 1,
    })),
    getAutoBackupConfig: vi.fn(async () => ({ enabled: false, backup_dir: "", interval_days: 1, auto_cleanup: false, keep_days: 30 })),
    getKokoroErrorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
  };
});

let container: HTMLDivElement;
let root: Root;

function click(element: Element) {
  element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

function byText(selector: string, text: string): Element[] {
  return Array.from(document.querySelectorAll(selector)).filter(element =>
    element.textContent?.includes(text),
  );
}

/** Opens the select whose trigger currently shows `triggerText`. */
async function openSelect(triggerText: string) {
  const trigger = byText("button", triggerText)[0];
  expect(trigger, `no select trigger showing ${triggerText}`).toBeDefined();
  await act(async () => { click(trigger); });
  return trigger;
}

/** Picks an option in the currently open dropdown (it renders through a portal). */
async function chooseOption(optionText: string) {
  const option = byText("#select-portal-root div", optionText)
    .filter(element => element.children.length <= 1)
    .pop();
  expect(option, `no dropdown option ${optionText}`).toBeDefined();
  await act(async () => { click(option!); });
}

async function renderBackupTab() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(createElement(BackupTab, {}));
  });
  await act(async () => {
    click(byText("button", "settings.backup.import_button")[0]);
  });
}

beforeEach(() => {
  vi.mocked(previewImport).mockClear();
  vi.mocked(importData).mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.getElementById("select-portal-root")?.remove();
});

describe("backup character mapping UI", () => {
  it("renders the shared dropdown instead of a native select", async () => {
    await renderBackupTab();

    expect(document.querySelectorAll("select")).toHaveLength(0);
    expect(byText("button", "settings.backup.character_target_new")).toHaveLength(1);
    expect(byText("button", "settings.backup.character_target_merge:Kokoro")).toHaveLength(1);
  });

  it("preselects a merge only for the unambiguous same-name match", async () => {
    await renderBackupTab();

    const triggers = byText("button", "settings.backup.character_target");
    expect(triggers.map(trigger => trigger.textContent)).toEqual([
      "settings.backup.character_target_merge:Kokoro",
      "settings.backup.character_target_new",
    ]);
  });

  it("sends a character_merges payload for the entries the user changed", async () => {
    await renderBackupTab();

    // Route the second backup character into a local character as well.
    await openSelect("settings.backup.character_target_new");
    await chooseOption("settings.backup.character_target_merge:Alice");

    const confirm = byText("button", "settings.backup.confirm_import")[0];
    await act(async () => { click(confirm); });

    expect(importData).toHaveBeenCalledTimes(1);
    const options = vi.mocked(importData).mock.calls[0][1] as ImportOptions;
    expect(options.character_merges).toEqual([
      { imported_id: "remote-kokoro", target_id: "local-kokoro" },
      { imported_id: "remote-pico", target_id: "local-alice" },
    ]);
    expect(options.ignored_characters).toEqual([]);
  });

  it("sends an ignored_characters payload for entries set to ignore", async () => {
    await renderBackupTab();

    expect(byText("span", "settings.backup.character_ignore_warning")).toHaveLength(0);

    await openSelect("settings.backup.character_target_new");
    await chooseOption("settings.backup.character_target_ignore");

    expect(byText("span", "settings.backup.character_ignore_warning")).toHaveLength(1);

    await act(async () => {
      click(byText("button", "settings.backup.confirm_import")[0]);
    });

    const options = vi.mocked(importData).mock.calls[0][1] as ImportOptions;
    expect(options.ignored_characters).toEqual(["remote-pico"]);
    expect(options.character_merges).toEqual([
      { imported_id: "remote-kokoro", target_id: "local-kokoro" },
    ]);
  });

  it("sends no merge when every character stays a new import", async () => {
    await renderBackupTab();

    await openSelect("settings.backup.character_target_merge:Kokoro");
    await chooseOption("settings.backup.character_target_new");

    await act(async () => {
      click(byText("button", "settings.backup.confirm_import")[0]);
    });

    const options = vi.mocked(importData).mock.calls[0][1] as ImportOptions;
    expect(options.character_merges).toEqual([]);
    expect(options.ignored_characters).toEqual([]);
  });
});
