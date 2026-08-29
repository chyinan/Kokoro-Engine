// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";

import {
  createCharacterActivationService,
  type CharacterActivationDependencies,
} from "./character-activation";
import type {
  CharacterActivationToken,
  CommittedCharacterRuntime,
} from "@/lib/kokoro-bridge";
import type {
  FrontendRuntimeState,
  PreparedCharacterRuntime,
} from "./character-runtime-profile";

function oldFrontendState(): FrontendRuntimeState {
  return {
    activeCharacterId: "old-character",
    live2dModel: "models/old.model3.json",
    background: "backgrounds/old.webp",
    tts: {
      enabled: true,
      mode: "configured_provider",
      providerId: "edge-old",
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

function preparedRuntime(characterId: string): PreparedCharacterRuntime {
  return {
    character_id: characterId,
    character_name: characterId,
    user_name: "User",
    system_prompt: `persona-${characterId}`,
    response_language: "zh",
    proactive_enabled: true,
    current_conversation_id: `conversation-${characterId}`,
    live2d_model: packageAsset(`models/${characterId}.model3.json`),
    background: packageAsset(`backgrounds/${characterId}.webp`),
    cue_profile: packageAsset(`cues/${characterId}.json`),
    tts: {
      mode: "configured_provider",
      provider_id: "edge-main",
      provider_type: "edge_tts",
      local_preset: null,
      endpoint: null,
      voice: `${characterId}-voice`,
      speed: 1,
      pitch: 1,
      requires_save_confirmation: false,
    },
  };
}

function token(characterId: string, revision: number): CharacterActivationToken {
  return {
    revision,
    nonce: `nonce-${revision}`,
    character_updated_at: revision,
    previous_committed: preparedRuntime("old-character"),
    resolved_runtime: preparedRuntime(characterId),
    prompt: {
      character_name: characterId,
      user_name: "User",
      persona: `persona-${characterId}`,
      example_dialogue: "",
    },
    target_conversation_id: `conversation-${characterId}`,
    greeting_action: "none" as const,
    recommendations: { vision: null, memory: null, mcp_servers: [], bot_platforms: [] },
  };
}

function committed(characterId: string, revision: number): CommittedCharacterRuntime {
  return {
    revision,
    runtime: preparedRuntime(characterId),
    target_conversation_id: `conversation-${characterId}`,
  };
}

function dependencies(
  overrides: Partial<CharacterActivationDependencies> = {},
): CharacterActivationDependencies {
  return {
    prepareCharacterActivation: vi.fn(async (characterId: string) => token(characterId, 1)),
    commitCharacterActivation: vi.fn(async (prepared) =>
      committed(prepared.resolved_runtime.character_id, prepared.revision)),
    getCommittedCharacterRuntime: vi.fn(async () => null),
    readFrontendRuntime: vi.fn(() => oldFrontendState()),
    applyFrontendRuntime: vi.fn(async () => undefined),
    restoreFrontendRuntime: vi.fn(async () => undefined),
    writeRuntimeCache: vi.fn(() => undefined),
    dispatchRuntimeChanged: vi.fn(() => undefined),
    probeLocalTtsPreset: vi.fn(async () => true),
    presentLocalTtsPresetProbe: vi.fn(() => undefined),
    confirmLocalTtsPresetSave: vi.fn(async () => false),
    saveConfirmedLocalTtsPreset: vi.fn(async () => undefined),
    ...overrides,
  };
}

describe("character activation shell", () => {
  it("prepares before snapshotting and applying the complete frontend runtime", async () => {
    const calls: Array<string> = [];
    const deps = dependencies({
      prepareCharacterActivation: vi.fn(async () => {
        calls.push("prepare");
        return token("new-character", 1);
      }),
      readFrontendRuntime: vi.fn(() => {
        calls.push("snapshot");
        return oldFrontendState();
      }),
      applyFrontendRuntime: vi.fn(async () => {
        calls.push("apply");
      }),
      commitCharacterActivation: vi.fn(async () => {
        calls.push("commit");
        return committed("new-character", 1);
      }),
    });

    await createCharacterActivationService(deps).activateCharacter("new-character");

    expect(calls).toEqual(["prepare", "snapshot", "apply", "commit"]);
    expect(deps.applyFrontendRuntime).toHaveBeenCalledWith({
      activeCharacterId: "new-character",
      live2dModel: "models/new-character.model3.json",
      background: "backgrounds/new-character.webp",
      tts: {
        enabled: true,
        mode: "configured_provider",
        providerId: "edge-main",
        voice: "new-character-voice",
        speed: 1,
        pitch: 1,
      },
      cueProfile: "cues/new-character.json",
    });
  });

  it("rolls back the complete frontend snapshot when frontend apply fails", async () => {
    const snapshot = oldFrontendState();
    const deps = dependencies({
      readFrontendRuntime: vi.fn(() => snapshot),
      applyFrontendRuntime: vi.fn(async () => {
        throw new Error("Live2D apply failed");
      }),
    });

    await expect(
      createCharacterActivationService(deps).activateCharacter("new-character"),
    ).rejects.toThrow("Live2D apply failed");

    expect(deps.restoreFrontendRuntime).toHaveBeenCalledWith(snapshot);
    expect(deps.commitCharacterActivation).not.toHaveBeenCalled();
    expect(deps.dispatchRuntimeChanged).not.toHaveBeenCalled();
  });

  it("rolls back frontend state when backend commit rejects a stale token", async () => {
    const snapshot = oldFrontendState();
    const deps = dependencies({
      readFrontendRuntime: vi.fn(() => snapshot),
      commitCharacterActivation: vi.fn(async () => {
        throw new Error("stale activation token; prepare activation again");
      }),
    });

    await expect(
      createCharacterActivationService(deps).activateCharacter("new-character"),
    ).rejects.toThrow("stale activation token");

    expect(deps.restoreFrontendRuntime).toHaveBeenCalledWith(snapshot);
    expect(deps.dispatchRuntimeChanged).not.toHaveBeenCalled();
  });

  it("recovers authoritative committed runtime before character-dependent rendering", async () => {
    const deps = dependencies({
      getCommittedCharacterRuntime: vi.fn(async () => committed("recovered", 7)),
    });

    const result = await createCharacterActivationService(deps).recoverCommittedRuntime();

    expect(result?.revision).toBe(7);
    expect(deps.applyFrontendRuntime).toHaveBeenCalledWith(
      expect.objectContaining({ activeCharacterId: "recovered" }),
    );
    expect(deps.writeRuntimeCache).toHaveBeenCalledWith(committed("recovered", 7));
    expect(deps.dispatchRuntimeChanged).toHaveBeenCalledWith(committed("recovered", 7));
  });

  it("serializes concurrent activation requests in revision order", async () => {
    let nextRevision = 0;
    const calls: Array<string> = [];
    const deps = dependencies({
      prepareCharacterActivation: vi.fn(async (characterId: string) => {
        nextRevision += 1;
        calls.push(`prepare:${characterId}:${nextRevision}`);
        return token(characterId, nextRevision);
      }),
      applyFrontendRuntime: vi.fn(async (runtime) => {
        calls.push(`apply:${runtime.activeCharacterId}`);
      }),
      commitCharacterActivation: vi.fn(async (prepared) => {
        calls.push(`commit:${prepared.resolved_runtime.character_id}:${prepared.revision}`);
        return committed(prepared.resolved_runtime.character_id, prepared.revision);
      }),
    });
    const service = createCharacterActivationService(deps);

    const first = service.activateCharacter("first");
    const second = service.activateCharacter("second");
    await Promise.all([first, second]);

    expect(calls).toEqual([
      "prepare:first:1",
      "apply:first",
      "commit:first:1",
      "prepare:second:2",
      "apply:second",
      "commit:second:2",
    ]);
  });

  it("dispatches exactly one runtime event only after successful commit", async () => {
    const deps = dependencies();

    const result = await createCharacterActivationService(deps).activateCharacter("new-character");

    expect(deps.dispatchRuntimeChanged).toHaveBeenCalledTimes(1);
    expect(deps.dispatchRuntimeChanged).toHaveBeenCalledWith(result);
  });

  it("keeps the committed result when best-effort cache writing fails", async () => {
    const deps = dependencies({
      writeRuntimeCache: vi.fn(() => {
        throw new Error("quota exceeded");
      }),
    });

    const result = await createCharacterActivationService(deps).activateCharacter("new-character");

    expect(result).toEqual({
      ...committed("new-character", 1),
      recommendations: token("new-character", 1).recommendations,
    });
    expect(deps.restoreFrontendRuntime).not.toHaveBeenCalled();
    expect(deps.dispatchRuntimeChanged).toHaveBeenCalledTimes(1);
  });

  it("shows a conventional local preset probe and saves only after explicit confirmation", async () => {
    const prepared = token("local-character", 1);
    const local = {
      ...prepared,
      resolved_runtime: {
        ...prepared.resolved_runtime,
        tts: {
          mode: "local_preset_confirmation" as const,
          provider_id: null,
          provider_type: "gpt_sovits",
          local_preset: "gpt-sovits-loopback",
          endpoint: "http://127.0.0.1:9880",
          voice: null,
          speed: 1,
          pitch: 1,
          requires_save_confirmation: true,
        },
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn()
        .mockResolvedValueOnce(local)
        .mockResolvedValueOnce(token("local-character", 2)),
      confirmLocalTtsPresetSave: vi.fn(async () => true),
    });

    await createCharacterActivationService(deps).activateCharacter("local-character");

    expect(deps.probeLocalTtsPreset).toHaveBeenCalledWith("http://127.0.0.1:9880");
    expect(deps.presentLocalTtsPresetProbe).toHaveBeenCalledWith({
      endpoint: "http://127.0.0.1:9880",
      available: true,
    });
    expect(deps.confirmLocalTtsPresetSave).toHaveBeenCalledWith({
      endpoint: "http://127.0.0.1:9880",
      available: true,
    });
    expect(deps.saveConfirmedLocalTtsPreset).toHaveBeenCalledTimes(1);
  });

  it("re-prepares after saving a local preset and commits only the server-resolved provider", async () => {
    const first = token("local-character", 1);
    const local = {
      ...first,
      resolved_runtime: {
        ...first.resolved_runtime,
        tts: {
          ...first.resolved_runtime.tts,
          mode: "local_preset_confirmation" as const,
          provider_id: null,
          provider_type: "gpt_sovits",
          local_preset: "gpt-sovits-loopback",
          endpoint: "http://127.0.0.1:9880",
          requires_save_confirmation: true,
        },
      },
    };
    const configured: CharacterActivationToken = {
      ...token("local-character", 2),
      resolved_runtime: {
        ...token("local-character", 2).resolved_runtime,
        tts: {
          ...token("local-character", 2).resolved_runtime.tts,
          provider_id: "gpt-sovits-loopback",
          provider_type: "gpt_sovits",
        },
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn()
        .mockResolvedValueOnce(local)
        .mockResolvedValueOnce(configured),
      confirmLocalTtsPresetSave: vi.fn(async () => true),
    });

    await createCharacterActivationService(deps).activateCharacter("local-character");

    expect(deps.prepareCharacterActivation).toHaveBeenCalledTimes(2);
    expect(deps.commitCharacterActivation).toHaveBeenCalledWith(configured);
    expect(deps.applyFrontendRuntime).toHaveBeenCalledWith(expect.objectContaining({
      tts: expect.objectContaining({ providerId: "gpt-sovits-loopback" }),
    }));
  });

  it.each([
    [false, true],
    [true, false],
  ])("re-prepares an explicit fallback when probe availability is %s and confirmation is %s", async (available, confirmed) => {
    const first = token("fallback-character", 1);
    const local = {
      ...first,
      resolved_runtime: {
        ...first.resolved_runtime,
        tts: {
          ...first.resolved_runtime.tts,
          mode: "local_preset_confirmation" as const,
          provider_id: null,
          provider_type: "gpt_sovits",
          local_preset: "gpt-sovits-loopback",
          endpoint: "http://127.0.0.1:9880",
          requires_save_confirmation: true,
        },
      },
    };
    const fallback: CharacterActivationToken = {
      ...token("fallback-character", 2),
      resolved_runtime: {
        ...token("fallback-character", 2).resolved_runtime,
        tts: {
          ...token("fallback-character", 2).resolved_runtime.tts,
          mode: "browser",
          provider_id: "browser",
          provider_type: "browser",
        },
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn()
        .mockResolvedValueOnce(local)
        .mockResolvedValueOnce(fallback),
      probeLocalTtsPreset: vi.fn(async () => available),
      confirmLocalTtsPresetSave: vi.fn(async () => confirmed),
    });

    await createCharacterActivationService(deps).activateCharacter("fallback-character");

    expect(deps.prepareCharacterActivation).toHaveBeenLastCalledWith(
      "fallback-character",
      { allowLocalPreset: false },
    );
    expect(deps.commitCharacterActivation).toHaveBeenCalledWith(fallback);
    expect(deps.saveConfirmedLocalTtsPreset).not.toHaveBeenCalled();
  });

  it("treats a failed local probe as unavailable and still resolves the fallback", async () => {
    const first = token("probe-failure", 1);
    const local = {
      ...first,
      resolved_runtime: {
        ...first.resolved_runtime,
        tts: {
          ...first.resolved_runtime.tts,
          mode: "local_preset_confirmation" as const,
          provider_id: null,
          provider_type: "gpt_sovits",
          local_preset: "gpt-sovits-loopback",
          endpoint: "http://127.0.0.1:9880",
          requires_save_confirmation: true,
        },
      },
    };
    const fallback: CharacterActivationToken = {
      ...token("probe-failure", 2),
      resolved_runtime: {
        ...token("probe-failure", 2).resolved_runtime,
        tts: {
          ...token("probe-failure", 2).resolved_runtime.tts,
          mode: "text_only",
          provider_id: null,
          provider_type: null,
        },
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn()
        .mockResolvedValueOnce(local)
        .mockResolvedValueOnce(fallback),
      probeLocalTtsPreset: vi.fn(async () => {
        throw new Error("connection refused");
      }),
    });

    await createCharacterActivationService(deps).activateCharacter("probe-failure");

    expect(deps.presentLocalTtsPresetProbe).toHaveBeenCalledWith({
      endpoint: "http://127.0.0.1:9880",
      available: false,
    });
    expect(deps.prepareCharacterActivation).toHaveBeenLastCalledWith(
      "probe-failure",
      { allowLocalPreset: false },
    );
    expect(deps.commitCharacterActivation).toHaveBeenCalledWith(fallback);
  });

  it("returns the sanitized prepared recommendations only after commit succeeds", async () => {
    const prepared: CharacterActivationToken = {
      ...token("recommended", 1),
      recommendations: {
        vision: true,
        memory: false,
        mcp_servers: ["calendar"],
        bot_platforms: ["telegram"],
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn(async () => prepared),
    });

    const result = await createCharacterActivationService(deps).activateCharacter("recommended");

    expect(deps.commitCharacterActivation).toHaveBeenCalledTimes(1);
    expect(result.recommendations).toEqual(prepared.recommendations);
  });

  it("never probes or saves a custom local-preset endpoint", async () => {
    const prepared = token("unsafe", 1);
    const unsafe = {
      ...prepared,
      resolved_runtime: {
        ...prepared.resolved_runtime,
        tts: {
          mode: "local_preset_confirmation" as const,
          provider_id: null,
          provider_type: "gpt_sovits",
          local_preset: "gpt-sovits-loopback",
          endpoint: "https://attacker.example/api",
          voice: null,
          speed: 1,
          pitch: 1,
          requires_save_confirmation: true,
        },
      },
    };
    const deps = dependencies({
      prepareCharacterActivation: vi.fn(async () => unsafe),
    });

    await expect(
      createCharacterActivationService(deps).activateCharacter("unsafe"),
    ).rejects.toThrow("invalid local TTS preset endpoint");

    expect(deps.probeLocalTtsPreset).not.toHaveBeenCalled();
    expect(deps.saveConfirmedLocalTtsPreset).not.toHaveBeenCalled();
  });
});
