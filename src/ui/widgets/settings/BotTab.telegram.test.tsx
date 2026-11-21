// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BotConfig } from "@/lib/kokoro-bridge";
import { saveBotConfig, startTelegramBot } from "@/lib/kokoro-bridge";
import zh from "@/ui/locales/zh.json";
import BotTab from "./BotTab";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key === "telegram.whitelist.desc" ? zh.telegram.whitelist.desc : key,
  }),
}));

vi.mock("@/lib/kokoro-bridge", () => ({
  getTelegramStatus: vi.fn(async () => ({ running: true, enabled: true, has_token: true })),
  getBotStatus: vi.fn(async () => null),
  listCharacters: vi.fn(async () => []),
  saveBotConfig: vi.fn(async (config: BotConfig) => config),
  startTelegramBot: vi.fn(async () => undefined),
  stopTelegramBot: vi.fn(async () => undefined),
  startBotPlatform: vi.fn(),
  stopBotPlatform: vi.fn(),
}));

const initialConfig: BotConfig = {
  revision: 1,
  selected_platform: "telegram",
  telegram: { enabled: true, bot_token: "local-test-token", allowed_chat_ids: [], send_voice_reply: false },
  qq: { enabled: false, allow_c2c: false, allowed_user_openids: [], allowed_group_openids: [] },
  discord: { enabled: false, allowed_channel_ids: [], allow_direct_messages: true, send_voice_reply: false },
  line: { enabled: false, webhook_path: "/line", allowed_user_ids: [] },
  webhook: { enabled: false, bind_host: "127.0.0.1", port: 8787, endpoint_path: "/webhook", send_voice_reply: false },
};

let container: HTMLDivElement;
let root: Root;
const changed = vi.fn();

async function renderBot() {
  changed.mockClear();
  vi.mocked(saveBotConfig).mockClear();
  vi.mocked(startTelegramBot).mockClear();
  function Harness() {
    const [config, setConfig] = useState(initialConfig);
    return createElement(BotTab, {
      botConfig: config,
      onBotConfigChange: next => { changed(next); setConfig(next); },
      onSaveBotConfig: async () => { await saveBotConfig(config); },
    });
  }
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(createElement(Harness)));
}

async function addChat(value: string) {
  const input = container.querySelector<HTMLInputElement>('input[placeholder="telegram.whitelist.placeholder"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const add = Array.from(container.querySelectorAll("button"))
    .find(button => button.textContent === "telegram.whitelist.add")!;
  await act(async () => add.click());
  return input;
}

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("Telegram chat authorization settings", () => {
  it("explains that running with an empty list authorizes no chats", async () => {
    await renderBot();
    expect(container.textContent).toContain("telegram.status.running");
    expect(container.textContent).toContain("尚未授权任何聊天");
    expect(container.textContent).toContain("群聊 ID 授权整个群聊");
    expect(changed).not.toHaveBeenCalled();
    expect(saveBotConfig).not.toHaveBeenCalled();
  });

  it("preserves exact signed chat IDs and supports removing every grant", async () => {
    await renderBot();
    for (const value of ["123", "-1001234567890", "4503599627370495", "-4503599627370495"]) {
      await addChat(value);
    }
    expect(changed.mock.lastCall?.[0].telegram.allowed_chat_ids).toEqual([
      123, -1001234567890, 4503599627370495, -4503599627370495,
    ]);
    for (const button of Array.from(container.querySelectorAll<HTMLButtonElement>('button[aria-label="common.actions.delete"]'))) {
      await act(async () => button.click());
    }
    expect(changed.mock.lastCall?.[0].telegram.allowed_chat_ids).toEqual([]);
  });

  it("does not silently round an out-of-range authorization ID or accept a wildcard", async () => {
    await renderBot();
    for (const value of ["9007199254740993", "-9007199254740993", "1.5", "*", "all", "1e3"]) {
      const input = await addChat(value);
      expect(input.value).toBe(value);
      expect(changed).not.toHaveBeenCalled();
    }
    expect(saveBotConfig).not.toHaveBeenCalled();
    expect(startTelegramBot).not.toHaveBeenCalled();
  });
});
