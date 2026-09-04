// pattern: Functional Core

import { describe, expect, it } from "vitest";
import {
    validateTurnStart,
    validateTurnFinish,
    validateStreamChatResponse,
    reconcileTurnMessageIds,
    isChatSessionCurrent,
    type TurnStartValidationContext,
    type TurnFinishValidationContext,
    type StreamChatResponseValidationContext,
} from "./chat-turn-lifecycle";
import type { ChatPanelMessage } from "./turn-state";

describe("chat turn lifecycle validation", () => {
    describe("validateTurnStart", () => {
        it("accepts turn start matching current generation and client request id", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-1",
                    generation: 1,
                    conversationId: "conv-1",
                    characterId: "char-1",
                },
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-1",
                conversation_id: "conv-1",
                user_message_id: 10,
            });

            expect(result).toEqual({
                valid: true,
                shouldUpdateConversation: true,
                targetConversationId: "conv-1",
                matchedClientRequestId: "req-1",
            });
        });

        it("accepts new conversation initialization when pending request had null conversationId", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: null,
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-1",
                    generation: 1,
                    conversationId: null,
                    characterId: "char-1",
                },
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-1",
                conversation_id: "conv-new-created",
                user_message_id: 10,
            });

            expect(result).toEqual({
                valid: true,
                shouldUpdateConversation: true,
                targetConversationId: "conv-new-created",
                matchedClientRequestId: "req-1",
            });
        });

        it("rejects turn start when session generation changed (user switched conversation)", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 2, // switched to new conversation session
                activeConversationId: "conv-2",
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-1",
                    generation: 1, // belonged to session 1
                    conversationId: "conv-1",
                    characterId: "char-1",
                },
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-1",
                conversation_id: "conv-1",
                user_message_id: 10,
            });

            expect(result).toEqual({
                valid: false,
                reason: "generation_mismatch",
            });
        });

        it("rejects turn start when client request id does not match pending request", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-2",
                    generation: 1,
                    conversationId: "conv-1",
                    characterId: "char-1",
                },
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-old-1",
                conversation_id: "conv-1",
            });

            expect(result).toEqual({
                valid: false,
                reason: "request_mismatch",
            });
        });

        it("rejects turn start when cancel was requested", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-1",
                    generation: 1,
                    conversationId: "conv-1",
                    characterId: "char-1",
                },
                isCancelRequested: true,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-1",
                conversation_id: "conv-1",
            });

            expect(result).toEqual({
                valid: false,
                reason: "cancelled",
            });
        });

        it("rejects turn start when conversation id conflicts with pending request", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                activeCharacterId: "char-1",
                pendingRequest: {
                    clientRequestId: "req-1",
                    generation: 1,
                    conversationId: "conv-1",
                    characterId: "char-1",
                },
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-1",
                client_request_id: "req-1",
                conversation_id: "conv-different",
            });

            expect(result).toEqual({
                valid: false,
                reason: "conversation_mismatch",
            });
        });

        it("handles untracked external trigger matching active conversation", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                activeCharacterId: "char-1",
                pendingRequest: null,
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-external",
                conversation_id: "conv-1",
            });

            expect(result).toEqual({
                valid: true,
                shouldUpdateConversation: true,
                targetConversationId: "conv-1",
                matchedClientRequestId: undefined,
            });
        });

        it("rejects untracked external trigger targeting a different conversation", () => {
            const context: TurnStartValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-active",
                activeCharacterId: "char-1",
                pendingRequest: null,
                isCancelRequested: false,
            };

            const result = validateTurnStart(context, {
                turn_id: "turn-external",
                conversation_id: "conv-other",
            });

            expect(result).toEqual({
                valid: false,
                reason: "conversation_mismatch",
            });
        });
    });

    describe("validateTurnFinish", () => {
        it("accepts finish event when turn, generation and conversation all match", () => {
            const context: TurnFinishValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                currentTurn: {
                    turnId: "turn-1",
                    generation: 1,
                    conversationId: "conv-1",
                },
            };

            const result = validateTurnFinish(context, {
                turn_id: "turn-1",
                status: "completed",
                conversation_id: "conv-1",
                assistant_message_id: 20,
            });

            expect(result).toEqual({
                valid: true,
                shouldUpdateConversation: true,
                targetConversationId: "conv-1",
            });
        });

        it("rejects finish event when no active turn exists (e.g. ChatPanel.tsx:1098 bug)", () => {
            const context: TurnFinishValidationContext = {
                currentGeneration: 2,
                activeConversationId: "conv-2",
                currentTurn: null, // cleared by conversation switch / startEmptyConversation
            };

            const result = validateTurnFinish(context, {
                turn_id: "turn-old",
                status: "completed",
                conversation_id: "conv-old",
                assistant_message_id: 99,
            });

            expect(result).toEqual({
                valid: false,
                reason: "no_active_turn",
            });
        });

        it("rejects finish event when turn id does not match active turn", () => {
            const context: TurnFinishValidationContext = {
                currentGeneration: 2,
                activeConversationId: "conv-2",
                currentTurn: {
                    turnId: "turn-new-2",
                    generation: 2,
                    conversationId: "conv-2",
                },
            };

            const result = validateTurnFinish(context, {
                turn_id: "turn-stale-1",
                status: "completed",
                conversation_id: "conv-1",
            });

            expect(result).toEqual({
                valid: false,
                reason: "turn_mismatch",
            });
        });

        it("rejects finish event when generation changed", () => {
            const context: TurnFinishValidationContext = {
                currentGeneration: 3,
                activeConversationId: "conv-3",
                currentTurn: {
                    turnId: "turn-1",
                    generation: 2, // generation out of date
                    conversationId: "conv-2",
                },
            };

            const result = validateTurnFinish(context, {
                turn_id: "turn-1",
                status: "completed",
                conversation_id: "conv-2",
            });

            expect(result).toEqual({
                valid: false,
                reason: "generation_mismatch",
            });
        });

        it("rejects finish event when conversation id conflicts with current turn conversationId", () => {
            const context: TurnFinishValidationContext = {
                currentGeneration: 1,
                activeConversationId: "conv-1",
                currentTurn: {
                    turnId: "turn-1",
                    generation: 1,
                    conversationId: "conv-1",
                },
            };

            const result = validateTurnFinish(context, {
                turn_id: "turn-1",
                status: "completed",
                conversation_id: "conv-tampered",
            });

            expect(result).toEqual({
                valid: false,
                reason: "conversation_mismatch",
            });
        });
    });

    describe("validateStreamChatResponse", () => {
        it("accepts streamChat response when generation matches", () => {
            const context: StreamChatResponseValidationContext = {
                requestGeneration: 1,
                currentGeneration: 1,
                clientRequestId: "req-1",
                activeConversationId: null,
            };

            const result = validateStreamChatResponse(context, {
                conversation_id: "conv-created",
                user_message_id: 5,
            });

            expect(result).toEqual({
                valid: true,
                shouldUpdateConversation: true,
                targetConversationId: "conv-created",
            });
        });

        it("rejects streamChat response when user switched conversation while in-flight", () => {
            const context: StreamChatResponseValidationContext = {
                requestGeneration: 1,
                currentGeneration: 2, // switched to new conversation
                clientRequestId: "req-1",
                activeConversationId: "conv-2",
            };

            const result = validateStreamChatResponse(context, {
                conversation_id: "conv-1",
                user_message_id: 5,
            });

            expect(result).toEqual({
                valid: false,
                reason: "generation_mismatch",
            });
        });
    });

    describe("cross-turn and new-conversation race condition scenarios", () => {
        it("prevents late turn-start and late turn-finish from hijacking conversation when user switched to Conv B", () => {
            // Step 1: User is in Conversation A (Generation 1) and sends a message
            let currentGeneration = 1;
            let activeConversationId: string | null = "conv-A";
            const pendingRequest = {
                clientRequestId: "req-conv-A",
                generation: 1,
                conversationId: "conv-A",
                characterId: "char-1",
            };

            // Step 2: User switches to Conversation B before turn-start arrives
            currentGeneration = 2;
            activeConversationId = "conv-B";

            // Step 3: Late turn-start arrives from Conv A
            const turnStartResult = validateTurnStart({
                currentGeneration,
                activeConversationId,
                activeCharacterId: "char-1",
                pendingRequest,
                isCancelRequested: false,
            }, {
                turn_id: "turn-A-1",
                client_request_id: "req-conv-A",
                conversation_id: "conv-A",
                user_message_id: 42,
            });

            // Must be rejected with generation_mismatch
            expect(turnStartResult.valid).toBe(false);
            if (!turnStartResult.valid) {
                expect(turnStartResult.reason).toBe("generation_mismatch");
            }
            // activeConversationId remains Conv B!
            expect(activeConversationId).toBe("conv-B");

            // Step 4: Late turn-finish arrives from Conv A
            const turnFinishResult = validateTurnFinish({
                currentGeneration,
                activeConversationId,
                currentTurn: null, // Conv B has no active turn yet
            }, {
                turn_id: "turn-A-1",
                status: "completed",
                conversation_id: "conv-A",
                assistant_message_id: 43,
            });

            // Must be rejected with no_active_turn
            expect(turnFinishResult.valid).toBe(false);
            if (!turnFinishResult.valid) {
                expect(turnFinishResult.reason).toBe("no_active_turn");
            }
            // activeConversationId STILL remains Conv B!
            expect(activeConversationId).toBe("conv-B");
        });

        it("prevents late turn-finish from reverting newly created empty conversation back to old conversation", () => {
            // Step 1: Turn was started in Conv Old (Generation 1)
            let currentGeneration = 1;
            let activeConversationId: string | null = "conv-old";

            // Step 2: User clicks "New Conversation" -> Generation becomes 2, activeConversationId becomes null
            currentGeneration = 2;
            activeConversationId = null;

            // Step 3: Late turn-finish arrives from Conv Old
            const turnFinishResult = validateTurnFinish({
                currentGeneration,
                activeConversationId,
                currentTurn: null, // cleared upon clicking "New Conversation"
            }, {
                turn_id: "turn-old-1",
                status: "completed",
                conversation_id: "conv-old",
                assistant_message_id: 99,
            });

            // Must be rejected: no active turn in current generation
            expect(turnFinishResult.valid).toBe(false);
            // activeConversationId remains null (empty new conversation), NOT overridden back to conv-old!
            expect(activeConversationId).toBeNull();
        });
    });

    describe("reconcileTurnMessageIds", () => {
        it("reconciles user and assistant message ids matching clientRequestId when finish event was missed", () => {
            const initialMessages: ChatPanelMessage[] = [
                {
                    role: "user",
                    text: "Hello",
                    clientRequestId: "req-1",
                },
                {
                    role: "kokoro",
                    text: "Hi there!",
                    clientRequestId: "req-1",
                },
            ];

            const reconciled = reconcileTurnMessageIds(
                initialMessages,
                "req-1",
                101, // userMessageId from StreamChatResponse
                102, // assistantMessageId from StreamChatResponse
            );

            expect(reconciled[0].id).toBe(101);
            expect(reconciled[1].id).toBe(102);
        });

        it("is idempotent and leaves existing message ids intact", () => {
            const initialMessages: ChatPanelMessage[] = [
                {
                    id: 101,
                    role: "user",
                    text: "Hello",
                    clientRequestId: "req-1",
                },
                {
                    id: 102,
                    role: "kokoro",
                    text: "Hi there!",
                    clientRequestId: "req-1",
                },
            ];

            const reconciled = reconcileTurnMessageIds(
                initialMessages,
                "req-1",
                999,
                888,
            );

            // Returns original array reference when no changes are needed
            expect(reconciled).toBe(initialMessages);
            expect(reconciled[0].id).toBe(101);
            expect(reconciled[1].id).toBe(102);
        });

        it("reconciles assistant message id for regenerated turn using lastIndexOf fallback", () => {
            const initialMessages: ChatPanelMessage[] = [
                {
                    id: 50,
                    role: "user",
                    text: "Tell me a joke",
                },
                {
                    role: "kokoro",
                    text: "Why did the chicken cross the road?",
                },
            ];

            const reconciled = reconcileTurnMessageIds(
                initialMessages,
                "regen-req-1",
                null,
                52, // assistantMessageId
            );

            expect(reconciled[0].id).toBe(50);
            expect(reconciled[1].id).toBe(52);
            expect(reconciled[1].clientRequestId).toBe("regen-req-1");
        });

        it("returns original array when both user and assistant message ids are null/undefined", () => {
            const initialMessages: ChatPanelMessage[] = [
                {
                    role: "user",
                    text: "Test",
                },
            ];

            const reconciled = reconcileTurnMessageIds(
                initialMessages,
                "req-1",
                null,
                null,
            );

            expect(reconciled).toBe(initialMessages);
        });
    });
});

describe("isChatSessionCurrent", () => {
    it("returns true when both generation and conversation id are unchanged", () => {
        expect(isChatSessionCurrent(1, "conv-1", 1, "conv-1")).toBe(true);
    });

    it("returns true for null conversation ids when generation is unchanged", () => {
        expect(isChatSessionCurrent(2, null, 2, null)).toBe(true);
    });

    it("returns false when generation changed", () => {
        expect(isChatSessionCurrent(1, "conv-1", 2, "conv-1")).toBe(false);
    });

    it("returns false when conversation id changed", () => {
        expect(isChatSessionCurrent(1, "conv-1", 1, "conv-2")).toBe(false);
    });

    it("returns false when conversation id materialized from null", () => {
        expect(isChatSessionCurrent(1, null, 1, "conv-1")).toBe(false);
    });

    it("returns false when both generation and conversation id changed", () => {
        expect(isChatSessionCurrent(1, "conv-1", 2, "conv-2")).toBe(false);
    });
});
