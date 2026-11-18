import type { commands } from "@/lib/generated/ipc";

export type {
    AudioFormat as TtsAudioFormat,
    CacheConfig,
    Gender,
    ModThemeAssets,
    ProviderCapabilities,
    ProviderStatus,
    QueueConfig,
    TtsEngine,
    VoiceProfile,
} from "@/lib/generated/ipc";

export type TtsConfig = Parameters<typeof commands.synthesize>[1];
type WireTtsSystemConfig = Parameters<typeof commands.saveTtsConfig>[0];
type WireTtsProviderConfig = NonNullable<WireTtsSystemConfig["providers"]>[number];
export type ProviderConfigData = WireTtsProviderConfig & Required<Pick<WireTtsProviderConfig, "enabled" | "extra">>;
export type TtsSystemConfig = Omit<WireTtsSystemConfig, "cache" | "queue" | "providers"> & {
    cache: Required<NonNullable<WireTtsSystemConfig["cache"]>>;
    queue: Required<NonNullable<WireTtsSystemConfig["queue"]>>;
    providers: ProviderConfigData[];
};
export type ModManifest = Awaited<ReturnType<typeof commands.listMods>>[number];
export type ModThemeJson = NonNullable<Awaited<ReturnType<typeof commands.getModTheme>>>;
