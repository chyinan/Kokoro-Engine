import { describe, expect, it } from "vitest";
import {
  isBackgroundConfigDirty,
  isBotConfigDirty,
  isCharacterEditDirty,
  isImageGenConfigDirty,
  isLlmConfigDirty,
  isRuntimeDirty,
  isSttConfigDirty,
  isTtsConfigDirty,
  isTtsParamsDirty,
  isUserProfileDirty,
  isVisionConfigDirty,
  type TtsParamSnapshot,
  type UserProfile,
} from "./settings-dirty-check";
import type {
  BotConfig,
  CharacterRecord,
  ImageGenSystemConfig,
  LlmConfig,
  SttConfig,
  TtsSystemConfig,
  VisionConfig,
} from "../../lib/kokoro-bridge";
import type { BackgroundConfig } from "../hooks/useBackgroundSlideshow";

describe("Settings Dirty Checking", () => {
  describe("isCharacterEditDirty", () => {
    const base: CharacterRecord = {
      id: "char-1",
      name: "Kokoro",
      user_nickname: "Master",
      persona: "Helpful companion",
      source_format: "manual",
      created_at: 1000,
      updated_at: 1000,
    };

    it("returns false when character fields are identical", () => {
      expect(isCharacterEditDirty(base, { ...base })).toBe(false);
      expect(isCharacterEditDirty(null, null)).toBe(false);
    });

    it("returns true when any core field changes", () => {
      expect(isCharacterEditDirty(base, { ...base, name: "New Name" })).toBe(true);
      expect(isCharacterEditDirty(base, { ...base, user_nickname: "Friend" })).toBe(true);
      expect(isCharacterEditDirty(base, { ...base, persona: "Different personality" })).toBe(true);
      expect(isCharacterEditDirty(base, null)).toBe(false);
      expect(isCharacterEditDirty(null, base)).toBe(true);
    });
  });

  describe("isUserProfileDirty", () => {
    const profile: UserProfile = {
      name: "User",
      persona: "Student",
    };

    it("returns false when profile is unchanged", () => {
      expect(isUserProfileDirty(profile, { ...profile })).toBe(false);
      expect(isUserProfileDirty(null, null)).toBe(false);
    });

    it("returns true when profile name or persona changes", () => {
      expect(isUserProfileDirty(profile, { ...profile, name: "New Name" })).toBe(true);
      expect(isUserProfileDirty(profile, { ...profile, persona: "Engineer" })).toBe(true);
      expect(isUserProfileDirty(profile, null)).toBe(false);
      expect(isUserProfileDirty(null, profile)).toBe(true);
    });
  });

  describe("isTtsParamsDirty", () => {
    const base: TtsParamSnapshot = {
      enabled: true,
      providerId: "edge_tts",
      voice: "zh-CN-XiaoyiNeural",
      speed: "1.0",
      pitch: "1.0",
    };

    it("detects no changes when parameters match", () => {
      expect(isTtsParamsDirty(base, { ...base })).toBe(false);
      expect(isTtsParamsDirty(null, null)).toBe(false);
    });

    it("detects parameter changes", () => {
      expect(isTtsParamsDirty(base, { ...base, enabled: false })).toBe(true);
      expect(isTtsParamsDirty(base, { ...base, providerId: "openai" })).toBe(true);
      expect(isTtsParamsDirty(base, { ...base, voice: "alloy" })).toBe(true);
      expect(isTtsParamsDirty(base, { ...base, speed: "1.2" })).toBe(true);
      expect(isTtsParamsDirty(base, { ...base, pitch: "0.9" })).toBe(true);
    });
  });

  describe("isTtsConfigDirty", () => {
    const baseConfig: TtsSystemConfig = {
      cache: { enabled: true, max_entries: 100, ttl_secs: 3600 },
      queue: { max_concurrent: 2 },
      providers: [
        {
          id: "openai",
          provider_type: "openai",
          enabled: true,
          default_voice: "alloy",
          base_url: "https://api.openai.com/v1",
          api_key: "key",
          extra: {},
        },
      ],
    };

    it("returns false for identical config", () => {
      expect(isTtsConfigDirty(baseConfig, { ...baseConfig })).toBe(false);
      expect(isTtsConfigDirty(null, null)).toBe(false);
    });

    it("returns true when providers or settings change", () => {
      const changed: TtsSystemConfig = {
        ...baseConfig,
        providers: [
          {
            ...baseConfig.providers[0],
            default_voice: "echo",
          },
        ],
      };
      expect(isTtsConfigDirty(baseConfig, changed)).toBe(true);
      expect(isTtsConfigDirty(baseConfig, null)).toBe(true);
    });
  });

  describe("isVisionConfigDirty", () => {
    const baseVision: VisionConfig = {
      auto_vision_enabled: false,
      capture_interval_secs: 10,
      change_threshold: 0.1,
      proactive_vision_enabled: false,
      vision_context_history_mode: "latest",
      vlm_api_key: "",
      vlm_base_url: "",
      vlm_enabled: true,
      vlm_model: "llava",
      vlm_provider: "ollama",
      camera_enabled: false,
      camera_device_id: null,
    };

    it("returns false for unchanged config", () => {
      expect(isVisionConfigDirty(baseVision, { ...baseVision })).toBe(false);
      expect(isVisionConfigDirty(null, null)).toBe(false);
    });

    it("returns true when interval or model changes", () => {
      expect(isVisionConfigDirty(baseVision, { ...baseVision, capture_interval_secs: 15 })).toBe(true);
      expect(isVisionConfigDirty(baseVision, { ...baseVision, vlm_provider: "openai" })).toBe(true);
    });
  });

  describe("isSttConfigDirty", () => {
    const baseStt: SttConfig = {
      active_provider: "browser",
      auto_send: true,
      continuous_listening: false,
      language: "zh-CN",
      wake_word_enabled: false,
      wake_word: "hey",
      providers: [],
    };

    it("returns false when STT config and voiceInterrupt match", () => {
      expect(isSttConfigDirty(baseStt, { ...baseStt }, false, false)).toBe(false);
      expect(isSttConfigDirty(null, null, true, true)).toBe(false);
    });

    it("returns true when voiceInterrupt changes", () => {
      expect(isSttConfigDirty(baseStt, { ...baseStt }, false, true)).toBe(true);
    });

    it("returns true when STT config changes", () => {
      expect(isSttConfigDirty(baseStt, { ...baseStt, auto_send: false }, false, false)).toBe(true);
      expect(isSttConfigDirty(baseStt, null, false, false)).toBe(true);
    });
  });

  describe("isImageGenConfigDirty", () => {
    const baseImg: ImageGenSystemConfig = {
      enabled: true,
      default_provider: "dall-e-3",
      providers: [
        {
          id: "dall-e-3",
          provider_type: "openai",
          enabled: true,
        },
      ],
    };

    it("returns false when unchanged", () => {
      expect(isImageGenConfigDirty(baseImg, { ...baseImg })).toBe(false);
    });

    it("returns true when modified", () => {
      expect(isImageGenConfigDirty(baseImg, { ...baseImg, enabled: false })).toBe(true);
    });
  });

  describe("isBotConfigDirty", () => {
    const baseBot: BotConfig = {
      revision: 1,
      selected_platform: "telegram",
      telegram: {
        enabled: false,
        bot_token: "",
        allowed_chat_ids: [],
        send_voice_reply: false,
      },
      qq: {
        enabled: false,
        app_id: "123",
        allow_c2c: false,
        allowed_user_openids: [],
        allowed_group_openids: [],
      },
      discord: {
        enabled: false,
        allowed_channel_ids: [],
        allow_direct_messages: true,
        send_voice_reply: false,
      },
      line: {
        enabled: false,
        webhook_path: "/webhook/line",
        allowed_user_ids: [],
      },
      webhook: {
        enabled: false,
        bind_host: "127.0.0.1",
        port: 8080,
        endpoint_path: "/webhook",
        send_voice_reply: false,
      },
    };

    it("returns false when unchanged", () => {
      expect(isBotConfigDirty(baseBot, { ...baseBot })).toBe(false);
    });

    it("returns true when modified", () => {
      expect(isBotConfigDirty(baseBot, { ...baseBot, revision: 2 })).toBe(true);
    });
  });

  describe("isLlmConfigDirty", () => {
    const baseLlm: LlmConfig = {
      providers: [],
      active_provider: "openai",
    };

    it("returns false when unchanged", () => {
      expect(isLlmConfigDirty(baseLlm, { ...baseLlm })).toBe(false);
    });

    it("returns true when modified", () => {
      expect(isLlmConfigDirty(baseLlm, { ...baseLlm, active_provider: "anthropic" })).toBe(true);
    });
  });

  describe("isBackgroundConfigDirty", () => {
    const baseBg: BackgroundConfig = {
      enabled: true,
      blur: false,
      blurAmount: 8,
      interval: 30,
      rotation: "sequential",
      mode: "static",
    };

    it("returns false when unchanged", () => {
      expect(isBackgroundConfigDirty(baseBg, { ...baseBg })).toBe(false);
    });

    it("returns true when modified", () => {
      expect(isBackgroundConfigDirty(baseBg, { ...baseBg, blur: true })).toBe(true);
    });
  });

  describe("isRuntimeDirty", () => {
    it("returns false when no runtime fields changed", () => {
      expect(
        isRuntimeDirty({
          personaDirty: false,
          ttsDirty: false,
          modelDirty: false,
          responseLangDirty: false,
        }),
      ).toBe(false);
    });

    it("returns true when any runtime field changed", () => {
      expect(
        isRuntimeDirty({
          personaDirty: true,
          ttsDirty: false,
          modelDirty: false,
          responseLangDirty: false,
        }),
      ).toBe(true);

      expect(
        isRuntimeDirty({
          personaDirty: false,
          ttsDirty: true,
          modelDirty: false,
          responseLangDirty: false,
        }),
      ).toBe(true);

      expect(
        isRuntimeDirty({
          personaDirty: false,
          ttsDirty: false,
          modelDirty: true,
          responseLangDirty: false,
        }),
      ).toBe(true);

      expect(
        isRuntimeDirty({
          personaDirty: false,
          ttsDirty: false,
          modelDirty: false,
          responseLangDirty: true,
        }),
      ).toBe(true);
    });
  });
});
