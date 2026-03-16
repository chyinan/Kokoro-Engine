/** Shared singleton — CameraWatcher writes, ChatPanel reads on send. */
const FRAME_MAX_AGE_MS = 30_000;

let _latestFrame: string | null = null;
let _capturedAt: number = 0;

export function setLatestCameraFrame(dataUrl: string | null) {
    _latestFrame = dataUrl;
    _capturedAt = dataUrl ? Date.now() : 0;
}

/** Returns the latest frame only if it was captured within the last 30 seconds. */
export function getLatestCameraFrame(): string | null {
    if (!_latestFrame) return null;
    if (Date.now() - _capturedAt > FRAME_MAX_AGE_MS) return null;
    return _latestFrame;
}
