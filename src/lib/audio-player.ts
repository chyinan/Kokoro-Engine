// pattern: Imperative Shell

export interface AudioAnalysis {
    /** RMS volume amplitude (0-1) */
    amplitude: number;
    /** Low-frequency energy ~200-800Hz (0-1) — wider/rounder mouth shapes */
    lowFreqEnergy: number;
    /** High-frequency energy ~1500-4000Hz (0-1) — narrower/tighter mouth shapes */
    highFreqEnergy: number;
}

export type AudioContainer = "mp3" | "wav" | "unknown";

export function detectAudioContainer(chunk: Uint8Array): AudioContainer {
    if (chunk.byteLength >= 12) {
        const isRiff = chunk[0] === 0x52 && chunk[1] === 0x49 && chunk[2] === 0x46 && chunk[3] === 0x46;
        const isWave = chunk[8] === 0x57 && chunk[9] === 0x41 && chunk[10] === 0x56 && chunk[11] === 0x45;
        if (isRiff && isWave) {
            return "wav";
        }
    }

    if (chunk.byteLength >= 3) {
        const hasId3 = chunk[0] === 0x49 && chunk[1] === 0x44 && chunk[2] === 0x33;
        if (hasId3) {
            return "mp3";
        }
    }

    if (chunk.byteLength >= 2) {
        const hasMpegSync = chunk[0] === 0xff && (chunk[1] & 0xe0) === 0xe0;
        if (hasMpegSync) {
            return "mp3";
        }
    }

    return "unknown";
}

export class AudioStreamManager {
    private audioContext: AudioContext;
    private analyser: AnalyserNode;
    private audioElement: HTMLAudioElement;
    private mediaElementSource: MediaElementAudioSourceNode;
    private appendQueue: Uint8Array[] = [];
    private wavQueue: AudioBuffer[] = [];
    private mediaSource: MediaSource | null = null;
    private sourceBuffer: SourceBuffer | null = null;
    private objectUrl: string | null = null;
    private streamEnded = false;
    private playbackStarted = false;
    private currentSource: AudioBufferSourceNode | null = null;
    private streamMode: AudioContainer | null = null;
    private _isPlaying = false;
    private analysisListeners: ((data: AudioAnalysis) => void)[] = [];
    private playStateListeners: ((playing: boolean) => void)[] = [];
    private animationFrameId?: number;
    private analysisActive = false;

    constructor() {
        this.audioContext = new AudioContext();
        this.analyser = this.audioContext.createAnalyser();
        this.analyser.fftSize = 256;

        this.audioElement = new Audio();
        this.audioElement.preload = "auto";
        this.audioElement.autoplay = false;
        this.audioElement.crossOrigin = "anonymous";

        this.mediaElementSource = this.audioContext.createMediaElementSource(this.audioElement);
        this.mediaElementSource.connect(this.analyser);
        this.analyser.connect(this.audioContext.destination);

        this.audioElement.onplay = () => {
            this._isPlaying = true;
            this.broadcastPlayState(true);
            this.startAnalysis();
        };

        this.audioElement.onpause = () => {
            if (this.audioElement.ended) return;
            this._isPlaying = false;
            this.stopAnalysis();
            this.broadcastPlayState(false);
        };

        this.audioElement.onended = () => {
            this._isPlaying = false;
            this.stopAnalysis();
            this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
            this.broadcastPlayState(false);
        };
    }

    public async resume() {
        if (this.audioContext.state === "suspended") {
            await this.audioContext.resume();
        }
    }

    public async queueAudio(data: Uint8Array | number[]) {
        await this.resume();

        const chunk = data instanceof Uint8Array ? data : new Uint8Array(data);
        if (chunk.byteLength === 0) {
            return;
        }

        if (!this.streamMode) {
            this.streamMode = detectAudioContainer(chunk);
        }

        if (this.streamMode === "wav") {
            await this.queueWavChunk(chunk);
            return;
        }

        this.ensureMpegStream();
        this.appendQueue.push(chunk);
        this.flushAppendQueue();
    }

    public finishStream() {
        this.streamEnded = true;

        if (this.streamMode === "wav") {
            if (!this.currentSource && this.wavQueue.length === 0) {
                this._isPlaying = false;
                this.stopAnalysis();
                this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
                this.broadcastPlayState(false);
            }
            return;
        }

        this.tryEndStream();
    }

    public addAmplitudeListener(callback: (data: AudioAnalysis) => void) {
        this.analysisListeners.push(callback);
    }

    public removeAmplitudeListener(callback: (data: AudioAnalysis) => void) {
        this.analysisListeners = this.analysisListeners.filter(l => l !== callback);
    }

    public onPlayStateChange(callback: (playing: boolean) => void): () => void {
        this.playStateListeners.push(callback);
        return () => {
            this.playStateListeners = this.playStateListeners.filter(l => l !== callback);
        };
    }

    public get isPlaying(): boolean {
        return this._isPlaying;
    }

    public stop() {
        this.audioElement.pause();
        if (this.objectUrl) {
            this.audioElement.removeAttribute("src");
            this.audioElement.load();
            URL.revokeObjectURL(this.objectUrl);
            this.objectUrl = null;
        }

        if (this.mediaSource) {
            try {
                if (this.mediaSource.readyState === "open") {
                    this.mediaSource.endOfStream();
                }
            } catch {
                // Ignore end-of-stream errors during teardown.
            }
        }

        if (this.currentSource) {
            try {
                this.currentSource.stop();
                this.currentSource.disconnect();
            } catch {
                // Ignore source teardown errors.
            }
            this.currentSource = null;
        }

        this.mediaSource = null;
        this.sourceBuffer = null;
        this.appendQueue = [];
        this.wavQueue = [];
        this.streamEnded = false;
        this.playbackStarted = false;
        this.streamMode = null;
        this._isPlaying = false;

        this.stopAnalysis();

        this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
        this.broadcastPlayState(false);
    }

    public clearQueue() {
        this.stop();
    }

    private ensureMpegStream() {
        if (this.mediaSource || !("MediaSource" in window)) {
            return;
        }

        if (!MediaSource.isTypeSupported("audio/mpeg")) {
            throw new Error("MediaSource audio/mpeg is not supported in this environment");
        }

        const mediaSource = new MediaSource();
        this.mediaSource = mediaSource;
        this.objectUrl = URL.createObjectURL(mediaSource);
        this.audioElement.src = this.objectUrl;

        mediaSource.addEventListener("sourceopen", () => {
            if (!this.mediaSource || this.sourceBuffer) {
                return;
            }

            this.sourceBuffer = this.mediaSource.addSourceBuffer("audio/mpeg");
            this.sourceBuffer.mode = "sequence";
            this.sourceBuffer.addEventListener("updateend", () => {
                this.flushAppendQueue();
            });
            this.flushAppendQueue();
        }, { once: true });
    }

    private async queueWavChunk(chunk: Uint8Array) {
        try {
            const buffer = chunk.buffer.slice(chunk.byteOffset, chunk.byteOffset + chunk.byteLength);
            const decoded = await this.audioContext.decodeAudioData(buffer);
            this.wavQueue.push(decoded);
            this.playNextWav();
        } catch (error) {
            console.error("[Audio] Failed to decode WAV chunk:", error);
            this.stop();
        }
    }

    private playNextWav() {
        if (this.currentSource || this.wavQueue.length === 0) {
            return;
        }

        const buffer = this.wavQueue.shift();
        if (!buffer) {
            return;
        }

        const source = this.audioContext.createBufferSource();
        source.buffer = buffer;
        source.connect(this.analyser);

        this.currentSource = source;
        this._isPlaying = true;
        this.broadcastPlayState(true);
        this.startAnalysis();

        source.onended = () => {
            if (this.currentSource === source) {
                this.currentSource = null;
            }
            if (this.wavQueue.length > 0) {
                this.playNextWav();
                return;
            }

            this._isPlaying = false;
            this.broadcastPlayState(false);
            this.stopAnalysis();
            this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
        };

        source.start();
    }

    private flushAppendQueue() {
        if (!this.sourceBuffer || this.sourceBuffer.updating) {
            return;
        }

        if (!this.mediaSource || this.mediaSource.readyState !== "open") {
            return;
        }

        const chunk = this.appendQueue.shift();
        if (!chunk) {
            this.tryEndStream();
            return;
        }

        try {
            this.sourceBuffer.appendBuffer(chunk);
            this.startPlaybackIfNeeded();
        } catch (error) {
            console.error("[Audio] Failed to append streaming chunk:", error);
            this.stop();
        }
    }

    private startPlaybackIfNeeded() {
        if (this.playbackStarted) {
            return;
        }

        this.playbackStarted = true;
        this.audioElement.play().catch(error => {
            console.warn("[Audio] Autoplay blocked or playback failed:", error);
            this.playbackStarted = false;
        });
    }

    private tryEndStream() {
        if (!this.streamEnded || !this.mediaSource || !this.sourceBuffer) {
            return;
        }

        if (this.appendQueue.length > 0 || this.sourceBuffer.updating) {
            return;
        }

        try {
            if (this.mediaSource.readyState === "open") {
                this.mediaSource.endOfStream();
            }
        } catch (error) {
            console.warn("[Audio] Failed to finalize media stream:", error);
        }
    }

    private startAnalysis() {
        if (this.analysisActive) {
            return;
        }

        this.analysisActive = true;
        const timeDomain = new Uint8Array(this.analyser.frequencyBinCount);
        const freqDomain = new Uint8Array(this.analyser.frequencyBinCount);
        const sampleRate = this.audioContext.sampleRate;
        const binCount = this.analyser.frequencyBinCount;
        const binHz = sampleRate / (binCount * 2);

        const lowStart = Math.floor(200 / binHz);
        const lowEnd = Math.min(Math.ceil(800 / binHz), binCount - 1);
        const highStart = Math.floor(1500 / binHz);
        const highEnd = Math.min(Math.ceil(4000 / binHz), binCount - 1);

        const update = () => {
            if (!this.analysisActive) {
                this.animationFrameId = undefined;
                return;
            }

            if (this.audioContext.state === "suspended") {
                this.animationFrameId = requestAnimationFrame(update);
                return;
            }

            this.analyser.getByteTimeDomainData(timeDomain);
            let sum = 0;
            for (let i = 0; i < timeDomain.length; i++) {
                const v = (timeDomain[i] - 128) / 128.0;
                sum += v * v;
            }
            const rms = Math.sqrt(sum / timeDomain.length);
            const amplitude = Math.min(rms * 4.0, 1.0);

            this.analyser.getByteFrequencyData(freqDomain);

            let lowSum = 0;
            let lowCount = 0;
            for (let i = lowStart; i <= lowEnd; i++) {
                lowSum += freqDomain[i];
                lowCount++;
            }

            let highSum = 0;
            let highCount = 0;
            for (let i = highStart; i <= highEnd; i++) {
                highSum += freqDomain[i];
                highCount++;
            }

            const lowFreqEnergy = lowCount > 0 ? Math.min((lowSum / lowCount) / 180, 1.0) : 0;
            const highFreqEnergy = highCount > 0 ? Math.min((highSum / highCount) / 180, 1.0) : 0;

            this.broadcastAnalysis({ amplitude, lowFreqEnergy, highFreqEnergy });
            this.animationFrameId = requestAnimationFrame(update);
        };

        update();
    }

    private stopAnalysis() {
        this.analysisActive = false;

        if (this.animationFrameId !== undefined) {
            cancelAnimationFrame(this.animationFrameId);
            this.animationFrameId = undefined;
        }
    }

    private broadcastAnalysis(data: AudioAnalysis) {
        this.analysisListeners.forEach(listener => listener(data));
    }

    private broadcastPlayState(playing: boolean) {
        this.playStateListeners.forEach(listener => listener(playing));
    }
}
