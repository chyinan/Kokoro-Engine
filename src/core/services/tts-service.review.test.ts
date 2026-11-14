// pattern: Imperative Shell
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fakes = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => unknown>(),
  playStateCallback: null as ((playing: boolean) => void) | null,
  playbackErrorCallback: null as ((error: string) => void) | null,
  playing: false,
  pendingMedia: false,
  finishChatPlayback: vi.fn().mockResolvedValue(undefined),
  cancelTts: vi.fn().mockResolvedValue(undefined),
  player: {
    clearQueue: vi.fn(), queueAudio: vi.fn(), finishStream: vi.fn(), stop: vi.fn(),
    onPlaybackError: vi.fn((callback: (error: string) => void) => {
      fakes.playbackErrorCallback = callback;
      return () => { fakes.playbackErrorCallback = null; };
    }),
    onPlayStateChange: vi.fn((callback: (playing: boolean) => void) => {
      fakes.playStateCallback = callback;
      return () => { fakes.playStateCallback = null; };
    }),
    get isPlaying() { return fakes.playing; },
    get hasPendingMediaPlayback() { return fakes.pendingMedia; },
  },
  interrupt: { callback: null as (() => void) | null },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => unknown) => {
    fakes.handlers.set(name, callback);
    return () => { fakes.handlers.delete(name); };
  }),
  emit: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "pet" }) }));
vi.mock("../services", () => ({ audioPlayer: fakes.player }));
vi.mock("../../lib/kokoro-bridge", () => ({
  cancelTts: fakes.cancelTts,
  finishChatPlayback: fakes.finishChatPlayback,
  getLatestTtsRequestId: vi.fn(() => null),
}));
vi.mock("./voice-interrupt-service", () => ({
  VoiceInterruptService: class {
    start = vi.fn().mockResolvedValue(undefined);
    stop = vi.fn();
    onInterrupt(callback: () => void) { fakes.interrupt.callback = callback; return () => {}; }
  },
}));

describe("review_r09 TTS event lifecycle", () => {
  let service: import("./tts-service").TtsService;
  let speechQueue: Array<string>;
  let utterances: Array<{ text: string; onerror: (event: { error: string }) => void }>;

  beforeEach(async () => {
    vi.resetModules();
    vi.clearAllMocks();
    fakes.handlers.clear();
    fakes.playStateCallback = null;
    fakes.playing = false;
    fakes.pendingMedia = false;
    speechQueue = [];
    utterances = [];
    vi.stubGlobal("window", { speechSynthesis: {
      cancel: () => { speechQueue = []; },
      speak: (utterance: typeof utterances[number]) => { speechQueue.push(utterance.text); utterances.push(utterance); },
      getVoices: () => [],
    } });
    vi.stubGlobal("SpeechSynthesisUtterance", class { constructor(public text: string) {} });
    vi.stubGlobal("localStorage", { getItem: (key: string) => key === "kokoro_voice_interrupt" ? "true" : "false" });
    const module = await import("./tts-service");
    service = new module.TtsService();
    await service.init();
  });

  afterEach(() => { service.cleanup(); vi.unstubAllGlobals(); });

  it("subscribes only to its physical window's playback events", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    expect(vi.mocked(listen).mock.calls).toHaveLength(5);
    for (const call of vi.mocked(listen).mock.calls) expect(call[2]).toEqual({ target: "pet" });
  });

  it("keeps earlier browser sentences queued until playback finishes", () => {
    fakes.handlers.get("tts:start")?.({ payload: { text: "First. Second." } });
    fakes.handlers.get("tts:browser-delegate")?.({ payload: { text: "First." } });
    fakes.handlers.get("tts:browser-delegate")?.({ payload: { text: "Second." } });
    expect(speechQueue).toEqual(["First.", "Second."]);
  });

  it("does not replay late synthesis audio after voice interruption", async () => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "req-interrupt", text: "Still generating." } });
    fakes.playStateCallback?.(true);
    fakes.interrupt.callback?.();
    expect(fakes.player.stop).toHaveBeenCalledOnce();

    await fakes.handlers.get("tts:audio")?.({ payload: { request_id: "req-interrupt", data: [1, 2, 3] } });

    expect(fakes.player.queueAudio).not.toHaveBeenCalled();
  });

  it("ignores late audio and end events from a superseded request", () => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "req-old", text: "Old request." } });
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "req-new", text: "New request." } });

    fakes.handlers.get("tts:audio")?.({ payload: { request_id: "req-old", data: [1, 2] } });
    fakes.handlers.get("tts:end")?.({ payload: { request_id: "req-old", text: "Old request.", status: "completed" } });
    fakes.handlers.get("tts:end")?.({ payload: { request_id: "req-old", text: "Old request.", status: "cancelled" } });

    expect(fakes.player.queueAudio).not.toHaveBeenCalled();
    expect(fakes.player.finishStream).not.toHaveBeenCalled();
    expect(fakes.player.stop).not.toHaveBeenCalled();
  });

  it("stops on cancellation before a pending audio failure can be reported", async () => {
    const { emit } = await import("@tauri-apps/api/event");
    const requestId = "chat_cancelled";
    let rejectAudio!: (error: Error) => void;
    fakes.player.queueAudio.mockImplementationOnce(() => new Promise<void>((_resolve, reject) => {
      rejectAudio = reject;
    }));
    fakes.handlers.get("tts:start")?.({ payload: { request_id: requestId } });
    fakes.handlers.get("tts:audio")?.({ payload: { request_id: requestId, data: [1, 2, 3] } });
    await Promise.resolve();
    expect(fakes.player.queueAudio).toHaveBeenCalledOnce();

    const cancelled = fakes.handlers.get("tts:end")?.({ payload: { request_id: requestId, status: "cancelled" } });
    // The replacement has not started and the old audio is still pending.
    const stoppedImmediately = fakes.player.stop.mock.calls.length;
    rejectAudio(new Error("Old audio failed after cancellation"));
    await cancelled;
    await new Promise<void>(resolve => setImmediate(resolve));

    expect(emit).not.toHaveBeenCalled();
    expect(fakes.finishChatPlayback).toHaveBeenCalledExactlyOnceWith(requestId);
    expect(stoppedImmediately).toBe(1);
    expect(fakes.player.stop).toHaveBeenCalledOnce();
    expect(fakes.player.finishStream).not.toHaveBeenCalled();
  });

  it("keeps the request active after server completion until local playback drains", async () => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "req-drain", text: "Playing." } });
    fakes.playing = true;
    const ended = fakes.handlers.get("tts:end")?.({
      payload: { request_id: "req-drain", text: "Playing.", status: "completed" },
    });
    await Promise.resolve(ended);

    expect(fakes.player.finishStream).toHaveBeenCalledOnce();
    expect((service as unknown as { isStreamActive: boolean }).isStreamActive).toBe(true);

    await fakes.handlers.get("tts:audio")?.({
      payload: { request_id: "req-drain", data: [1, 2], format: { codec: "pcm_s16_le", sample_rate: 24000, channels: 1 } },
    });
    await Promise.resolve();
    expect(fakes.player.queueAudio).toHaveBeenCalledOnce();

    fakes.playing = false;
    fakes.playStateCallback?.(false);
    expect((service as unknown as { isStreamActive: boolean }).isStreamActive).toBe(false);
  });

  it("stops local playback and cancels the matching backend request", async () => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "req-stop", text: "Stop me." } });

    await service.stopPlayback("req-stop");

    expect(fakes.player.stop).toHaveBeenCalledOnce();
    expect(fakes.cancelTts).toHaveBeenCalledWith("req-stop");
    expect(fakes.finishChatPlayback).toHaveBeenCalledWith("req-stop");
  });

  it.each(["decode callback", "queued audio rejection", "media startup after synthesis", "browser speech error"])(
    "reports %s for the active chat playback and stops it once", async source => {
      const { emit } = await import("@tauri-apps/api/event");
      const requestId = "chat_failed";
      const error = source === "browser speech error" ? "Browser speech playback failed: audio-hardware" : "Controlled playback failure";
      fakes.handlers.get("tts:start")?.({ payload: { request_id: requestId } });
      if (source === "media startup after synthesis") {
        fakes.pendingMedia = true;
        await fakes.handlers.get("tts:end")?.({ payload: { request_id: requestId, status: "completed" } });
        expect(fakes.finishChatPlayback).not.toHaveBeenCalled();
      }
      if (source === "queued audio rejection") {
        fakes.player.queueAudio.mockRejectedValueOnce(new Error(error));
        fakes.handlers.get("tts:audio")?.({ payload: { request_id: requestId, data: [1, 2, 3] } });
      } else if (source === "browser speech error") {
        fakes.handlers.get("tts:browser-delegate")?.({ payload: { request_id: requestId, text: "Reply" } });
        utterances[0].onerror({ error: "audio-hardware" });
      } else {
        fakes.playbackErrorCallback?.(error);
      }
      await vi.waitFor(() => expect(fakes.finishChatPlayback).toHaveBeenCalledExactlyOnceWith(requestId, error));
      fakes.playbackErrorCallback?.("Second error after stopping");
      expect(emit).toHaveBeenCalledExactlyOnceWith("tts:playback-error", { request_id: requestId, error });
      expect(fakes.cancelTts).toHaveBeenCalledExactlyOnceWith(requestId);
      expect(fakes.player.stop).toHaveBeenCalledOnce();
    },
  );

  it("ignores browser errors delivered after an utterance is replaced", async () => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "chat_old" } });
    fakes.handlers.get("tts:browser-delegate")?.({ payload: { request_id: "chat_old", text: "Old reply" } });
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "chat_new" } });
    utterances[0].onerror({ error: "interrupted" });
    expect(fakes.finishChatPlayback).toHaveBeenCalledExactlyOnceWith("chat_old");
    expect(fakes.cancelTts).not.toHaveBeenCalled();
  });

  it.each(["cleanup", "interrupt", "replacement"])("releases playback ownership on %s", async reason => {
    fakes.handlers.get("tts:start")?.({ payload: { request_id: "chat_owned", text: "Playing." } });
    if (reason === "cleanup") service.cleanup();
    else if (reason === "interrupt") { fakes.playStateCallback?.(true); fakes.interrupt.callback?.(); }
    else fakes.handlers.get("tts:start")?.({ payload: { request_id: "chat_replacement", text: "New." } });
    expect(fakes.finishChatPlayback).toHaveBeenCalledWith("chat_owned");
  });
});
