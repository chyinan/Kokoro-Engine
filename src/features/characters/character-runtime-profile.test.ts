// pattern: Functional Core

import { describe, expect, it } from "vitest";

import {
  isConventionalLoopbackTtsEndpoint,
  resolveFrontendRuntimeProfile,
  snapshotFrontendRuntime,
  type FrontendRuntimeState,
  type PreparedCharacterRuntime,
} from "./character-runtime-profile";

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
    live2d_model: "models/new.model3.json",
    background: "backgrounds/new.webp",
    cue_profile: "cues/new.json",
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
      background: "backgrounds/new.webp",
      tts: {
        enabled: true,
        mode: "configured_provider",
        providerId: "edge-new",
        voice: "new-voice",
        speed: 1.2,
        pitch: 0.9,
      },
      cueProfile: "cues/new.json",
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

    expect(resolved.live2dModel).toBe(fallback.live2dModel);
    expect(resolved.background).toBe(fallback.background);
    expect(resolved.cueProfile).toBe(fallback.cueProfile);
    expect(resolved.tts).toEqual({
      enabled: false,
      mode: "text_only",
      providerId: null,
      voice: null,
      speed: 1,
      pitch: 1,
    });
  });

  it("allows local preset probes only at conventional loopback endpoints", () => {
    expect(isConventionalLoopbackTtsEndpoint("http://127.0.0.1:9880")).toBe(true);
    expect(isConventionalLoopbackTtsEndpoint("http://localhost:9880")).toBe(true);
    expect(isConventionalLoopbackTtsEndpoint("https://tts.example.com/api")).toBe(false);
    expect(isConventionalLoopbackTtsEndpoint("http://127.0.0.1:43210/custom")).toBe(false);
  });
});
