import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatApplication, emptyChatView, type ChatTransport } from "../../lib/chat/application";
import { AudioStreamManager } from "../../lib/audio-player";
import type { ChatViewSnapshot } from "../../lib/kokoro-bridge";

const fakes = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => unknown>(),
  read: vi.fn(), loadConversation: vi.fn(), cancelTts: vi.fn().mockResolvedValue(undefined),
  finishChatPlayback: vi.fn().mockResolvedValue(undefined),
  player: null as unknown as AudioStreamManager,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => unknown) => {
    fakes.handlers.set(name, callback); return () => { fakes.handlers.delete(name); };
  }), emit: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", async () => {
  const { listen } = await import("@tauri-apps/api/event");
  return { getCurrentWindow: () => ({
    label: "main",
    listen: (name: string, callback: Parameters<typeof listen>[1]) =>
      listen(name, callback, { target: { kind: "Window", label: "main" } }),
  }) };
});
vi.mock("../services", () => ({ get audioPlayer() { return fakes.player; } }));
vi.mock("../../lib/kokoro-bridge", () => ({
  cancelTts: fakes.cancelTts, finishChatPlayback: fakes.finishChatPlayback,
  getLatestTtsRequestId: vi.fn(() => null), loadConversation: fakes.loadConversation,
}));
vi.mock("./voice-interrupt-service", () => ({
  VoiceInterruptService: class { start = vi.fn().mockResolvedValue(undefined); stop = vi.fn(); onInterrupt() { return () => {}; } },
}));
const scope = { client_request_id: "request-a", turn_id: "turn-a", character_id: "role-a", conversation_id: "conversation-a", selection_generation: 1 };
const emitEvent = (name: string, payload: object) => fakes.handlers.get(name)!({ payload });
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

describe("automatic speech follows the selected view", () => {
  let chats: ChatApplication;
  let service: import("./tts-service").TtsService;
  let view: ChatViewSnapshot;
  let spoken: string[];
  let sources: Array<{ start: ReturnType<typeof vi.fn>; stop: ReturnType<typeof vi.fn> }>;
  let decode: ReturnType<typeof vi.fn>;
  let browserCancel: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    vi.clearAllMocks(); fakes.handlers.clear(); spoken = []; sources = [];
    browserCancel = vi.fn(); decode = vi.fn().mockResolvedValue({ duration: 1 });
    vi.stubGlobal("window", { speechSynthesis: { cancel: browserCancel, speak: (utterance: { text: string }) => spoken.push(utterance.text), getVoices: () => [] } });
    vi.stubGlobal("SpeechSynthesisUtterance", class { constructor(public text: string) {} });
    vi.stubGlobal("localStorage", { getItem: () => "false" });
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1)); vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.stubGlobal("Audio", class { pause = vi.fn(); load = vi.fn(); removeAttribute = vi.fn(); });
    vi.stubGlobal("AudioContext", class {
      state = "running"; currentTime = 0; destination = {};
      createAnalyser() { return { connect: vi.fn(), frequencyBinCount: 128, getByteTimeDomainData: vi.fn(), getByteFrequencyData: vi.fn() }; }
      createMediaElementSource() { return { connect: vi.fn() }; }
      createBufferSource() { const source = { start: vi.fn(), stop: vi.fn(), connect: vi.fn(), disconnect: vi.fn() }; sources.push(source); return source; }
      decodeAudioData = decode;
    });
    fakes.player = new AudioStreamManager();
    view = { ...emptyChatView, character_id: "role-a", conversation_id: "conversation-a", generation: 1 };
    fakes.read.mockImplementation(async () => structuredClone(view));
    const io: ChatTransport = { read: () => fakes.read(), listen: async () => () => {}, send: vi.fn(), cancel: vi.fn() };
    chats = new ChatApplication(io); await chats.refresh();
    fakes.loadConversation.mockImplementation(async (id: string) => { view = { ...view, conversation_id: id, generation: view.generation + 1 }; });
    const { TtsService } = await import("./tts-service");
    service = new TtsService(chats); await service.init();
  });
  afterEach(() => { service.cleanup(); chats.disconnect(); vi.unstubAllGlobals(); });
  async function start() {
    await emitEvent("tts:start", { request_id: "chat_turn-a", text: "A reply", chat_scope: scope });
    await settle();
  }
  const browser = () => emitEvent("tts:browser-delegate", { request_id: "chat_turn-a", text: "A reply" });

  it.each(["conversation", "character"])("rejects completion after a %s selection change", async kind => {
    if (kind === "conversation") await chats.selectConversation("conversation-b");
    else { view = { ...view, character_id: "role-b", conversation_id: "conversation-b", generation: 2 }; await chats.refresh(); }
    await start(); await browser(); await settle();
    expect(spoken).toEqual([]); expect(fakes.cancelTts).toHaveBeenCalledWith("chat_turn-a");
  });
  it("plays selected browser speech and decoded audio normally", async () => {
    await start(); await browser();
    await emitEvent("tts:audio", { request_id: "chat_turn-a", data: [1, 2, 3], format: { codec: "wav" } }); await settle();
    expect(spoken).toEqual(["A reply"]); expect(sources).toHaveLength(1); expect(sources[0].start).toHaveBeenCalledOnce();
  });
  it("revokes queued browser speech and late chunks after A to B to A", async () => {
    await start(); await browser(); const cancels = browserCancel.mock.calls.length;
    await chats.selectConversation("conversation-b"); await chats.selectConversation("conversation-a");
    await browser(); await emitEvent("tts:audio", { request_id: "chat_turn-a", data: [1], format: { codec: "wav" } }); await settle();
    expect(spoken).toEqual(["A reply"]); expect(browserCancel.mock.calls.length).toBeGreaterThan(cancels); expect(decode).not.toHaveBeenCalled();
  });
  it("invalidates an actual audio decode still pending when selection changes", async () => {
    let release!: (buffer: { duration: number }) => void;
    decode.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    await start(); void emitEvent("tts:audio", { request_id: "chat_turn-a", data: [1], format: { codec: "wav" } });
    await vi.waitFor(() => expect(decode).toHaveBeenCalledOnce());
    await chats.selectConversation("conversation-b"); release({ duration: 1 }); await settle();
    expect(sources).toHaveLength(0); expect(fakes.cancelTts).toHaveBeenCalledWith("chat_turn-a");
  });
  it.each(["manual", "automatic"])("rejects a late scoped start without replacing B %s playback", async kind => {
    await chats.selectConversation("conversation-b");
    const chat_scope = kind === "automatic" ? { ...scope, client_request_id: "request-b", turn_id: "turn-b", conversation_id: "conversation-b", selection_generation: 2 } : undefined;
    await emitEvent("tts:start", { request_id: "manual-b", text: "B preview", chat_scope });
    await emitEvent("tts:browser-delegate", { request_id: "manual-b", text: "B preview" });
    const cancels = browserCancel.mock.calls.length;
    await start();
    await emitEvent("tts:browser-delegate", { request_id: "manual-b", text: "B continues" });
    expect(spoken).toEqual(["B preview", "B continues"]);
    expect(browserCancel.mock.calls.length).toBe(cancels);
    expect(fakes.finishChatPlayback).not.toHaveBeenCalledWith("manual-b");
  });
  it("drops the first delegate when selection changed during synthesis", async () => {
    await start(); await chats.selectConversation("conversation-b"); await browser();
    expect(spoken).toEqual([]); expect(fakes.cancelTts).toHaveBeenCalledWith("chat_turn-a");
  });
  it("waits for pending start validation before consuming the first browser delegate", async () => {
    let release!: (view: ChatViewSnapshot) => void;
    fakes.read.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const starting = emitEvent("tts:start", { request_id: "chat_turn-a", text: "A reply", chat_scope: scope });
    const media = browser(); expect(spoken).toEqual([]);
    release(structuredClone(view)); await starting; await media;
    expect(spoken).toEqual(["A reply"]);
  });
  it("retains an immediate provider error while scoped start validation is pending", async () => {
    let release!: (view: ChatViewSnapshot) => void;
    const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
    fakes.read.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const starting = emitEvent("tts:start", { request_id: "chat_turn-a", text: "A reply", chat_scope: scope });
    const error = emitEvent("tts:error", { request_id: "chat_turn-a", error: "Provider unavailable" });
    expect(warning).not.toHaveBeenCalled(); release(structuredClone(view)); await starting; await error;
    expect(warning).toHaveBeenCalledWith("[TTS] Synthesis request failed:", "Provider unavailable"); warning.mockRestore();
  });
  it("bounds audio waiting for start validation without interrupting current manual playback", async () => {
    await emitEvent("tts:start", { request_id: "manual-b", text: "B preview" });
    await emitEvent("tts:browser-delegate", { request_id: "manual-b", text: "B preview" });
    const cancels = browserCancel.mock.calls.length;
    let release!: (view: ChatViewSnapshot) => void;
    fakes.read.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const starting = emitEvent("tts:start", { request_id: "chat_turn-a", text: "A reply", chat_scope: scope });
    const first = emitEvent("tts:audio", { request_id: "chat_turn-a", data: new Array(600_000).fill(0), format: { codec: "wav" } });
    await emitEvent("tts:audio", { request_id: "chat_turn-a", data: new Array(600_000).fill(0), format: { codec: "wav" } });
    expect(fakes.cancelTts).toHaveBeenCalledWith("chat_turn-a");
    release(structuredClone(view)); await starting; await first; await settle();
    expect(decode).not.toHaveBeenCalled(); expect(browserCancel.mock.calls.length).toBe(cancels);
    await emitEvent("tts:browser-delegate", { request_id: "manual-b", text: "B continues" });
    expect(spoken).toEqual(["B preview", "B continues"]);
  });
  it("cleanup releases a start waiting for its view read and never plays it later", async () => {
    let release!: (view: ChatViewSnapshot) => void;
    fakes.read.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const starting = emitEvent("tts:start", { request_id: "chat_turn-a", text: "A reply", chat_scope: scope });
    service.cleanup();
    expect(fakes.cancelTts).toHaveBeenCalledWith("chat_turn-a");
    expect(fakes.finishChatPlayback).toHaveBeenCalledWith("chat_turn-a");
    release(structuredClone(view)); await starting;
    expect(spoken).toEqual([]);
  });
  it("leaves explicit manual speech independent of conversation selection", async () => {
    await emitEvent("tts:start", { request_id: "manual-preview", text: "Preview" });
    await chats.selectConversation("conversation-b");
    await emitEvent("tts:browser-delegate", { request_id: "manual-preview", text: "Preview" });
    expect(spoken).toEqual(["Preview"]); expect(fakes.cancelTts).not.toHaveBeenCalled();
  });
});
