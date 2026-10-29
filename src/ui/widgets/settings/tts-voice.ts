import type { VoiceProfile, TtsSystemConfig } from "../../../lib/kokoro-bridge";

function getProviderConfig(providerId: string, ttsConfig?: TtsSystemConfig | null) {
    return ttsConfig?.providers.find(provider => provider.id === providerId);
}

function isReferenceCloneProvider(providerId: string, ttsConfig?: TtsSystemConfig | null): boolean {
    const providerType = getProviderConfig(providerId, ttsConfig)?.provider_type;
    return providerType === "gpt_sovits" || providerType === "omnivoice";
}

export function stripTtsProviderVoiceId(providerId: string, voiceId: string): string {
    return voiceId.startsWith(`${providerId}_`) ? voiceId.slice(providerId.length + 1) : voiceId;
}

export function usesShortTtsVoiceId(providerId: string, ttsConfig?: TtsSystemConfig | null): boolean {
    const providerType = getProviderConfig(providerId, ttsConfig)?.provider_type;
    return providerType === "edge_tts"
        || providerType === "openai"
        || providerType === "openai_realtime";
}

function getDefaultTtsVoice(
    providerId: string,
    voices: ReadonlyArray<VoiceProfile>,
    ttsConfig?: TtsSystemConfig | null,
): string {
    if (providerId === "browser") return "";

    const provider = getProviderConfig(providerId, ttsConfig);
    if (provider?.provider_type === "openai" || provider?.provider_type === "openai_realtime") {
        return provider.default_voice || (provider.provider_type === "openai_realtime" ? "marin" : "alloy");
    }
    if (!provider && providerId === "openai") return "alloy";

    return voices.find(voice => voice.provider_id === providerId)?.voice_id ?? "";
}

export function normalizeTtsVoice(
    providerId: string,
    voice: string,
    voices: ReadonlyArray<VoiceProfile>,
    ttsConfig?: TtsSystemConfig | null,
): string {
    if (isReferenceCloneProvider(providerId, ttsConfig)) return "";

    const provider = getProviderConfig(providerId, ttsConfig);
    if (!voice) {
        if (provider?.provider_type === "openai" || provider?.provider_type === "openai_realtime") {
            return provider.default_voice || (provider.provider_type === "openai_realtime" ? "marin" : "alloy");
        }

        if (usesShortTtsVoiceId(providerId, ttsConfig)) {
            if (provider?.default_voice) return provider.default_voice;
            const preferredChineseVoice = voices.find(
                item => item.provider_id === providerId && item.voice_id.includes("zh-CN-XiaoyiNeural"),
            );
            if (preferredChineseVoice) return stripTtsProviderVoiceId(providerId, preferredChineseVoice.voice_id);
            const providerVoice = voices.find(item => item.provider_id === providerId);
            return providerVoice ? stripTtsProviderVoiceId(providerId, providerVoice.voice_id) : "";
        }

        return getDefaultTtsVoice(providerId, voices, ttsConfig);
    }

    if (providerId === "browser") return voice === "browser_default" ? voice : "";
    if (provider?.provider_type === "openai" || provider?.provider_type === "openai_realtime") {
        return stripTtsProviderVoiceId(providerId, voice);
    }

    const matchesProvider = voices.some(item => {
        if (item.provider_id !== providerId) return false;
        return usesShortTtsVoiceId(providerId, ttsConfig)
            ? stripTtsProviderVoiceId(providerId, item.voice_id) === voice
            : item.voice_id === voice;
    });
    if (matchesProvider) return voice;

    if (usesShortTtsVoiceId(providerId, ttsConfig)) {
        if (provider?.default_voice) return provider.default_voice;
        const preferredChineseVoice = voices.find(
            item => item.provider_id === providerId && item.voice_id.includes("zh-CN-XiaoyiNeural"),
        );
        if (preferredChineseVoice) return stripTtsProviderVoiceId(providerId, preferredChineseVoice.voice_id);
    }

    return getDefaultTtsVoice(providerId, voices, ttsConfig);
}
