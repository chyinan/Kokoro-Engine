// pattern: Functional Core

import type {
  BackendCharacterRuntime,
  PackageAssetReference,
  ResolvedTtsMode,
} from "@/lib/kokoro-bridge";
import { BUILTIN_LIVE2D_MODEL_PATH } from "@/lib/kokoro-bridge";

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
  /** Identifies whether the current value is a user override or character asset. */
  live2dModelSource?: FrontendAssetSource;
  background: string | null;
  backgroundSource?: FrontendAssetSource;
  tts: FrontendTtsRuntime;
  cueProfile: string | null;
  cueProfileSource?: FrontendAssetSource;
};

export type FrontendAssetSource = "user" | "package" | "builtin" | "none";

/** Validates a persisted source marker and migrates legacy non-empty values. */
export function resolveFrontendAssetSource(
  marker: string | undefined,
  value: string | null,
): FrontendAssetSource {
  if (!marker) return value === null ? "none" : "user";
  return marker === "user" || marker === "package" || marker === "builtin" || marker === "none"
    ? marker
    : value === null ? "none" : "user";
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
    ...(state.live2dModelSource === undefined ? {} : { live2dModelSource: state.live2dModelSource }),
    background: state.background,
    ...(state.backgroundSource === undefined ? {} : { backgroundSource: state.backgroundSource }),
    tts: { ...state.tts },
    cueProfile: state.cueProfile,
    ...(state.cueProfileSource === undefined ? {} : { cueProfileSource: state.cueProfileSource }),
  };
}

function resolveAsset(
  reference: PackageAssetReference | null,
  fallbackValue: string | null,
  fallbackSource: FrontendAssetSource | undefined,
  defaultValue: string | null,
  defaultSource: FrontendAssetSource,
): { value: string | null; source: FrontendAssetSource } {
  if (reference?.source === "package") {
    return { value: reference.path, source: "package" };
  }
  if (reference?.source === "library") {
    return { value: reference.model_id, source: "user" };
  }
  if (fallbackSource === "user" && fallbackValue !== null) {
    return { value: fallbackValue, source: "user" };
  }
  return { value: defaultValue, source: defaultSource };
}

/** Maps the backend-authoritative runtime onto the complete frontend state. */
export function resolveFrontendRuntimeProfile(
  runtime: Readonly<PreparedCharacterRuntime>,
  fallback: Readonly<FrontendRuntimeState>,
): FrontendRuntimeState {
  const isTtsEnabled = runtime.tts.mode !== "text_only";
  const live2d = resolveAsset(
    runtime.live2d_model,
    fallback.live2dModel,
    fallback.live2dModelSource,
    BUILTIN_LIVE2D_MODEL_PATH,
    "builtin",
  );
  const background = resolveAsset(
    runtime.background,
    fallback.background,
    fallback.backgroundSource,
    null,
    "none",
  );
  const cueProfile = resolveAsset(
    runtime.cue_profile,
    fallback.cueProfile,
    fallback.cueProfileSource,
    null,
    "none",
  );
  return {
    activeCharacterId: runtime.character_id,
    live2dModel: live2d.value,
    live2dModelSource: live2d.source,
    background: background.value,
    backgroundSource: background.source,
    tts: {
      enabled: isTtsEnabled,
      mode: runtime.tts.mode,
      providerId: runtime.tts.provider_id,
      voice: runtime.tts.voice,
      speed: runtime.tts.speed ?? 1,
      pitch: runtime.tts.pitch ?? 1,
    },
    cueProfile: cueProfile.value,
    cueProfileSource: cueProfile.source,
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
