// pattern: Imperative Shell
import { beforeEach, describe, expect, it, vi } from "vitest";

type SetupResult = {
  decodeAudioDataMock: ReturnType<typeof vi.fn>;
  isTypeSupportedMock: ReturnType<typeof vi.fn>;
};

function setupAudioEnvironment(): SetupResult {
  const globalAny = globalThis as Record<string, unknown>;

  const decodeAudioDataMock = vi.fn().mockResolvedValue({ duration: 1 });
  const isTypeSupportedMock = vi.fn().mockReturnValue(true);

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
      return {
        buffer: null,
        connect: vi.fn(),
        disconnect: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        onended: null,
      } as unknown as AudioBufferSourceNode;
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
  };
}

describe("AudioStreamManager format routing", () => {
  let env: SetupResult;

  beforeEach(() => {
    vi.resetModules();
    env = setupAudioEnvironment();
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
