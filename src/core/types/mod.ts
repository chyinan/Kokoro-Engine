// ── TTS Types ──────────────────────────────────────────

export interface TtsConfig {
    provider_id?: string;
    api_key?: string;
    endpoint?: string;
    model?: string;
    voice?: string;
    speed?: number;
    pitch?: number;
    emotion?: string;
}

export interface ProviderCapabilities {
    supports_streaming: boolean;
    supports_emotions: boolean;
    supports_speed: boolean;
    supports_pitch: boolean;
    supports_cloning: boolean;
    supports_ssml: boolean;
}

export interface ProviderStatus {
    id: string;
    available: boolean;
    capabilities: ProviderCapabilities;
}

export type Gender = "male" | "female" | "neutral";
export type TtsEngine = "vits" | "rvc" | "cloud" | "native";

export interface VoiceProfile {
    voice_id: string;
    name: string;
    gender: Gender;
    language: string;
    engine: TtsEngine;
    provider_id: string;
    extra_params: Record<string, string>;
}

// ── TTS System Config (mirrors Rust TtsSystemConfig) ───

export interface ProviderConfigData {
    id: string;
    provider_type: string;
    enabled: boolean;
    api_key?: string | null;
    api_key_env?: string | null;
    base_url?: string | null;
    endpoint?: string | null;
    model?: string | null;
    default_voice?: string | null;
    model_path?: string | null;
    extra: Record<string, unknown>;
}

export interface CacheConfig {
    enabled: boolean;
    max_entries: number;
    ttl_secs: number;
}

export interface QueueConfig {
    max_concurrent: number;
}

export interface TtsSystemConfig {
    default_provider?: string | null;
    cache: CacheConfig;
    queue: QueueConfig;
    providers: ProviderConfigData[];
}

// ── Character Types ────────────────────────────────────

export interface CharacterConfig {
    model_path: string;
    system_prompt: string;
    tts: TtsConfig;
}

// ── Mod Types ──────────────────────────────────────────

export interface ModManifest {
    id: string;
    name: string;
    version: string;
    description: string;
    engine_version?: string;
    layout?: string;
    theme?: string;
    components?: Record<string, string>;
    scripts?: string[];
    permissions?: string[];
    entry?: string;
    ui_entry?: string;
}

// ── Mod Theme (loaded from theme.json) ─────────────────

export interface ModThemeAssets {
    fonts?: string[];
    background?: string;
    noise_texture?: string;
    [key: string]: string | string[] | undefined;
}

export interface ModThemeJson {
    id?: string;
    name?: string;
    variables: Record<string, string>;
    assets?: ModThemeAssets;
    animations?: Record<string, {
        initial?: Record<string, number | string>;
        animate?: Record<string, number | string>;
        exit?: Record<string, number | string>;
        transition?: Record<string, number | string>;
    }>;
}
