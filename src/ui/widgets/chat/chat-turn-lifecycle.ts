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

/**
 * 协调并将 StreamChatResponse 返回的权威消息 ID（user_message_id 与 assistant_message_id）
 * 补偿填充到前端消息列表中。
 *
 * 当因组件初始化、生命周期切换或异步事件丢失而未收到 chat-turn-finish 事件时，
 * 该函数可防止 assistant 消息缺少 ID 导致后续无法编辑的问题。
 *
 * @param messages 当前消息列表
 * @param clientRequestId 客户端请求唯一标识符
 * @param userMessageId 后端持久化返回的用户消息权威 ID
 * @param assistantMessageId 后端持久化返回的助手消息权威 ID
 * @returns 补齐 ID 后的消息列表副本；若无需变更则返回原数组以保留对象引用
 */
export function reconcileTurnMessageIds(
    messages: ReadonlyArray<ChatPanelMessage>,
    clientRequestId?: string | null,
    userMessageId?: number | null,
    assistantMessageId?: number | null,
): Array<ChatPanelMessage> {
    if (!userMessageId && !assistantMessageId) {
        return messages as Array<ChatPanelMessage>;
    }

    let isModified = false;
    const nextMessages = [...messages];

    // 第一步：如果提供了 userMessageId，定位并补齐用户消息 ID
    if (userMessageId) {
        let userIndex = clientRequestId
            ? nextMessages.findIndex(m => m.clientRequestId === clientRequestId)
            : -1;

        if (userIndex === -1 && !clientRequestId) {
            userIndex = nextMessages.map(m => m.role).lastIndexOf("user");
        }

        if (userIndex !== -1 && !nextMessages[userIndex].id) {
            nextMessages[userIndex] = {
                ...nextMessages[userIndex],
                id: userMessageId,
            };
            isModified = true;
        }
    }

    // 第二步：如果提供了 assistantMessageId，定位并补齐助手消息 ID
    if (assistantMessageId) {
        let assistantIndex = -1;

        // 优先根据 clientRequestId 匹配 assistant 气泡
        if (clientRequestId) {
            assistantIndex = nextMessages.findIndex(
                m => m.role === "kokoro" && m.clientRequestId === clientRequestId,
            );
        }

        // 次选：若 assistant 气泡尚未打上 clientRequestId，寻找紧随该 user 消息之后的第一个 kokoro 气泡
        if (assistantIndex === -1 && clientRequestId) {
            const userIndex = nextMessages.findIndex(m => m.clientRequestId === clientRequestId);
            if (userIndex !== -1) {
                for (let i = userIndex + 1; i < nextMessages.length; i++) {
                    if (nextMessages[i].role === "kokoro") {
                        assistantIndex = i;
                        break;
                    }
                }
            }
        }

        // 兜底（如重新生成无新 user 消息）：取最后一条 kokoro 助手气泡
        if (assistantIndex === -1) {
            assistantIndex = nextMessages.map(m => m.role).lastIndexOf("kokoro");
        }

        if (assistantIndex !== -1 && !nextMessages[assistantIndex].id) {
            nextMessages[assistantIndex] = {
                ...nextMessages[assistantIndex],
                id: assistantMessageId,
                clientRequestId: nextMessages[assistantIndex].clientRequestId ?? clientRequestId ?? undefined,
            };
            isModified = true;
        }
    }

    return isModified ? nextMessages : (messages as Array<ChatPanelMessage>);
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
