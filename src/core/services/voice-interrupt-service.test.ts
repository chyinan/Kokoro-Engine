import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { VoiceInterruptService } from "./voice-interrupt-service";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function microphone() {
  const stop = vi.fn();
  const stream = { getTracks: () => [{ stop }] } as unknown as MediaStream;
  return { stream, stop };
}

describe("VoiceInterruptService microphone lifecycle", () => {
  let service: VoiceInterruptService;
  let getUserMedia: ReturnType<typeof vi.fn>;
  let createContext: ReturnType<typeof vi.fn<() => void>>;
  let close: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    getUserMedia = vi.fn();
    createContext = vi.fn();
    close = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia } });
    vi.stubGlobal("AudioContext", class {
      constructor() { createContext(); }
      createAnalyser() {
        return {
          frequencyBinCount: 256,
          getByteTimeDomainData: (data: Uint8Array) => data.fill(128),
        };
      }
      createMediaStreamSource() { return { connect: vi.fn(), disconnect: vi.fn() }; }
      close = close;
    });
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.spyOn(console, "log").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    service = new VoiceInterruptService();
  });

  afterEach(() => {
    service.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("releases a microphone granted after stop without starting detection", async () => {
    const request = deferred<MediaStream>();
    const mic = microphone();
    getUserMedia.mockReturnValueOnce(request.promise);
    const pending = service.start();
    service.stop();
    request.resolve(mic.stream);
    await pending;
    expect(service.isActive).toBe(false);
    expect(mic.stop).toHaveBeenCalledOnce();
    expect(createContext).not.toHaveBeenCalled();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
  });

  it("does not acquire multiple microphones for concurrent starts", async () => {
    const request = deferred<MediaStream>();
    const mic = microphone();
    getUserMedia.mockReturnValue(request.promise);
    const first = service.start();
    const second = service.start();
    request.resolve(mic.stream);
    await Promise.all([first, second]);
    expect(getUserMedia).toHaveBeenCalledOnce();
    expect(createContext).toHaveBeenCalledOnce();
    expect(service.isActive).toBe(true);
    service.stop();
    expect(mic.stop).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
  });

  it("releases an old grant without replacing the new active microphone", async () => {
    const oldRequest = deferred<MediaStream>();
    const oldMic = microphone();
    const newMic = microphone();
    getUserMedia.mockReturnValueOnce(oldRequest.promise).mockResolvedValueOnce(newMic.stream);
    const oldStart = service.start();
    service.stop();
    await service.start();
    oldRequest.resolve(oldMic.stream);
    await oldStart;
    expect(oldMic.stop).toHaveBeenCalledOnce();
    expect(newMic.stop).not.toHaveBeenCalled();
    expect(createContext).toHaveBeenCalledOnce();
    expect(service.isActive).toBe(true);
    service.stop();
    expect(newMic.stop).toHaveBeenCalledOnce();
  });

  it.each(["resolve", "reject"] as const)("keeps a newer pending start protected when the old request settles: %s", async (outcome) => {
    const oldRequest = deferred<MediaStream>();
    const newRequest = deferred<MediaStream>();
    const oldMic = microphone();
    const newMic = microphone();
    getUserMedia.mockReturnValueOnce(oldRequest.promise).mockReturnValue(newRequest.promise);
    const oldStart = service.start();
    service.stop();
    const newStart = service.start();
    if (outcome === "resolve") oldRequest.resolve(oldMic.stream);
    else oldRequest.reject(new Error("Permission denied"));
    await oldStart;
    const duplicate = service.start();
    newRequest.resolve(newMic.stream);
    await Promise.all([newStart, duplicate]);
    expect(getUserMedia).toHaveBeenCalledTimes(2);
    expect(createContext).toHaveBeenCalledOnce();
    expect(service.isActive).toBe(true);
  });

  it("allows retry after microphone permission is denied", async () => {
    const mic = microphone();
    getUserMedia.mockRejectedValueOnce(new Error("Permission denied")).mockResolvedValueOnce(mic.stream);
    await service.start();
    expect(service.isActive).toBe(false);
    await service.start();
    expect(service.isActive).toBe(true);
    expect(getUserMedia).toHaveBeenCalledTimes(2);
    await service.start();
    expect(getUserMedia).toHaveBeenCalledTimes(2);
  });
});
