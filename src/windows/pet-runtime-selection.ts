// pattern: Functional Core

import type { CommittedCharacterRuntime } from "../lib/kokoro-bridge";

export type PetModelSource = "user" | "package" | "builtin";

export type PetModelSelection = {
  readonly modelPath: string;
  readonly source: PetModelSource;
};

/** Selects the committed character model, falling back safely during recovery. */
export function selectPetModelSelection(
  committed: Readonly<CommittedCharacterRuntime> | null,
  fallback: string,
): PetModelSelection {
  const reference = committed?.runtime.live2d_model;
  if (reference?.source === "package") return { modelPath: reference.path, source: "package" };
  if (reference?.source === "library") return { modelPath: reference.model_id, source: "user" };
  return { modelPath: fallback, source: "builtin" };
}

export function selectPetModelPath(
  committed: Readonly<CommittedCharacterRuntime> | null,
  fallback: string,
): string {
  return selectPetModelSelection(committed, fallback).modelPath;
}
