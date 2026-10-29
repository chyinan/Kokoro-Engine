// pattern: Imperative Shell
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fakes = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => unknown>(),
  playStateCallback: null as ((playing: boolean) => void) | null,
  playing: false,
  cancelTts: vi.fn().mockResolvedValue(undefined),
  player: {
    clearQueue: vi.fn(), queueAudio: vi.fn(), finishStream: vi.fn(), stop: vi.fn(),
    onPlaybackError: vi.fn(() => () => {}),
    onPlayStateChange: vi.fn((callback: (playing: boolean) => void) => {
      fakes.playStateCallback = callback;
      return () => { fakes.playStateCallback = null; };
    }),
    get isPlaying() { return fakes.playing; },
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
vi.mock("../services", () => ({ audioPlayer: fakes.player }));
vi.mock("../../lib/kokoro-bridge", () => ({
  cancelTts: fakes.cancelTts,
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

  beforeEach(async () => {
    vi.resetModules();
    vi.clearAllMocks();
    fakes.handlers.clear();
    fakes.playStateCallback = null;
    fakes.playing = false;
    speechQueue = [];
    vi.stubGlobal("window", { speechSynthesis: {
      cancel: () => { speechQueue = []; },
      speak: (utterance: { text: string }) => { speechQueue.push(utterance.text); },
      getVoices: () => [],
    } });
    vi.stubGlobal("SpeechSynthesisUtterance", class { constructor(public text: string) {} });
    vi.stubGlobal("localStorage", { getItem: (key: string) => key === "kokoro_voice_interrupt" ? "true" : "false" });
    const module = await import("./tts-service");
    service = new module.TtsService();
    await service.init();
  });

  afterEach(() => { service.cleanup(); vi.unstubAllGlobals(); });

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

    expect(fakes.player.queueAudio).not.toHaveBeenCalled();
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
  });
});
