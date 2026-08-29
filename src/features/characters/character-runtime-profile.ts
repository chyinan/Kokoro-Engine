// pattern: Functional Core

import type {
  BackendCharacterRuntime,
  ResolvedTtsMode,
} from "@/lib/kokoro-bridge";

export type PreparedCharacterRuntime = BackendCharacterRuntime;

export type FrontendTtsRuntime = {
  enabled: boolean;
  mode: ResolvedTtsMode;
  providerId: string | null;
  voice: string | null;
  speed: number;
  pitch: number;
};

export type FrontendRuntimeState = {
  activeCharacterId: string | null;
  live2dModel: string | null;
  background: string | null;
  tts: FrontendTtsRuntime;
  cueProfile: string | null;
};

function packageAssetPath(
  reference: PreparedCharacterRuntime["live2d_model"],
): string | null {
  if (reference?.source === "package") return reference.path;
  if (reference?.source === "library") return reference.model_id;
  return null;
}

/**
 * Copies every frontend value owned by character activation so rollback cannot
 * observe later mutations from React state or settings adapters.
 */
export function snapshotFrontendRuntime(
  state: Readonly<FrontendRuntimeState>,
): FrontendRuntimeState {
  return {
    activeCharacterId: state.activeCharacterId,
    live2dModel: state.live2dModel,
    background: state.background,
    tts: { ...state.tts },
    cueProfile: state.cueProfile,
  };
}

/** Maps the backend-authoritative runtime onto the complete frontend state. */
export function resolveFrontendRuntimeProfile(
  runtime: Readonly<PreparedCharacterRuntime>,
  fallback: Readonly<FrontendRuntimeState>,
): FrontendRuntimeState {
  const isTtsEnabled = runtime.tts.mode !== "text_only";
  return {
    activeCharacterId: runtime.character_id,
    live2dModel: packageAssetPath(runtime.live2d_model) ?? fallback.live2dModel,
    background: packageAssetPath(runtime.background) ?? fallback.background,
    tts: {
      enabled: isTtsEnabled,
      mode: runtime.tts.mode,
      providerId: runtime.tts.provider_id,
      voice: runtime.tts.voice,
      speed: runtime.tts.speed ?? 1,
      pitch: runtime.tts.pitch ?? 1,
    },
    cueProfile: packageAssetPath(runtime.cue_profile) ?? fallback.cueProfile,
  };
}

/** Restricts character-requested local probes to Kokoro's conventional endpoint. */
export function isConventionalLoopbackTtsEndpoint(endpoint: string): boolean {
  try {
    const url = new URL(endpoint);
    const isLoopbackHost = url.hostname === "127.0.0.1" || url.hostname === "localhost";
    const isRootPath = url.pathname === "/" && url.search === "" && url.hash === "";
    return url.protocol === "http:" && isLoopbackHost && url.port === "9880" && isRootPath;
  } catch {
    return false;
  }
}
