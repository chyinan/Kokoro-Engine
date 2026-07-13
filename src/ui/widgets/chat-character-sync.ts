// pattern: Imperative Shell

import type {
  Conversation,
  LoadedConversation,
} from "../../lib/kokoro-bridge";

import {
  buildChatMessagesFromConversation,
  type ChatHistoryMessage,
} from "./chat-history";

export type CharacterConversationTarget = {
  readonly characterId: string;
  readonly preferredConversationId: string | null;
};

export type VisibleCharacterConversation = {
  readonly characterId: string;
  readonly conversationId: string | null;
  readonly messages: ReadonlyArray<ChatHistoryMessage>;
};

export type ChatCharacterSyncDependencies = {
  readonly listConversations: (characterId: string) => Promise<Array<Conversation>>;
  readonly loadConversation: (conversationId: string) => Promise<LoadedConversation>;
  readonly clearVisibleConversation: (characterId: string) => void;
  readonly applyVisibleConversation: (
    conversation: Readonly<VisibleCharacterConversation>,
  ) => void;
};

export type ChatCharacterSynchronizer = {
  readonly synchronize: (
    target: Readonly<CharacterConversationTarget>,
  ) => Promise<void>;
  readonly startEmptyConversation: (characterId: string) => void;
  readonly invalidate: () => void;
};

function selectOwnedConversation(
  conversations: ReadonlyArray<Conversation>,
  target: Readonly<CharacterConversationTarget>,
): Conversation | null {
  const owned = conversations.filter(
    (conversation) => conversation.character_id === target.characterId,
  );
  return owned.find(
    (conversation) => conversation.id === target.preferredConversationId,
  ) ?? owned[0] ?? null;
}

/**
 * Creates the single race-safe loader for character-owned chat history.
 * Each request clears the visible chat before any I/O and supersedes older
 * requests so a slow prior character can never become visible again.
 */
export function createChatCharacterSynchronizer(
  dependencies: Readonly<ChatCharacterSyncDependencies>,
): ChatCharacterSynchronizer {
  let requestRevision = 0;

  async function synchronize(
    target: Readonly<CharacterConversationTarget>,
  ): Promise<void> {
    requestRevision += 1;
    const revision = requestRevision;
    dependencies.clearVisibleConversation(target.characterId);

    const conversations = await dependencies.listConversations(target.characterId);
    if (revision !== requestRevision) return;

    const selected = selectOwnedConversation(conversations, target);
    if (selected === null) {
      dependencies.applyVisibleConversation({
        characterId: target.characterId,
        conversationId: null,
        messages: [],
      });
      return;
    }

    const loaded = await dependencies.loadConversation(selected.id);
    if (revision !== requestRevision) return;

    dependencies.applyVisibleConversation({
      characterId: target.characterId,
      conversationId: selected.id,
      messages: buildChatMessagesFromConversation(loaded.messages),
    });
  }

  return {
    synchronize,
    startEmptyConversation(characterId: string): void {
      requestRevision += 1;
      dependencies.clearVisibleConversation(characterId);
    },
    invalidate(): void {
      requestRevision += 1;
    },
  };
}
