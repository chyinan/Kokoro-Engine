// pattern: Functional Core

import { describe, expect, it } from "vitest";

import {
  canStartBasicChat,
  deriveMemoryModelState,
  semanticRetrievalMode,
  type MemoryModelSnapshot,
} from "./memory-model-gate";

describe("memory model gate", () => {
  it.each([
    ["missing", { installed: false }],
    ["downloading", { installed: false, downloading: true }],
    ["error", { installed: false, error: "download failed" }],
  ] as const)("keeps basic chat available while the model is %s", (expected, snapshot) => {
    const state = deriveMemoryModelState(snapshot satisfies MemoryModelSnapshot);

    expect(state).toBe(expected);
    expect(canStartBasicChat(state)).toBe(true);
    expect(semanticRetrievalMode(state)).toBe("unavailable");
  });

  it("uses semantic retrieval only after the model is ready", () => {
    const state = deriveMemoryModelState({ installed: true });

    expect(state).toBe("ready");
    expect(canStartBasicChat(state)).toBe(true);
    expect(semanticRetrievalMode(state)).toBe("ready");
  });
});
