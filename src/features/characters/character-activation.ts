// pattern: Imperative Shell

import type {
  CharacterActivationToken,
  CommittedCharacterRuntime,
} from "@/lib/kokoro-bridge";

import {
  isConventionalLoopbackTtsEndpoint,
  resolveFrontendRuntimeProfile,
  snapshotFrontendRuntime,
  type FrontendRuntimeState,
} from "./character-runtime-profile";

export type LocalTtsPresetProbe = {
  readonly endpoint: string;
  readonly available: boolean;
};

export type CharacterActivationDependencies = {
  readonly prepareCharacterActivation: (
    characterId: string,
  ) => Promise<CharacterActivationToken>;
  readonly commitCharacterActivation: (
    token: Readonly<CharacterActivationToken>,
  ) => Promise<CommittedCharacterRuntime>;
  readonly getCommittedCharacterRuntime: () => Promise<CommittedCharacterRuntime | null>;
  readonly readFrontendRuntime: () => FrontendRuntimeState;
  readonly applyFrontendRuntime: (
    runtime: Readonly<FrontendRuntimeState>,
  ) => Promise<void>;
  readonly restoreFrontendRuntime: (
    snapshot: Readonly<FrontendRuntimeState>,
  ) => Promise<void>;
  readonly writeRuntimeCache: (runtime: Readonly<CommittedCharacterRuntime>) => void;
  readonly dispatchRuntimeChanged: (runtime: Readonly<CommittedCharacterRuntime>) => void;
  readonly probeLocalTtsPreset: (endpoint: string) => Promise<boolean>;
  readonly presentLocalTtsPresetProbe: (probe: Readonly<LocalTtsPresetProbe>) => void;
  readonly confirmLocalTtsPresetSave: (
    probe: Readonly<LocalTtsPresetProbe>,
  ) => Promise<boolean>;
  readonly saveConfirmedLocalTtsPreset: (
    token: Readonly<CharacterActivationToken>,
  ) => Promise<void>;
};

export type CharacterActivationService = {
  readonly activateCharacter: (characterId: string) => Promise<CommittedCharacterRuntime>;
  readonly recoverCommittedRuntime: () => Promise<CommittedCharacterRuntime | null>;
};

function writeCacheBestEffort(
  dependencies: Readonly<CharacterActivationDependencies>,
  committed: Readonly<CommittedCharacterRuntime>,
): void {
  try {
    dependencies.writeRuntimeCache(committed);
  } catch (error) {
    console.warn("[CharacterActivation] failed to write runtime cache", error);
  }
}

async function handleLocalPreset(
  dependencies: Readonly<CharacterActivationDependencies>,
  token: Readonly<CharacterActivationToken>,
): Promise<void> {
  const tts = token.resolved_runtime.tts;
  if (tts.mode !== "local_preset_confirmation") return;

  const endpoint = tts.endpoint;
  if (endpoint === null || !isConventionalLoopbackTtsEndpoint(endpoint)) {
    throw new Error("invalid local TTS preset endpoint");
  }

  const probe = {
    endpoint,
    available: await dependencies.probeLocalTtsPreset(endpoint),
  };
  dependencies.presentLocalTtsPresetProbe(probe);
  const isConfirmed = await dependencies.confirmLocalTtsPresetSave(probe);
  if (isConfirmed) {
    await dependencies.saveConfirmedLocalTtsPreset(token);
  }
}

/**
 * Creates the sole frontend character-activation transaction owner. Calls are
 * serialized so prepared revisions reach the backend in the same order.
 */
export function createCharacterActivationService(
  dependencies: Readonly<CharacterActivationDependencies>,
): CharacterActivationService {
  let queue: Promise<void> = Promise.resolve();
  let currentRevision = 0;

  function serialize<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const result = queue.then(operation, operation);
    queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  async function activateNow(characterId: string): Promise<CommittedCharacterRuntime> {
    const token = await dependencies.prepareCharacterActivation(characterId);
    currentRevision = Math.max(currentRevision, token.revision);
    const snapshot = snapshotFrontendRuntime(dependencies.readFrontendRuntime());

    try {
      await handleLocalPreset(dependencies, token);
      const nextRuntime = resolveFrontendRuntimeProfile(token.resolved_runtime, snapshot);
      await dependencies.applyFrontendRuntime(nextRuntime);
      const committed = await dependencies.commitCharacterActivation(token);
      currentRevision = Math.max(currentRevision, committed.revision);
      writeCacheBestEffort(dependencies, committed);
      dependencies.dispatchRuntimeChanged(committed);
      return committed;
    } catch (error) {
      if (token.revision === currentRevision) {
        await dependencies.restoreFrontendRuntime(snapshot);
      }
      throw error;
    }
  }

  async function recoverNow(): Promise<CommittedCharacterRuntime | null> {
    const committed = await dependencies.getCommittedCharacterRuntime();
    if (committed === null) return null;

    const current = snapshotFrontendRuntime(dependencies.readFrontendRuntime());
    const recovered = resolveFrontendRuntimeProfile(committed.runtime, current);
    await dependencies.applyFrontendRuntime(recovered);
    currentRevision = Math.max(currentRevision, committed.revision);
    writeCacheBestEffort(dependencies, committed);
    dependencies.dispatchRuntimeChanged(committed);
    return committed;
  }

  return {
    activateCharacter(characterId: string): Promise<CommittedCharacterRuntime> {
      return serialize(() => activateNow(characterId));
    },
    recoverCommittedRuntime(): Promise<CommittedCharacterRuntime | null> {
      return serialize(recoverNow);
    },
  };
}
