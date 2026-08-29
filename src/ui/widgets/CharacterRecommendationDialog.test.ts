// pattern: Imperative Shell

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import i18n from "../i18n";

import {
  CharacterRecommendationDialog,
  applyRecommendationDecision,
  getRecommendationItems,
  getRecommendationSessionKey,
  type CharacterCapabilityRecommendations,
} from "./CharacterRecommendationDialog";

function recommendations(): CharacterCapabilityRecommendations {
  return {
    vision: true,
    memory: true,
    mcpServers: ["calendar", "notes"],
    botPlatforms: ["telegram"],
  };
}

describe("character capability recommendation consent", () => {
  it("changes session identity when the dialog reopens for a character", () => {
    const suggested = recommendations();

    expect(getRecommendationSessionKey(false, "Kokoro", suggested)).not.toBe(
      getRecommendationSessionKey(true, "Kokoro", suggested),
    );
    expect(getRecommendationSessionKey(true, "Kokoro", suggested)).not.toBe(
      getRecommendationSessionKey(true, "Pico", suggested),
    );
  });

  it("renders vision, memory, MCP, and bot recommendations as suggestions", async () => {
    await i18n.changeLanguage("en");
    expect(getRecommendationItems(recommendations())).toEqual([
      { type: "vision", value: "vision" },
      { type: "memory", value: "memory" },
      { type: "mcp", value: "calendar" },
      { type: "mcp", value: "notes" },
      { type: "bot", value: "telegram" },
    ]);
    const html = renderToStaticMarkup(createElement(CharacterRecommendationDialog, {
      open: true,
      characterName: "Kokoro",
      recommendations: recommendations(),
      onConfirm: async () => undefined,
      onDismiss: () => undefined,
    }));

    expect(html).toContain("Vision");
    expect(html).toContain("Memory");
    expect(html).toContain("MCP: calendar");
    expect(html).toContain("Bot: telegram");
  });

  it("does not change capability settings when recommendations are dismissed", async () => {
    const enableCapabilities = vi.fn(async () => undefined);

    await applyRecommendationDecision({
      decision: "dismiss",
      recommendations: recommendations(),
      enableCapabilities,
    });

    expect(enableCapabilities).not.toHaveBeenCalled();
  });

  it("changes capability settings only after explicit confirmation", async () => {
    const enableCapabilities = vi.fn(async () => undefined);
    const suggested = recommendations();

    await applyRecommendationDecision({
      decision: "confirm",
      recommendations: suggested,
      enableCapabilities,
    });

    expect(enableCapabilities).toHaveBeenCalledTimes(1);
    expect(enableCapabilities).toHaveBeenCalledWith(suggested);
  });
});
