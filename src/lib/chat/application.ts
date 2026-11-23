// Application commands and a per-subscriber read model. Rust remains authoritative.
// This module has no React, DOM, localStorage, or display-component dependency.
import * as bridge from "../kokoro-bridge";
import type { ChatRequest, ChatTurnSnapshot, ChatViewSnapshot, ChatViewVersion, StreamChatResponse } from "../kokoro-bridge";
import { canTruncateRefreshedHistory, getContinueFromCutoffIndex, getContinueFromRowBoundary, getExpectedTailMessageId } from "./history-commands";
import { findFailurePromptIndex, isActiveTurn, mergeTurnSnapshot, projectSavedMessages, turnMatchesView } from "./projection";

export interface ChatTransport {
  read(knownVersion?: ChatViewVersion): Promise<ChatViewSnapshot | null>;
  listen(callback: (turn: ChatTurnSnapshot) => void): Promise<() => void>;
  send(request: ChatRequest): Promise<StreamChatResponse>;
  cancel(id: string, reason?: string): Promise<void>;
}
export interface ChatMessageTarget {
  characterId?: string;
  conversationId: string;
  generation: number;
  messageId: number;
  historyRevision?: number | null;
}
export type EditMessageResult =
  | { status: "skipped" }
  | { status: "saved"; refresh?: { historyRevision: number | null; result: Promise<boolean> } };
const transport: ChatTransport = {
  read: known => known ? bridge.getChatView(known) : bridge.getChatView(),
  listen: callback => bridge.onChatRuntimeChanged(callback),
  send: request => bridge.streamChat(request), cancel: (id, reason) => bridge.cancelChatTurn(id, reason),
};
export const emptyChatView: ChatViewSnapshot = {
  revision: 0, history_revision: 0, character_id: "default", conversation_id: null,
  conversation_history_revision: null, generation: 0, busy: false, messages: [], turns: [],
};
const changedHistory = () => new Error("Conversation history changed before retry");
const MAX_ANNOUNCED_FAILURES = 256;

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
  private announcedFailures = new Set<string>();
  private mutation: Promise<unknown> | null = null;

  constructor(private readonly io: ChatTransport = transport) {}
  getSnapshot = () => this.view;
  claimFailureAnnouncement(requestId: string): boolean {
    if (this.announcedFailures.has(requestId)) return false;
    this.announcedFailures.add(requestId);
    // A stale read can omit a request whose command already reported an error.
    if (this.announcedFailures.size > MAX_ANNOUNCED_FAILURES) {
      const oldest = this.announcedFailures.values().next();
      if (!oldest.done) this.announcedFailures.delete(oldest.value);
    }
    return true;
  }
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
  private startRead(known?: ChatViewVersion, fresh = false): Promise<ChatViewSnapshot> {
    const conditional = known !== undefined;
    if (!fresh && this.readInFlight?.connection === this.connection && (conditional || !this.readInFlight.conditional)) {
      return this.readInFlight.promise;
    }
    const connection = this.connection;
    const promise = this.readView(connection, known).finally(() => {
      if (this.readInFlight?.promise === promise) this.readInFlight = undefined;
    });
    this.readInFlight = { connection, conditional, promise };
    return promise;
  }
  async refreshEditedHistory(): Promise<boolean> {
    const connection = this.connection;
    const version = this.readVersion;
    let read = this.startRead(undefined, true);
    for (;;) {
      await read.catch(() => {});
      if (connection !== this.connection || version !== this.readVersion) return true;
      // A newer read can supersede this one before it publishes its rows.
      const newer = this.readInFlight;
      if (newer?.connection !== connection || newer.promise === read) return false;
      read = newer.promise;
    }
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
    for (const [id, turn] of this.turns) {
      // An unresolved IPC result only protects an unregistered optimistic turn.
      // Import can retire a registered turn before that result reaches this view.
      const awaitingRegistration = turn.revision === 0 && this.pending.has(id);
      if (!ids.has(id) && !awaitingRegistration && turn.revision <= view.revision) this.turns.delete(id);
    }
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
        conversation_generation: request.conversation_generation ?? this.view.generation,
        message: request.message, images: request.images ?? [], hidden: request.hidden ?? false,
        reusing_saved_prompt: request.retry_failure !== undefined || request.regenerate_reply !== undefined,
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
    const target = id ?? this.view.turns.find(turn => isActiveTurn(turn) && turnMatchesView(this.view, turn))?.client_request_id;
    if (target) await this.io.cancel(target, reason);
    await this.refresh();
  }
  async retryPersistence(id: string): Promise<StreamChatResponse> {
    if (this.view.busy || this.mutation) throw new Error("history_mutation_busy");
    try {
      return await bridge.retryChatPersistence(id);
    } finally {
      void this.refresh().catch(() => {});
    }
  }
  approveTool(id: string) { return bridge.approveToolApproval(id); }
  rejectTool(id: string) { return bridge.rejectToolApproval(id, null); }
  async prepareCharacterSwitch(characterId: string) {
    void characterId;
    await this.refresh();
  }
  async selectConversation(id: string | null) {
    return this.mutate(async () => {
      if (id) await bridge.loadConversation(id); else await bridge.clearHistory();
    });
  }
  private beginMutation<T>(operation: () => Promise<T>): Promise<T> {
    if (this.mutation) throw new Error("history_mutation_busy");
    const result = operation();
    this.mutation = result;
    this.publish({ ...this.view, busy: true });
    return result;
  }
  private async mutate<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.beginMutation(operation);
    try { return await result; } finally {
      this.mutation = null;
      // History and selection changes do not advance the journal revision.
      // Start a new read so an older response cannot publish after the mutation.
      await this.startRead(undefined, true);
    }
  }
  async clear() {
    return this.mutate(async () => {
      await this.cancel(undefined, "clear_history");
      await bridge.clearHistory();
    });
  }
  async edit(target: ChatMessageTarget, content: string): Promise<EditMessageResult> {
    const before = this.view;
    if (!content.trim() || (target.characterId !== undefined && target.characterId !== before.character_id)
      || target.conversationId !== before.conversation_id || target.generation !== before.generation) return { status: "skipped" };

    const write = this.beginMutation(() => bridge.editConversationMessage({
      conversation_id: target.conversationId, conversation_generation: target.generation,
      message_id: target.messageId, new_content: content,
    }));
    let committed: bridge.EditConversationMessageResponse | undefined;
    let outcome: EditMessageResult = { status: "saved" };
    try {
      committed = await write;
    } finally {
      this.mutation = null;
      const current = this.view;
      const saved = committed;
      const original = before.messages.find(row => row.id === target.messageId);
      const currentRow = current.messages.find(row => row.id === target.messageId);
      const hasEditedRow = saved?.message_id === target.messageId
        && current.character_id === before.character_id
        && current.conversation_id === before.conversation_id && current.generation === before.generation
        && currentRow !== undefined;
      // An unrelated history update can advance the revision while the same row still needs its confirmed edit.
      const messages = hasEditedRow && currentRow.content === original?.content && currentRow.content !== saved.updated_content
        ? current.messages.map(row => row === currentRow ? { ...row, content: saved.updated_content } : row)
        : current.messages;
      const busy = this.readVersion?.busy === true || current.turns.some(isActiveTurn) || this.pending.size > 0;
      this.publish({ ...current, messages, busy });
      const result = this.refreshEditedHistory();
      if (hasEditedRow) {
        outcome = { status: "saved", refresh: { historyRevision: current.conversation_history_revision, result } };
      }
    }
    return outcome;
  }
  async deleteConversation(id: string) {
    // Rust resets selection only if the deleted conversation is still current.
    // A late response must never clear a selection made while deletion awaited IO.
    return this.mutate(() => bridge.deleteConversation(id));
  }
  async continueFrom(target: ChatMessageTarget, regenerate = false, options: Pick<ChatRequest, "allow_image_gen" | "client_request_id"> = {}) {
    const characterId = this.view.character_id;
    const matchesSelection = (view: ChatViewSnapshot) => view.character_id === characterId
      && view.conversation_id === target.conversationId && view.generation === target.generation;
    if (!matchesSelection(this.view)) return;

    const cached = this.view;
    const staleDeletion = () => new Error("Conversation history changed before deletion");
    if (target.historyRevision !== undefined && target.historyRevision !== cached.conversation_history_revision) {
      throw regenerate ? changedHistory() : staleDeletion();
    }
    const saved = projectSavedMessages(cached);
    const diagnosticIndex = saved.findIndex(message => message.id === target.messageId && message.failureScope);
    if (diagnosticIndex >= 0) {
      if (!regenerate) return;
      const prompt = saved[findFailurePromptIndex(cached, saved, diagnosticIndex)];
      const tail = getExpectedTailMessageId(saved);
      if (!prompt || tail === null || cached.conversation_history_revision === null) return;
      return this.send({
        ...options, message: prompt.text, images: prompt.images, regenerate: true,
        character_id: characterId, conversation_id: target.conversationId, conversation_generation: target.generation,
        retry_failure: {
          diagnostic_message_id: target.messageId, expected_tail_message_id: tail,
          expected_history_revision: cached.conversation_history_revision,
        },
      });
    }

    if (regenerate) {
      if (this.mutation) throw new Error("history_mutation_busy");
      // Check for newer writes without adopting them into the requested truncation.
      this.readInFlight = undefined;
      const view = await this.refresh();
      if (!matchesSelection(view)) return;
      const revision = cached.conversation_history_revision;
      if (revision !== view.conversation_history_revision) throw changedHistory();
      const index = saved.findIndex(message => message.id === target.messageId);
      const selected = saved[index];
      const tail = getExpectedTailMessageId(saved);
      if (!selected) throw changedHistory();
      if (selected.role !== "kokoro" || selected.failureScope || tail === null || revision === null) return;
      const prompt = saved.slice(0, index).reverse().find(message => message.role === "user");
      if (!prompt) return;
      return this.send({
        ...options, message: prompt.text, images: prompt.images, regenerate: true,
        character_id: characterId, conversation_id: target.conversationId, conversation_generation: target.generation,
        regenerate_reply: {
          assistant_message_id: target.messageId, expected_tail_message_id: tail, expected_history_revision: revision,
        },
      });
    }
    return this.mutate(async () => {
      // A turn can persist rows even if it is cancelled, fails, or its event is missed.
      // A read already in flight may have sampled history before those writes.
      this.readInFlight = undefined;
      const view = await this.refresh();
      if (!matchesSelection(view)) return;
      if (!canTruncateRefreshedHistory(cached, view)) throw staleDeletion();
      const messages = projectSavedMessages(view).filter(message => message.id !== undefined);
      const index = messages.findIndex(message => message.id === target.messageId);
      if (index < 0 || messages[index].failureScope) return;
      const cutoff = getContinueFromCutoffIndex(messages, index);
      if (cutoff === null) return;
      const keptId = messages[cutoff - 1]?.id;
      if (keptId === undefined || view.conversation_history_revision === null) return;
      const keepThroughId = getContinueFromRowBoundary(view, target.messageId, keptId);
      if (keepThroughId === null) throw new Error("history_turns_interleaved");
      if (view.messages.some(row => row.id !== undefined && row.id > keepThroughId)) {
        const count = messages.filter(message => message.id !== undefined && message.id > keepThroughId).length;
        await bridge.deleteLastMessages(count, target.conversationId, getExpectedTailMessageId(messages), {
          generation: target.generation, historyRevision: view.conversation_history_revision, keepThroughId,
        });
      }
    });
  }
}

// A cache/adapter in this WebView only; cross-window admission and idempotency are Rust-owned.
export const chatApplication = new ChatApplication();
export const submitChat = (request: ChatRequest) => chatApplication.send(request);
