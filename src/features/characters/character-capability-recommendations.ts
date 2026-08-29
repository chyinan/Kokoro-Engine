// pattern: Imperative Shell

import type { CharacterCapabilityRecommendations } from "@/ui/widgets/CharacterRecommendationDialog";

const ALLOWED_BOT_PLATFORMS = new Set(["telegram", "qq", "discord", "line", "webhook"]);

export type CharacterCapabilityRecommendationDependencies = {
  readonly enableVisionBackend: () => Promise<void>;
  readonly cacheVisionEnabled: (enabled: boolean) => void;
  readonly dispatchVisionChanged: () => void;
  readonly setMemoryEnabled: (enabled: boolean) => Promise<void>;
  readonly listMcpServerNames: () => ReadonlyArray<string>;
  readonly toggleMcpServer: (name: string, enabled: boolean) => Promise<void>;
  readonly refreshMcpServers: () => Promise<void>;
  readonly enableBotPlatforms: (platforms: ReadonlyArray<string>) => Promise<void>;
};

export type ApplyCharacterCapabilityRecommendationsOptions = {
  readonly recommendations: Readonly<CharacterCapabilityRecommendations>;
  readonly dependencies: Readonly<CharacterCapabilityRecommendationDependencies>;
};

/** Applies a confirmed recommendation payload through allowlisted app-owned adapters. */
export async function applyCharacterCapabilityRecommendations(
  options: Readonly<ApplyCharacterCapabilityRecommendationsOptions>,
): Promise<void> {
  const { recommendations, dependencies } = options;
  const updates: Array<Promise<unknown>> = [];
  if (recommendations.vision) {
    updates.push(dependencies.enableVisionBackend().then(() => {
      dependencies.cacheVisionEnabled(true);
      dependencies.dispatchVisionChanged();
    }));
  }
  if (recommendations.memory) {
    updates.push(dependencies.setMemoryEnabled(true));
  }
  const availableMcpServers = new Set(dependencies.listMcpServerNames());
  const mcpServers = recommendations.mcpServers.filter((name) => availableMcpServers.has(name));
  for (const server of mcpServers) {
    updates.push(dependencies.toggleMcpServer(server, true));
  }
  const botPlatforms = recommendations.botPlatforms.filter((platform) => (
    ALLOWED_BOT_PLATFORMS.has(platform)
  ));
  if (botPlatforms.length > 0) {
    updates.push(dependencies.enableBotPlatforms(botPlatforms));
  }
  await Promise.all(updates);
  if (mcpServers.length > 0) {
    await dependencies.refreshMcpServers();
  }
}
