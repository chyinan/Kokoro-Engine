// pattern: Imperative Shell
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fakes = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => unknown>(),
  player: { clearQueue: vi.fn(), queueAudio: vi.fn(), finishStream: vi.fn(), stop: vi.fn() },
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
    fakes.handlers.get("tts:start")?.({ payload: { text: "Still generating." } });
    fakes.interrupt.callback?.();
    expect(fakes.player.stop).toHaveBeenCalledOnce();

    await fakes.handlers.get("tts:audio")?.({ payload: { data: [1, 2, 3] } });

    expect(fakes.player.queueAudio).not.toHaveBeenCalled();
  });
});
