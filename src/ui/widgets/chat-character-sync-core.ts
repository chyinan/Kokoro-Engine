// pattern: Functional Core

export type CachedCommittedConversation = {
  readonly revision?: number;
  readonly runtime?: { readonly character_id?: string | null } | null;
  readonly target_conversation_id?: string | null;
};

export type ChatFailureIdentity = {
  readonly character_id?: string | null;
  readonly turn_id?: string | null;
};

export type InitialCharacterConversationTarget = {
  readonly characterId: string;
  readonly preferredConversationId: string | null;
};

/** Uses the committed cache only when it belongs to the active character. */
export function getInitialCharacterConversationTarget(
  activeCharacterId: string,
  committed: Readonly<CachedCommittedConversation> | null,
): InitialCharacterConversationTarget {
  const isMatchingCharacter = committed?.runtime?.character_id === activeCharacterId;
  return {
    characterId: activeCharacterId,
    preferredConversationId: isMatchingCharacter
      ? committed?.target_conversation_id ?? null
      : null,
  };
}

/** Rejects delayed structured failures that no longer own the visible chat turn. */
export function isFailureForActiveChat(
  failure: Readonly<ChatFailureIdentity>,
  activeCharacterId: string,
  activeTurnId: string | null,
): boolean {
  if (failure.character_id && failure.character_id !== activeCharacterId) return false;
  if (failure.turn_id) return activeTurnId !== null && failure.turn_id === activeTurnId;
  return activeTurnId !== null;
}
