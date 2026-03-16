/** Shared singleton — CameraWatcher writes, ChatPanel reads on send. */
let _latestFrame: string | null = null;

export function setLatestCameraFrame(dataUrl: string | null) {
    _latestFrame = dataUrl;
}

export function getLatestCameraFrame(): string | null {
    return _latestFrame;
}
