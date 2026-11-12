// Application commands and a per-subscriber read model. Rust remains authoritative.
// This module has no React, DOM, localStorage, or display-component dependency.
import * as bridge from "../kokoro-bridge";
import type { ChatRequest, ChatTurnSnapshot, ChatViewSnapshot, ChatViewVersion, StreamChatResponse } from "../kokoro-bridge";
import { getContinueFromCutoffIndex, getExpectedTailMessageId } from "./history-commands";
import { isActiveTurn, isTurnAwaitingHistory, mergeTurnSnapshot, projectSavedMessages } from "./projection";

export interface ChatTransport {
  read(knownVersion?: ChatViewVersion): Promise<ChatViewSnapshot | null>;
  listen(callback: (turn: ChatTurnSnapshot) => void): Promise<() => void>;
  send(request: ChatRequest): Promise<StreamChatResponse>;
  cancel(id: string, reason?: string): Promise<void>;
}
export interface ChatMessageTarget {
  conversationId: string;
  generation: number;
  messageId: number;
}
const transport: ChatTransport = {
  read: known => known ? bridge.getChatView(known) : bridge.getChatView(),
  listen: callback => bridge.onChatRuntimeChanged(callback),
  send: request => bridge.streamChat(request), cancel: (id, reason) => bridge.cancelChatTurn(id, reason),
};
export const emptyChatView: ChatViewSnapshot = {
  revision: 0, history_revision: 0, character_id: "default", conversation_id: null, generation: 0, busy: false, messages: [], turns: [],
};

export class ChatApplication {
  private view = emptyChatView;
  private turns = new Map<string, ChatTurnSnapshot>();
  private listeners = new Set<() => void>();
  private stop: (() => void) | undefined;
  private timer: ReturnType<typeof setInterval> | undefined;
  private connection = 0;
  private readSequence = 0;
  private readInFlight: { connection: number; conditional: boolean; promise: Promise<ChatViewSnapshot> } | undefined;
  private readVersion: ChatViewVersion | undefined;
  private historyEventRevision = 0;
  private restoredRevision = 0;
  private pending = new Map<string, { fingerprint: string; promise: Promise<StreamChatResponse> }>();
  private mutation: Promise<unknown> | null = null;

  constructor(private readonly io: ChatTransport = transport) {}
  getSnapshot = () => this.view;
  subscribe = (callback: () => void) => {
    this.listeners.add(callback);
    if (this.listeners.size === 1) void this.connect();
    return () => {
      this.listeners.delete(callback);
      if (!this.listeners.size) this.disconnect();
    };
  };
  private publish(view: ChatViewSnapshot) {
    this.view = view;
    for (const listener of this.listeners) listener();
  }
  private async connect() {
    const generation = ++this.connection;
    try {
      const stop = await this.io.listen(turn => {
        if (generation !== this.connection || turn.revision <= this.restoredRevision) return;
        const previous = this.turns.get(turn.client_request_id);
        if (previous && turn.revision <= previous.revision) return;
        // Deltas already contain the full streaming projection. Read SQLite only
        // for identity/status changes and periodic missed-event reconciliation.
        const historyChanged = !previous || previous.status !== turn.status
          || previous.character_id !== turn.character_id || previous.conversation_id !== turn.conversation_id
          || previous.user_message_id !== turn.user_message_id || previous.assistant_message_id !== turn.assistant_message_id;
        if (historyChanged) this.historyEventRevision = Math.max(this.historyEventRevision, turn.revision);
        this.turns = mergeTurnSnapshot(this.turns, turn);
        const turns = [...this.turns.values()];
        this.publish({ ...this.view, turns, busy: turns.some(isActiveTurn) || this.mutation !== null });
        if (historyChanged) void this.refresh().catch(() => {});
      });
      if (generation !== this.connection) { stop(); return; }
      this.stop = stop;
      // Subscribe before reading so newer streaming events can be merged into the snapshot.
      await this.refresh();
    } catch {
      // Polling also recovers if the listener or the initial read failed.
    } finally {
      if (generation === this.connection) {
        this.timer = setInterval(() => { void this.startRead(this.readVersion).catch(() => {}); }, 750);
      }
    }
  }
  disconnect() {
    ++this.connection;
    this.stop?.(); this.stop = undefined;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.readVersion = undefined;
    // Detaching a view never cancels a business request.
  }
  refresh(): Promise<ChatViewSnapshot> {
    return this.startRead();
  }
  private startRead(known?: ChatViewVersion): Promise<ChatViewSnapshot> {
    const conditional = known !== undefined;
    if (this.readInFlight?.connection === this.connection && (conditional || !this.readInFlight.conditional)) {
      return this.readInFlight.promise;
    }
    const connection = this.connection;
    const promise = this.readView(connection, known).finally(() => {
      if (this.readInFlight?.promise === promise) this.readInFlight = undefined;
    });
    this.readInFlight = { connection, conditional, promise };
    return promise;
  }
  private async readView(connection: number, known?: ChatViewVersion): Promise<ChatViewSnapshot> {
    const sequence = ++this.readSequence;
    const historyEventAtStart = this.historyEventRevision;
    const view = await this.io.read(known);
    if (sequence !== this.readSequence || connection !== this.connection || !view) return this.view;
    // Streaming deltas merge below. Only a newer history-changing event can
    // invalidate the SQLite rows that were read alongside the journal.
    if (this.historyEventRevision > historyEventAtStart && this.historyEventRevision > view.revision) {
      return this.readView(connection);
    }
    const ids = new Set(view.turns.map(turn => turn.client_request_id));
    for (const [id, turn] of this.turns) if (!ids.has(id) && !this.pending.has(id) && turn.revision <= view.revision) this.turns.delete(id);
    for (const turn of view.turns) this.turns = mergeTurnSnapshot(this.turns, turn);
    this.restoredRevision = Math.max(this.restoredRevision, view.revision);
    const { revision, history_revision, generation } = view;
    this.readVersion = { revision, history_revision, generation, busy: view.busy };
    const turns = [...this.turns.values()];
    const busy = view.busy || turns.some(isActiveTurn) || [...this.pending.keys()].some(id => !this.turns.has(id)) || this.mutation !== null;
    this.publish({ ...view, busy, turns });
    return this.view;
  }
  send(request: ChatRequest): Promise<StreamChatResponse> {
    if (this.mutation) return Promise.reject(new Error("history_mutation_busy"));
    const id = request.client_request_id ?? `req_${crypto.randomUUID()}`;
    const existing = this.pending.get(id);
    const fingerprint = JSON.stringify(request);
    if (existing) return existing.fingerprint === fingerprint ? existing.promise : this.io.send({ ...request, client_request_id: id });
    if (!existing) {
      this.turns.set(id, {
        revision: 0, client_request_id: id, turn_id: null,
        character_id: request.character_id ?? this.view.character_id,
        conversation_id: request.conversation_id ?? this.view.conversation_id,
        message: request.message, images: request.images ?? [], hidden: request.hidden ?? false,
        status: "pending", text: "", translation: null, tools: [], error: null,
        user_message_id: null, assistant_message_id: null,
      });
      this.publish({ ...this.view, busy: true, turns: [...this.turns.values()] });
    }
    const promise = this.io.send({ ...request, client_request_id: id }).finally(() => {
      this.pending.delete(id);
      if (this.turns.get(id)?.revision === 0) this.turns.delete(id);
      this.publish({ ...this.view, busy: [...this.turns.values()].some(isActiveTurn) || this.mutation !== null, turns: [...this.turns.values()] });
      void this.refresh().catch(() => {});
    });
    this.pending.set(id, { fingerprint, promise });
    return promise;
  }
  async cancel(id?: string, reason = "stopped_by_user") {
    const target = id ?? this.view.turns.find(isActiveTurn)?.client_request_id ?? this.pending.keys().next().value;
    if (target) await this.io.cancel(target, reason);
    await this.refresh();
  }
  approveTool(id: string) { return bridge.approveToolApproval(id); }
  rejectTool(id: string) { return bridge.rejectToolApproval(id, null); }
  async prepareCharacterSwitch(characterId: string) {
    const view = await this.refresh();
    if (view.character_id !== characterId) await this.cancel(undefined, "character_switched");
  }
  async selectConversation(id: string | null) {
    return this.mutate(async () => {
      await this.cancel(undefined, "conversation_switched");
      if (id) await bridge.loadConversation(id); else await bridge.clearHistory();
    });
  }
  private async mutate<T>(operation: () => Promise<T>): Promise<T> {
    if (this.mutation) throw new Error("history_mutation_busy");
    const result = operation();
    this.mutation = result;
    this.publish({ ...this.view, busy: true });
    try { return await result; } finally {
      this.mutation = null;
      // History and selection changes do not advance the journal revision.
      // Start a new read so an older response cannot publish after the mutation.
      this.readInFlight = undefined;
      await this.refresh();
    }
  }
  async clear() {
    return this.mutate(async () => {
      await this.cancel(undefined, "clear_history");
      await bridge.clearHistory();
    });
  }
  async edit(target: ChatMessageTarget, content: string) {
    const text = content.trim();
    if (!text || target.conversationId !== this.view.conversation_id || target.generation !== this.view.generation) return;
    return this.mutate(() => bridge.editConversationMessage({
      conversation_id: target.conversationId, conversation_generation: target.generation,
      message_id: target.messageId, new_content: text,
    }));
  }
  async deleteConversation(id: string) {
    // Rust resets selection only if the deleted conversation is still current.
    // A late response must never clear a selection made while deletion awaited IO.
    return this.mutate(() => bridge.deleteConversation(id));
  }
  async continueFrom(target: ChatMessageTarget, regenerate = false, options: { allow_image_gen?: boolean } = {}) {
    const characterId = this.view.character_id;
    const matchesSelection = (view: ChatViewSnapshot) => view.character_id === characterId
      && view.conversation_id === target.conversationId && view.generation === target.generation;
    if (!matchesSelection(this.view)) return;

    const user = await this.mutate(async () => {
      const awaitingHistory = (view: ChatViewSnapshot) => view.turns.some(turn => isTurnAwaitingHistory(view, turn));
      if (awaitingHistory(this.view)) await this.refresh();
      const view = this.view;
      if (!matchesSelection(view) || awaitingHistory(view)) return;
      const messages = projectSavedMessages(view).filter(message => message.id !== undefined);
      const index = messages.findIndex(message => message.id === target.messageId);
      if (index < 0) return;
      const cutoff = regenerate ? index : getContinueFromCutoffIndex(messages, index);
      if (cutoff === null) return;
      const user = messages.slice(0, index).reverse().find(message => message.role === "user");
      if (regenerate && !user) return;
      if (messages.length > cutoff) {
        await bridge.deleteLastMessages(messages.length - cutoff, target.conversationId, getExpectedTailMessageId(messages));
      }
      return user;
    });
    // A history command must never send into a selection made while it awaited IO.
    if (regenerate && user && matchesSelection(this.view)) {
      return this.send({ message: user.text, images: user.images, regenerate: true, character_id: characterId, conversation_id: target.conversationId, conversation_generation: target.generation, ...options });
    }
  }
}

// A cache/adapter in this WebView only; cross-window admission and idempotency are Rust-owned.
export const chatApplication = new ChatApplication();
export const submitChat = (request: ChatRequest) => chatApplication.send(request);
