// pattern: Imperative Shell

import { describe, expect, it, vi } from "vitest";

import {
  createChatCharacterSynchronizer,
  getInitialCharacterConversationTarget,
  isFailureForActiveChat,
  shouldIgnoreLegacyChatError,
} from "./chat-character-sync";

type PendingValue<TValue> = {
  readonly promise: Promise<TValue>;
  readonly resolve: (value: TValue) => void;
};

function pendingValue<TValue>(): PendingValue<TValue> {
  let resolve!: (value: TValue) => void;
  const promise = new Promise<TValue>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function conversation(id: string, characterId: string) {
  return {
    id,
    character_id: characterId,
    title: id,
    topic: "",
    pinned_state: "{}",
    created_at: "2026-07-14T00:00:00Z",
    updated_at: "2026-07-14T00:00:00Z",
  };
}

function loaded(content: string) {
  return {
    topic: "",
    pinned_state: "{}",
    messages: [{
      role: "assistant",
      content,
      created_at: "2026-07-14T00:00:00Z",
    }],
  };
}

describe("character conversation synchronization", () => {
  it("drops unverifiable legacy chat errors while a turn is active", () => {
    expect(shouldIgnoreLegacyChatError("turn-1")).toBe(true);
    expect(shouldIgnoreLegacyChatError(null)).toBe(true);
  });
  it("hydrates the backend-committed conversation when activation completed before mount", () => {
    const target = getInitialCharacterConversationTarget("pico", {
      revision: 9,
      runtime: { character_id: "pico" },
      target_conversation_id: "pico-committed",
    });

    expect(target).toEqual({
      characterId: "pico",
      preferredConversationId: "pico-committed",
    });
  });

  it("ignores late failure events from an old character or old turn", () => {
    expect(isFailureForActiveChat({ character_id: "kokoro", turn_id: "turn-new" }, "pico", "turn-new")).toBe(false);
    expect(isFailureForActiveChat({ character_id: "pico", turn_id: "turn-old" }, "pico", "turn-new")).toBe(false);
    expect(isFailureForActiveChat({ character_id: "pico", turn_id: "turn-new" }, "pico", "turn-new")).toBe(true);
  });

  it("clears the prior character immediately and loads the activated conversation", async () => {
    const lists = pendingValue<Array<ReturnType<typeof conversation>>>();
    const clearVisibleConversation = vi.fn();
    const applyVisibleConversation = vi.fn();
    const sync = createChatCharacterSynchronizer({
      listConversations: vi.fn(() => lists.promise),
      loadConversation: vi.fn(async () => loaded("pico history")),
      clearVisibleConversation,
      applyVisibleConversation,
    });

    const operation = sync.synchronize({
      characterId: "pico",
      preferredConversationId: "pico-chat",
    });

    expect(clearVisibleConversation).toHaveBeenCalledWith("pico");
    expect(applyVisibleConversation).not.toHaveBeenCalled();

    lists.resolve([conversation("pico-chat", "pico")]);
    await operation;

    expect(applyVisibleConversation).toHaveBeenCalledWith({
      characterId: "pico",
      conversationId: "pico-chat",
      messages: [{ role: "kokoro", text: "pico history" }],
    });
  });

  it("does not let a slower prior-character load replace the newer character", async () => {
    const oldList = pendingValue<Array<ReturnType<typeof conversation>>>();
    const applyVisibleConversation = vi.fn();
    const listConversations = vi.fn((characterId: string) =>
      characterId === "kokoro"
        ? oldList.promise
        : Promise.resolve([conversation("seren-chat", "seren")]),
    );
    const sync = createChatCharacterSynchronizer({
      listConversations,
      loadConversation: vi.fn(async (id: string) => loaded(`${id} history`)),
      clearVisibleConversation: vi.fn(),
      applyVisibleConversation,
    });

    const oldOperation = sync.synchronize({ characterId: "kokoro", preferredConversationId: null });
    await sync.synchronize({ characterId: "seren", preferredConversationId: "seren-chat" });
    oldList.resolve([conversation("kokoro-chat", "kokoro")]);
    await oldOperation;

    expect(applyVisibleConversation).toHaveBeenCalledTimes(1);
    expect(applyVisibleConversation).toHaveBeenCalledWith(expect.objectContaining({
      characterId: "seren",
      conversationId: "seren-chat",
    }));
  });

  it("falls back to the first owned conversation when the preferred one is missing", async () => {
    const loadConversation = vi.fn(async () => loaded("fallback history"));
    const applyVisibleConversation = vi.fn();
    const sync = createChatCharacterSynchronizer({
      listConversations: vi.fn(async () => [
        conversation("foreign-chat", "kokoro"),
        conversation("pico-fallback", "pico"),
      ]),
      loadConversation,
      clearVisibleConversation: vi.fn(),
      applyVisibleConversation,
    });

    await sync.synchronize({
      characterId: "pico",
      preferredConversationId: "missing-chat",
    });

    expect(loadConversation).toHaveBeenCalledWith("pico-fallback");
    expect(applyVisibleConversation).toHaveBeenCalledWith(expect.objectContaining({
      characterId: "pico",
      conversationId: "pico-fallback",
    }));
  });

  it("starts an empty conversation without letting an in-flight history reload it", async () => {
    const oldList = pendingValue<Array<ReturnType<typeof conversation>>>();
    const clearVisibleConversation = vi.fn();
    const applyVisibleConversation = vi.fn();
    const sync = createChatCharacterSynchronizer({
      listConversations: vi.fn(() => oldList.promise),
      loadConversation: vi.fn(async () => loaded("old history")),
      clearVisibleConversation,
      applyVisibleConversation,
    });

    const oldOperation = sync.synchronize({
      characterId: "pico",
      preferredConversationId: null,
    });
    sync.startEmptyConversation("pico");
    oldList.resolve([conversation("pico-old", "pico")]);
    await oldOperation;

    expect(clearVisibleConversation).toHaveBeenLastCalledWith("pico");
    expect(applyVisibleConversation).not.toHaveBeenCalled();
  });
});
