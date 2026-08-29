// pattern: Functional Core

import { describe, expect, it } from "vitest";

import {
  isConventionalLoopbackTtsEndpoint,
  resolveFrontendAssetSource,
  resolveFrontendRuntimeProfile,
  snapshotFrontendRuntime,
  type FrontendRuntimeState,
  type PreparedCharacterRuntime,
} from "./character-runtime-profile";
import { BUILTIN_LIVE2D_MODEL_PATH } from "@/lib/kokoro-bridge";

function frontendState(): FrontendRuntimeState {
  return {
    activeCharacterId: "old-character",
    live2dModel: "models/old.model3.json",
    background: "backgrounds/old.webp",
    tts: {
      enabled: true,
      mode: "configured_provider",
      providerId: "edge-main",
      voice: "old-voice",
      speed: 1,
      pitch: 1,
    },
    cueProfile: "cues/old.json",
  };
}

function packageAsset(path: string) {
  return {
    source: "package" as const,
    template_id: "template",
    template_version: "1.0.0",
    path,
  };
}

function preparedRuntime(
  overrides: Partial<PreparedCharacterRuntime> = {},
): PreparedCharacterRuntime {
  return {
    character_id: "new-character",
    character_name: "New Character",
    user_name: "User",
    system_prompt: "persona",
    response_language: "zh",
    proactive_enabled: true,
    current_conversation_id: "conversation-new",
    live2d_model: packageAsset("models/new.model3.json"),
    background: packageAsset("backgrounds/new.webp"),
    cue_profile: packageAsset("cues/new.json"),
    tts: {
      mode: "configured_provider",
      provider_id: "edge-new",
      provider_type: "edge_tts",
      local_preset: null,
      endpoint: null,
      voice: "new-voice",
      speed: 1.2,
      pitch: 0.9,
      requires_save_confirmation: false,
    },
    ...overrides,
  };
}

describe("character runtime profile", () => {
  it("snapshots the complete frontend runtime without sharing nested state", () => {
    const current = frontendState();

    const snapshot = snapshotFrontendRuntime(current);
    current.tts.voice = "mutated-after-snapshot";

    expect(snapshot).toEqual(frontendState());
    expect(snapshot).not.toBe(current);
    expect(snapshot.tts).not.toBe(current.tts);
  });

  it("resolves Live2D, background, TTS, and cue settings from one prepared profile", () => {
    const resolved = resolveFrontendRuntimeProfile(
      preparedRuntime(),
      frontendState(),
    );

    expect(resolved).toEqual({
      activeCharacterId: "new-character",
      live2dModel: "models/new.model3.json",
      live2dModelSource: "package",
      background: "backgrounds/new.webp",
      backgroundSource: "package",
      tts: {
        enabled: true,
        mode: "configured_provider",
        providerId: "edge-new",
        voice: "new-voice",
        speed: 1.2,
        pitch: 0.9,
      },
      cueProfile: "cues/new.json",
      cueProfileSource: "package",
    });
  });

  it("uses safe frontend fallbacks when optional assets are unavailable", () => {
    const fallback = frontendState();
    const resolved = resolveFrontendRuntimeProfile(
      preparedRuntime({
        live2d_model: null,
        background: null,
        cue_profile: null,
        tts: {
          mode: "text_only",
          provider_id: null,
          provider_type: null,
          local_preset: null,
          endpoint: null,
          voice: null,
          speed: null,
          pitch: null,
          requires_save_confirmation: false,
        },
      }),
      fallback,
    );

    expect(resolved.live2dModel).toBe(BUILTIN_LIVE2D_MODEL_PATH);
    expect(resolved.live2dModelSource).toBe("builtin");
    expect(resolved.background).toBeNull();
    expect(resolved.backgroundSource).toBe("none");
    expect(resolved.cueProfile).toBeNull();
    expect(resolved.cueProfileSource).toBe("none");
    expect(resolved.tts).toEqual({
      enabled: false,
      mode: "text_only",
      providerId: null,
      voice: null,
      speed: 1,
      pitch: 1,
    });
  });

  it("retains only explicitly marked user assets when the next character has no assets", () => {
    const fallback = {
      ...frontendState(),
      live2dModelSource: "user" as const,
      backgroundSource: "user" as const,
      cueProfileSource: "user" as const,
    };

    const resolved = resolveFrontendRuntimeProfile(
      preparedRuntime({
        live2d_model: null,
        background: null,
        cue_profile: null,
      }),
      fallback,
    );

    expect(resolved.live2dModel).toBe(fallback.live2dModel);
    expect(resolved.live2dModelSource).toBe("user");
    expect(resolved.background).toBe(fallback.background);
    expect(resolved.backgroundSource).toBe("user");
    expect(resolved.cueProfile).toBe(fallback.cueProfile);
    expect(resolved.cueProfileSource).toBe("user");
  });

  it("migrates missing or invalid source markers without discarding legacy overrides", () => {
    expect(resolveFrontendAssetSource(undefined, "legacy/model.model3.json")).toBe("user");
    expect(resolveFrontendAssetSource("corrupt", "legacy/background.webp")).toBe("user");
    expect(resolveFrontendAssetSource(undefined, null)).toBe("none");
  });

  it("allows local preset probes only at conventional loopback endpoints", () => {
    expect(isConventionalLoopbackTtsEndpoint("http://127.0.0.1:9880")).toBe(true);
    expect(isConventionalLoopbackTtsEndpoint("http://localhost:9880")).toBe(true);
    expect(isConventionalLoopbackTtsEndpoint("https://tts.example.com/api")).toBe(false);
    expect(isConventionalLoopbackTtsEndpoint("http://127.0.0.1:43210/custom")).toBe(false);
  });
});
