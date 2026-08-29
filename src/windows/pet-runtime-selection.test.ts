// pattern: Functional Core

import { describe, expect, it } from "vitest";

import type { CommittedCharacterRuntime } from "../lib/kokoro-bridge";
import { selectPetModelPath } from "./pet-runtime-selection";

const fallback = "__builtin__/hiyori/hiyori_pro_t11.model3.json";

function committed(reference: CommittedCharacterRuntime["runtime"]["live2d_model"]): CommittedCharacterRuntime {
  return {
    revision: 1,
    runtime: {
      character_id: "kokoro",
      character_name: "Kokoro",
      user_name: "User",
      system_prompt: "persona",
      response_language: "English",
      proactive_enabled: true,
      current_conversation_id: "conversation-kokoro",
      live2d_model: reference,
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
    },
    target_conversation_id: "conversation-kokoro",
  };
}

describe("pet committed runtime model selection", () => {
  it("uses a committed package model after window recreation", () => {
    expect(selectPetModelPath(committed({
      source: "package",
      template_id: "kokoro",
      template_version: "1.0.0",
      path: "C:/app/characters/kokoro/model.model3.json",
    }), fallback)).toBe("C:/app/characters/kokoro/model.model3.json");
  });

  it("keeps the safe built-in model while committed runtime is unavailable", () => {
    expect(selectPetModelPath(null, fallback)).toBe(fallback);
    expect(selectPetModelPath(committed(null), fallback)).toBe(fallback);
  });
});
