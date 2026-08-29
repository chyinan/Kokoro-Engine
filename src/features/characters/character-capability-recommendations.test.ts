// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";

import { applyCharacterCapabilityRecommendations } from "./character-capability-recommendations";

describe("character capability recommendation application", () => {
  it("allowlists bot platforms and synchronizes vision state only after confirmation", async () => {
    const dependencies = {
      enableVisionBackend: vi.fn(async () => undefined),
      cacheVisionEnabled: vi.fn(),
      dispatchVisionChanged: vi.fn(),
      setMemoryEnabled: vi.fn(async () => undefined),
      listMcpServerNames: vi.fn(() => ["calendar"]),
      toggleMcpServer: vi.fn(async () => undefined),
      refreshMcpServers: vi.fn(async () => undefined),
      enableBotPlatforms: vi.fn(async () => undefined),
    };

    await applyCharacterCapabilityRecommendations({
      recommendations: {
        vision: true,
        memory: false,
        mcpServers: ["calendar", "uninstalled"],
        botPlatforms: ["telegram", "__proto__"],
      },
      dependencies,
    });

    expect(dependencies.cacheVisionEnabled).toHaveBeenCalledWith(true);
    expect(dependencies.dispatchVisionChanged).toHaveBeenCalledTimes(1);
    expect(dependencies.toggleMcpServer).toHaveBeenCalledTimes(1);
    expect(dependencies.toggleMcpServer).toHaveBeenCalledWith("calendar", true);
    expect(dependencies.enableBotPlatforms).toHaveBeenCalledWith(["telegram"]);
  });
});
