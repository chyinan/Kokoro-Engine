import { describe, expect, it } from "vitest";
import { normalizeTtsVoice, stripTtsProviderVoiceId, usesShortTtsVoiceId } from "./tts-voice";
import type { TtsSystemConfig, VoiceProfile } from "../../../lib/kokoro-bridge";

const realtimeConfig: TtsSystemConfig = {
    default_provider: "browser",
    cache: { enabled: true, max_entries: 500, ttl_secs: 3600 },
    queue: { max_concurrent: 3 },
    providers: [{
        id: "openai_realtime",
        provider_type: "openai_realtime",
        enabled: false,
        api_key_env: "OPENAI_API_KEY",
        model: "gpt-realtime-2.1-mini",
        default_voice: "marin",
        extra: { instructions: "Speak calmly", retained: true },
    }],
};

const realtimeVoices: Array<VoiceProfile> = ["marin", "cedar", "alloy"].map(voice => ({
    voice_id: `openai_realtime_${voice}`,
    name: voice,
    gender: "neutral",
    language: "multilingual",
    engine: "cloud",
    provider_id: "openai_realtime",
    extra_params: {},
}));

describe("OpenAI Realtime voice configuration", () => {
    it("stores and sends short preset IDs rather than internal provider-prefixed IDs", () => {
        expect(usesShortTtsVoiceId("openai_realtime", realtimeConfig)).toBe(true);
        expect(stripTtsProviderVoiceId("openai_realtime", "openai_realtime_cedar")).toBe("cedar");
        expect(normalizeTtsVoice("openai_realtime", "openai_realtime_cedar", realtimeVoices, realtimeConfig))
            .toBe("cedar");
    });

    it("uses the configured default voice after loading persisted settings", () => {
        expect(normalizeTtsVoice("openai_realtime", "", realtimeVoices, realtimeConfig)).toBe("marin");
        expect(realtimeConfig.providers[0].extra).toEqual({ instructions: "Speak calmly", retained: true });
    });
});
