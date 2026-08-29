// pattern: Functional Core

import { describe, expect, it } from "vitest";

import type { CharacterRecord } from "@/lib/kokoro-bridge";

import {
  buildCharacterRuntimeOverrideUpdate,
  selectCharacterForEditing,
} from "./character-runtime-overrides";
import { createCharacterRuntimeOverrideService } from "./character-runtime-override-service";

function character(id: string, runtimeProfileJson = "{}"): CharacterRecord {
  return {
    id,
    name: id,
    persona: `persona-${id}`,
    user_nickname: "User",
    source_format: "manual",
    created_at: 1,
    updated_at: 1,
    runtime_profile_json: runtimeProfileJson,
  };
}

describe("per-character runtime overrides", () => {
  it("merges safe instance settings without persisting application credentials", () => {
    const updated = buildCharacterRuntimeOverrideUpdate({
      character: character("pico", JSON.stringify({ response_language: "en" })),
      overrides: {
        persona: "edited pico",
        responseLanguage: "ja",
        proactiveEnabled: false,
        live2dModel: "pico/model.model3.json",
        tts: {
          enabled: true,
          providerId: "edge-main",
          providerType: "edge_tts",
          voice: "ja-JP-NanamiNeural",
          speed: 1.1,
          pitch: 0.9,
        },
      },
      updatedAt: 42,
    });

    expect(updated.persona).toBe("edited pico");
    expect(updated.updated_at).toBe(42);
    expect(JSON.parse(updated.runtime_profile_json ?? "{}")).toEqual({
      response_language: "ja",
      proactive_enabled: false,
      live2d_model: "pico/model.model3.json",
      tts: {
        enabled: true,
        provider_id: "edge-main",
        provider_type: "edge_tts",
        voice: "ja-JP-NanamiNeural",
        speed: 1.1,
        pitch: 0.9,
      },
    });
    expect(updated.runtime_profile_json).not.toContain("api_key");
    expect(updated.runtime_profile_json).not.toContain("base_url");
  });

  it("selects the requested catalog character for editing without changing identity", () => {
    const selected = selectCharacterForEditing(
      [character("kokoro"), character("pico")],
      "pico",
    );

    expect(selected.id).toBe("pico");
  });

  it("persists the active instance before rerunning the single activation owner", async () => {
    const calls: Array<string> = [];
    const service = createCharacterRuntimeOverrideService({
      getCharacter: async () => character("pico"),
      updateCharacter: async () => { calls.push("persist"); },
      activateCharacter: async () => { calls.push("activate"); },
      now: () => 42,
    });

    await service.update("pico", { responseLanguage: "ja" });

    expect(calls).toEqual(["persist", "activate"]);
  });
});
