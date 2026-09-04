// pattern: Functional Core
import type { ChatPanelMessage } from "./turn-state";

export type TurnStartValidationContext = {
    readonly currentGeneration: number;
    readonly activeConversationId: string | null;
    readonly activeCharacterId: string;
    readonly pendingRequest: {
        readonly clientRequestId?: string | null;
        readonly generation: number;
        readonly conversationId: string | null;
        readonly characterId: string;
    } | null;
    readonly isCancelRequested: boolean;
};

export type TurnStartEventPayload = {
    readonly turn_id: string;
    readonly client_request_id?: string | null;
    readonly conversation_id?: string | null;
    readonly user_message_id?: number | null;
};

export type TurnStartValidationResult =
    | {
          readonly valid: true;
          readonly shouldUpdateConversation: boolean;
          readonly targetConversationId: string | null;
          readonly matchedClientRequestId?: string | null;
      }
    | {
          readonly valid: false;
          readonly reason:
              | "cancelled"
              | "generation_mismatch"
              | "request_mismatch"
              | "conversation_mismatch";
      };

/**
 * Validates whether an incoming chat-turn-start event belongs to the current
 * conversation session and the pending client request.
 */
export function validateTurnStart(
    context: TurnStartValidationContext,
    payload: TurnStartEventPayload,
): TurnStartValidationResult {
    if (context.isCancelRequested) {
        return { valid: false, reason: "cancelled" };
    }

    if (context.pendingRequest) {
        if (context.pendingRequest.generation !== context.currentGeneration) {
            return { valid: false, reason: "generation_mismatch" };
        }

        if (
            payload.client_request_id &&
            context.pendingRequest.clientRequestId &&
            payload.client_request_id !== context.pendingRequest.clientRequestId
        ) {
            return { valid: false, reason: "request_mismatch" };
        }

        if (payload.conversation_id) {
            if (
                context.pendingRequest.conversationId !== null &&
                payload.conversation_id !== context.pendingRequest.conversationId
            ) {
                return { valid: false, reason: "conversation_mismatch" };
            }
        }

        return {
            valid: true,
            shouldUpdateConversation: Boolean(payload.conversation_id),
            targetConversationId: payload.conversation_id ?? context.activeConversationId,
            matchedClientRequestId: payload.client_request_id ?? context.pendingRequest.clientRequestId,
        };
    }

    // No pending request recorded (e.g. pet window or proactive trigger without tracking)
    if (payload.conversation_id) {
        if (
            context.activeConversationId !== null &&
            payload.conversation_id !== context.activeConversationId
        ) {
            return { valid: false, reason: "conversation_mismatch" };
        }
    }

    return {
        valid: true,
        shouldUpdateConversation: Boolean(payload.conversation_id),
        targetConversationId: payload.conversation_id ?? context.activeConversationId,
        matchedClientRequestId: payload.client_request_id,
    };
}

export type TurnFinishValidationContext = {
    readonly currentGeneration: number;
    readonly activeConversationId: string | null;
    readonly currentTurn: {
        readonly turnId: string;
        readonly generation: number;
        readonly conversationId: string | null;
    } | null;
};

export type TurnFinishEventPayload = {
    readonly turn_id: string;
    readonly status: "completed" | "error" | "cancelled";
    readonly conversation_id?: string | null;
    readonly assistant_message_id?: number | null;
    readonly client_request_id?: string | null;
};

export type TurnFinishValidationResult =
    | {
          readonly valid: true;
          readonly shouldUpdateConversation: boolean;
          readonly targetConversationId: string | null;
      }
    | {
          readonly valid: false;
          readonly reason:
              | "no_active_turn"
              | "turn_mismatch"
              | "generation_mismatch"
              | "conversation_mismatch";
      };

/**
 * Validates whether an incoming chat-turn-finish event belongs to the currently active turn
 * and conversation session before allowing conversation ID or message mutations.
 */
export function validateTurnFinish(
    context: TurnFinishValidationContext,
    payload: TurnFinishEventPayload,
): TurnFinishValidationResult {
    if (!context.currentTurn) {
        return { valid: false, reason: "no_active_turn" };
    }

    if (context.currentTurn.turnId !== payload.turn_id) {
        return { valid: false, reason: "turn_mismatch" };
    }

    if (context.currentTurn.generation !== context.currentGeneration) {
        return { valid: false, reason: "generation_mismatch" };
    }

    if (
        payload.conversation_id &&
        context.currentTurn.conversationId &&
        payload.conversation_id !== context.currentTurn.conversationId
    ) {
        return { valid: false, reason: "conversation_mismatch" };
    }

    return {
        valid: true,
        shouldUpdateConversation: Boolean(payload.conversation_id),
        targetConversationId:
            payload.conversation_id ??
            context.currentTurn.conversationId ??
            context.activeConversationId,
    };
}

export type StreamChatResponseValidationContext = {
    readonly requestGeneration: number;
    readonly currentGeneration: number;
    readonly clientRequestId?: string | null;
    readonly activeConversationId: string | null;
};

export type StreamChatResponsePayload = {
    readonly conversation_id: string;
    readonly user_message_id?: number | null;
    readonly assistant_message_id?: number | null;
};

export type StreamChatResponseValidationResult =
    | {
          readonly valid: true;
          readonly shouldUpdateConversation: boolean;
          readonly targetConversationId: string;
      }
    | {
          readonly valid: false;
          readonly reason: "generation_mismatch";
      };

/**
 * Validates whether the resolved streamChat response promise still belongs to
 * the active conversation session.
 */
export function validateStreamChatResponse(
    context: StreamChatResponseValidationContext,
    payload?: StreamChatResponsePayload | null,
): StreamChatResponseValidationResult {
    if (context.requestGeneration !== context.currentGeneration) {
        return { valid: false, reason: "generation_mismatch" };
    }

    const conversationId = payload?.conversation_id ?? context.activeConversationId;
    return {
        valid: true,
        shouldUpdateConversation: Boolean(payload?.conversation_id),
        targetConversationId: conversationId ?? "",
    };
}

export type TurnMessageIdReconciliation = {
    readonly messages: Array<ChatPanelMessage>;
    /**
     * true 表示权威 ID 无法在内存中可靠对齐（assistant 气泡缺失或已挂有冲突 ID）。
     * 调用方应放弃对齐并从后端重新同步消息列表，而不是按"最后一条消息"猜测写入。
     */
    readonly needsResync: boolean;
};

/**
 * 协调并将 StreamChatResponse 返回的权威消息 ID（user_message_id 与 assistant_message_id）
 * 补偿填充到前端消息列表中。
 *
 * 当因组件初始化、生命周期切换或异步事件丢失而未收到 chat-turn-finish 事件时，
 * 该函数可防止 assistant 消息缺少 ID 导致后续无法编辑的问题。
 *
 * 对齐策略为按 clientRequestId 严格精确匹配，绝不按位置猜测：找不到明确匹配项时
 * 返回 needsResync = true，由调用方从后端重新同步，避免把新消息 ID 绑定到
 * Telegram、已取消旧 turn 等无关消息上（否则后续编辑会修改错误记录）。
 *
 * @param messages 当前消息列表
 * @param clientRequestId 客户端请求唯一标识符
 * @param userMessageId 后端持久化返回的用户消息权威 ID
 * @param assistantMessageId 后端持久化返回的助手消息权威 ID
 * @returns 补齐 ID 后的消息列表副本；若无需变更则 messages 字段保留原数组引用
 */
export function reconcileTurnMessageIds(
    messages: ReadonlyArray<ChatPanelMessage>,
    clientRequestId?: string | null,
    userMessageId?: number | null,
    assistantMessageId?: number | null,
): TurnMessageIdReconciliation {
    if (!userMessageId && !assistantMessageId) {
        return { messages: messages as Array<ChatPanelMessage>, needsResync: false };
    }

    let isModified = false;
    let needsResync = false;
    const nextMessages = [...messages];

    // 第一步：如果提供了 userMessageId，按 clientRequestId 精确补齐用户消息 ID；
    // 未匹配时静默跳过（编辑时 onEdit 会安全失败，不会写错记录）
    if (userMessageId && clientRequestId) {
        const userIndex = nextMessages.findIndex(m => m.clientRequestId === clientRequestId);
        if (userIndex !== -1 && !nextMessages[userIndex].id) {
            nextMessages[userIndex] = {
                ...nextMessages[userIndex],
                id: userMessageId,
            };
            isModified = true;
        }
    }

    // 第二步：如果提供了 assistantMessageId，按 clientRequestId 精确补齐助手消息 ID
    if (assistantMessageId) {
        const assistantIndex = clientRequestId
            ? nextMessages.findIndex(
                m => m.role === "kokoro" && m.clientRequestId === clientRequestId,
            )
            : -1;

        if (assistantIndex === -1) {
            // 找不到明确匹配项：不猜测，交由调用方从后端重新同步
            needsResync = true;
        } else if (!nextMessages[assistantIndex].id) {
            nextMessages[assistantIndex] = {
                ...nextMessages[assistantIndex],
                id: assistantMessageId,
            };
            isModified = true;
        } else if (nextMessages[assistantIndex].id !== assistantMessageId) {
            // 已挂有冲突 ID（历史误绑或重复对齐）：内存态不可信，交由后端重新同步
            needsResync = true;
        }
    }

    return {
        messages: isModified ? nextMessages : (messages as Array<ChatPanelMessage>),
        needsResync,
    };
}

/**
 * resync 入口守卫：仅当 UI 空闲、或当前活动仍归属本次请求（turn-start/finish 事件
 * 丢失导致的残留）时允许重同步。busy 归属其他请求时返回 false，避免覆盖其乐观消息。
 */
export function shouldResyncConversation(
    clientRequestId: string,
    activeTurnClientRequestId: string | null | undefined,
    isBusy: boolean,
    pendingClientRequestId: string | null | undefined,
): boolean {
    if (activeTurnClientRequestId) {
        return activeTurnClientRequestId === clientRequestId;
    }
    if (!isBusy) {
        return true;
    }
    return pendingClientRequestId === clientRequestId;
}

/**
 * 将后端权威消息列表应用到当前内存消息列表（resync 专用）。
 *
 * 当前列表尾部"未绑定"的消息（无 id、无 clientRequestId、无 turnId——如 Telegram
 * 同步气泡、错误提示气泡）可能尚未进入 DB 或不属于当前会话，直接整体替换会丢失
 * 它们，因此将其保留并追加到权威列表之后；与权威列表内容重复（同角色同文本）的
 * 则丢弃，避免 DB 快照已包含该消息时出现重复气泡。
 */
export function mergeResyncedConversationMessages(
    current: ReadonlyArray<ChatPanelMessage>,
    authoritative: ReadonlyArray<ChatPanelMessage>,
): Array<ChatPanelMessage> {
    const trailing: ChatPanelMessage[] = [];
    for (let i = current.length - 1; i >= 0; i--) {
        const message = current[i];
        const isUnbound = message.id === undefined
            && message.clientRequestId === undefined
            && message.turnId === undefined;
        if (!isUnbound) break;
        trailing.unshift(message);
    }

    if (trailing.length === 0) {
        return authoritative as Array<ChatPanelMessage>;
    }

    const kept = trailing.filter(trailingMessage => !authoritative.some(
        dbMessage => dbMessage.role === trailingMessage.role && dbMessage.text === trailingMessage.text,
    ));

    return kept.length === 0
        ? (authoritative as Array<ChatPanelMessage>)
        : [...authoritative, ...kept];
}

/**
 * 判断跨多个 await 的异步链路（重新生成/继续）在入口处捕获的会话快照是否仍然有效。
 *
 * 在第一次异步操作前捕获代次与会话 ID，每个 await 返回后调用本函数校验：
 * 若期间用户切换了角色或会话（代次必递增，会话 ID 会变化），必须立即中止后续的
 * 删除与请求，防止操作作用到新会话上。
 */
export function isChatSessionCurrent(
    startGeneration: number,
    startConversationId: string | null,
    currentGeneration: number,
    currentConversationId: string | null,
): boolean {
    return (
        currentGeneration === startGeneration &&
        currentConversationId === startConversationId
    );
}
