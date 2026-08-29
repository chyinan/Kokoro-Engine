// pattern: Functional Core

import type { Live2dCueBinding } from "@/lib/kokoro-bridge";

export type AppliedCharacterCueProfile = {
  readonly cueMap: Readonly<Record<string, Live2dCueBinding>>;
  readonly semanticCueMap: Readonly<Record<string, string>>;
};

function optionalString(record: Readonly<Record<string, unknown>>, key: string): string | null {
  const value = record[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

/** Converts the backend-validated data file into the existing Live2D profile contract. */
export function parseCharacterCueProfile(value: unknown): AppliedCharacterCueProfile {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { cueMap: {}, semanticCueMap: {} };
  }
  const root = value as Record<string, unknown>;
  if (root.schema_version !== 1 || typeof root.cues !== "object" || root.cues === null || Array.isArray(root.cues)) {
    return { cueMap: {}, semanticCueMap: {} };
  }
  const cueMap: Record<string, Live2dCueBinding> = Object.create(null) as Record<string, Live2dCueBinding>;
  for (const [rawName, rawBinding] of Object.entries(root.cues as Record<string, unknown>)) {
    const name = rawName.trim();
    if (!name || name === "__proto__" || name === "constructor" || name === "prototype") continue;
    if (typeof rawBinding !== "object" || rawBinding === null || Array.isArray(rawBinding)) continue;
    const binding = rawBinding as Record<string, unknown>;
    const expression = optionalString(binding, "expression");
    const motionGroup = optionalString(binding, "motion_group");
    if (expression === null && motionGroup === null) continue;
    cueMap[name] = {
      ...(expression === null ? {} : { expression }),
      ...(motionGroup === null ? {} : { motion_group: motionGroup }),
    };
  }
  const defaultCue = typeof root.default === "string"
    && Object.prototype.hasOwnProperty.call(cueMap, root.default)
    ? root.default
    : null;
  return {
    cueMap,
    semanticCueMap: defaultCue === null ? {} : { default: defaultCue },
  };
}
