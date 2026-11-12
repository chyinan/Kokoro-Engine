// Application commands and a per-subscriber read model. Rust remains authoritative.
// This module has no React, DOM, localStorage, or display-component dependency.
import * as bridge from "../kokoro-bridge";
import type { ChatRequest, ChatTurnSnapshot, ChatViewSnapshot, ChatViewVersion, StreamChatResponse } from "../kokoro-bridge";
import { getContinueFromCutoffIndex, getExpectedTailMessageId } from "./history-commands";
import { isActiveTurn, mergeTurnSnapshot, projectMessages } from "./projection";

export interface ChatTransport {
  read(knownVersion?: ChatViewVersion): Promise<ChatViewSnapshot | null>;
  listen(callback: (turn: ChatTurnSnapshot) => void): Promise<() => void>;
  send(request: ChatRequest): Promise<StreamChatResponse>;
  cancel(id: string, reason?: string): Promise<void>;
}
export interface ChatEditTarget {
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
  private eventSequence = 0;
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
        this.eventSequence++;
        this.turns = mergeTurnSnapshot(this.turns, turn);
        this.publish({ ...this.view, turns: [...this.turns.values()], busy: [...this.turns.values()].some(isActiveTurn) || this.mutation !== null });
        // Deltas already contain the full streaming projection. Read SQLite only
        // for identity/status changes and periodic missed-event reconciliation.
        if (!previous || previous.status !== turn.status || previous.conversation_id !== turn.conversation_id
          || previous.user_message_id !== turn.user_message_id || previous.assistant_message_id !== turn.assistant_message_id) {
          void this.refresh().catch(() => {});
        }
      });
      if (generation !== this.connection) { stop(); return; }
      this.stop = stop;
      // Subscribe before reading. A raced event makes refresh retry instead of
      // allowing an older history snapshot to overwrite the committed outcome.
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
    const eventsAtStart = this.eventSequence;
    const view = await this.io.read(known);
    if (sequence !== this.readSequence || connection !== this.connection || !view) return this.view;
    if (eventsAtStart !== this.eventSequence && [...this.turns.values()].some(turn => turn.revision > view.revision)) return this.readView(connection);
    const ids = new Set(view.turns.map(turn => turn.client_request_id));
    for (const [id, turn] of this.turns) if (!ids.has(id) && !this.pending.has(id) && turn.revision <= view.revision) this.turns.delete(id);
    for (const turn of view.turns) this.turns = mergeTurnSnapshot(this.turns, turn);
    this.restoredRevision = Math.max(this.restoredRevision, view.revision);
    const { revision, history_revision, generation, busy } = view;
    this.readVersion = { revision, history_revision, generation, busy };
    this.publish({ ...view, busy: view.busy || [...this.pending.keys()].some(id => { const turn = this.turns.get(id); return !turn || isActiveTurn(turn); }) || this.mutation !== null, turns: [...this.turns.values()] });
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
  private async mutate(operation: () => Promise<unknown>) {
    if (this.mutation) throw new Error("history_mutation_busy");
    this.mutation = operation();
    this.publish({ ...this.view, busy: true });
    try { await this.mutation; } finally {
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
  async edit(target: ChatEditTarget, content: string) {
    const text = content.trim();
    if (!text || target.conversationId !== this.view.conversation_id || target.generation !== this.view.generation) return;
    return this.mutate(() => bridge.editConversationMessage({
      conversation_id: target.conversationId, message_id: target.messageId, new_content: text,
    }));
  }
  async deleteConversation(id: string) {
    // Rust resets selection only if the deleted conversation is still current.
    // A late response must never clear a selection made while deletion awaited IO.
    return this.mutate(() => bridge.deleteConversation(id));
  }
  async continueFrom(index: number, regenerate = false, options: { allow_image_gen?: boolean } = {}) {
    const view = this.view;
    const messages = projectMessages(view);
    const cutoff = regenerate ? index : getContinueFromCutoffIndex(messages, index);
    if (cutoff === null || !view.conversation_id) return;
    const user = messages.slice(0, index).reverse().find(message => message.role === "user");
    if (regenerate && !user) return;
    await this.mutate(async () => {
      if (messages.length > cutoff) await bridge.deleteLastMessages(messages.length - cutoff, view.conversation_id, getExpectedTailMessageId(messages));
    });
    // A history command must never send into a selection made while it awaited IO.
    if (regenerate && user && this.view.conversation_id === view.conversation_id && this.view.character_id === view.character_id && this.view.generation === view.generation) {
      return this.send({ message: user.text, images: user.images, regenerate: true, character_id: view.character_id, conversation_id: view.conversation_id, conversation_generation: view.generation, ...options });
    }
  }
}

// A cache/adapter in this WebView only; cross-window admission and idempotency are Rust-owned.
export const chatApplication = new ChatApplication();
export const submitChat = (request: ChatRequest) => chatApplication.send(request);
