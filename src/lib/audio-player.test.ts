// pattern: Imperative Shell
import { beforeEach, describe, expect, it, vi } from "vitest";

type SetupResult = {
  decodeAudioDataMock: ReturnType<typeof vi.fn>;
  isTypeSupportedMock: ReturnType<typeof vi.fn>;
  sources: AudioBufferSourceNode[];
};

function setupAudioEnvironment(): SetupResult {
  const globalAny = globalThis as Record<string, unknown>;

  const decodeAudioDataMock = vi.fn().mockResolvedValue({ duration: 1 });
  const isTypeSupportedMock = vi.fn().mockReturnValue(true);
  const sources: AudioBufferSourceNode[] = [];

  const analyser = {
    fftSize: 0,
    frequencyBinCount: 128,
    connect: vi.fn(),
    getByteTimeDomainData: vi.fn(),
    getByteFrequencyData: vi.fn(),
  };

  class MockAudio {
    preload = "";
    autoplay = false;
    crossOrigin = "";
    src = "";
    ended = false;
    onplay: (() => void) | null = null;
    onpause: (() => void) | null = null;
    onended: (() => void) | null = null;
    play = vi.fn().mockResolvedValue(undefined);
    pause = vi.fn();
    removeAttribute = vi.fn();
    load = vi.fn();
  }

  class MockAudioContext {
    state: AudioContextState = "running";
    sampleRate = 48000;
    destination = {};

    createAnalyser() {
      return analyser as unknown as AnalyserNode;
    }

    createMediaElementSource() {
      return {
        connect: vi.fn(),
        disconnect: vi.fn(),
      } as unknown as MediaElementAudioSourceNode;
    }

    createBufferSource() {
      const source = {
        buffer: null,
        connect: vi.fn(),
        disconnect: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        onended: null,
      } as unknown as AudioBufferSourceNode;
      sources.push(source);
      return source;
    }

    resume = vi.fn().mockResolvedValue(undefined);
    decodeAudioData = decodeAudioDataMock;
  }

  class MockSourceBuffer {
    mode: AppendMode = "sequence";
    updating = false;
    appendBuffer = vi.fn();
    addEventListener = vi.fn();
  }

  class MockMediaSource {
    static isTypeSupported = isTypeSupportedMock;
    readyState: ReadyState = "open";
    addEventListener = vi.fn();
    addSourceBuffer = vi.fn(() => new MockSourceBuffer() as unknown as SourceBuffer);
    endOfStream = vi.fn();
  }

  globalAny.window = globalAny;
  globalAny.Audio = MockAudio;
  globalAny.AudioContext = MockAudioContext;
  globalAny.MediaSource = MockMediaSource;
  const urlCtor = (globalThis as unknown as { URL: typeof URL }).URL;
  (urlCtor as unknown as { createObjectURL: ReturnType<typeof vi.fn>; revokeObjectURL: ReturnType<typeof vi.fn> }).createObjectURL = vi.fn(() => "blob:mock");
  (urlCtor as unknown as { createObjectURL: ReturnType<typeof vi.fn>; revokeObjectURL: ReturnType<typeof vi.fn> }).revokeObjectURL = vi.fn();
  globalAny.requestAnimationFrame = vi.fn(() => 1);
  globalAny.cancelAnimationFrame = vi.fn();

  return {
    decodeAudioDataMock,
    isTypeSupportedMock,
    sources,
  };
}

describe("AudioStreamManager format routing", () => {
  let env: SetupResult;

  beforeEach(() => {
    vi.resetModules();
    env = setupAudioEnvironment();
  });

  const wavChunk = new Uint8Array([
    0x52, 0x49, 0x46, 0x46, 0x24, 0x00, 0x00, 0x00,
    0x57, 0x41, 0x56, 0x45,
  ]);

  function deferDecode() {
    let resolve!: (buffer: AudioBuffer) => void;
    let reject!: (error: Error) => void;
    env.decodeAudioDataMock.mockImplementationOnce(() => new Promise<AudioBuffer>((res, rej) => {
      resolve = res;
      reject = rej;
    }));
    return {
      finish: () => resolve({ duration: 1 } as AudioBuffer),
      fail: () => reject(new Error("stale decode failure")),
    };
  }

  it("discards audio stopped while waiting for resume", async () => {
    const { AudioStreamManager } = await import("./audio-player");
    const manager = new AudioStreamManager();
    let resume!: () => void;
    vi.spyOn(manager, "resume").mockImplementationOnce(() => new Promise<void>(resolve => {
      resume = resolve;
    }));
    const pending = manager.queueAudio(wavChunk);
    manager.stop();
    resume();
    await pending;
    expect(env.decodeAudioDataMock).not.toHaveBeenCalled();
    expect(env.sources).toHaveLength(0);
    expect(manager.isPlaying).toBe(false);
  });

  it.each(["stop", "clearQueue"] as const)("does not restart pending WAV decoding after %s", async (action) => {
    const { AudioStreamManager } = await import("./audio-player");
    const manager = new AudioStreamManager();
    const decode = deferDecode();
    const pending = manager.queueAudio(wavChunk);
    await Promise.resolve();
    expect(env.decodeAudioDataMock).toHaveBeenCalledTimes(1);
    manager[action]();
    decode.finish();
    await pending;
    expect(env.sources).toHaveLength(0);
    expect(manager.isPlaying).toBe(false);
  });

  it.each(["finish", "fail"] as const)("ignores stale decode %s after new playback starts", async (outcome) => {
    const { AudioStreamManager } = await import("./audio-player");
    const manager = new AudioStreamManager();
    const decode = deferDecode();
    const pending = manager.queueAudio(wavChunk);
    await Promise.resolve();
    manager.clearQueue();
    await manager.queueAudio(wavChunk);
    decode[outcome]();
    await pending;
    expect(manager.isPlaying).toBe(true);
    expect(env.sources).toHaveLength(1);
    expect(env.sources[0].stop).not.toHaveBeenCalled();
    env.sources[0].onended?.call(env.sources[0], new Event("ended"));
    expect(env.sources).toHaveLength(1);
    expect(manager.isPlaying).toBe(false);
  });

  it("ignores the stopped source's delayed ended event during new playback", async () => {
    const { AudioStreamManager } = await import("./audio-player");
    const manager = new AudioStreamManager();
    await manager.queueAudio(wavChunk);
    const oldSource = env.sources[0];
    manager.stop();
    await manager.queueAudio(wavChunk);
    oldSource.onended?.call(oldSource, new Event("ended"));
    expect(manager.isPlaying).toBe(true);
    expect(env.sources[1].stop).not.toHaveBeenCalled();
  });

  it("detects WAV container from RIFF/WAVE header", async () => {
    const { detectAudioContainer } = await import("./audio-player");
    const wavHeader = new Uint8Array([
      0x52, 0x49, 0x46, 0x46,
      0x24, 0x00, 0x00, 0x00,
      0x57, 0x41, 0x56, 0x45,
    ]);

    expect(detectAudioContainer(wavHeader)).toBe("wav");
  });

  it("review_r09 preserves WAV arrival order when decoding finishes out of order", async () => {
    const { AudioStreamManager } = await import("./audio-player");
    const manager = new AudioStreamManager();
    const firstAudio = { duration: 1 } as AudioBuffer;
    const secondAudio = { duration: 2 } as AudioBuffer;
    let finishFirst!: (value: AudioBuffer) => void;
    env.decodeAudioDataMock
      .mockImplementationOnce(() => new Promise<AudioBuffer>(resolve => { finishFirst = resolve; }))
      .mockResolvedValueOnce(secondAudio);

    const firstRequest = manager.queueAudio(wavChunk);
    await Promise.resolve();
    await manager.queueAudio(wavChunk);
    finishFirst(firstAudio);
    await firstRequest;
    const firstPlayed = env.sources[0]?.buffer;
    manager.stop();

    expect(firstPlayed).toBe(firstAudio);
  });

  it("uses decodeAudioData path for WAV chunks instead of forcing MPEG MSE", async () => {
    const { AudioStreamManager } = await import("./audio-player");

    const manager = new AudioStreamManager();
    const wavChunk = new Uint8Array([
      0x52, 0x49, 0x46, 0x46,
      0x24, 0x00, 0x00, 0x00,
      0x57, 0x41, 0x56, 0x45,
      0x64, 0x61, 0x74, 0x61,
      0x00, 0x00, 0x00, 0x00,
    ]);

    await manager.queueAudio(wavChunk);

    expect(env.decodeAudioDataMock).toHaveBeenCalledTimes(1);
    expect(env.isTypeSupportedMock).not.toHaveBeenCalled();
  });
});
