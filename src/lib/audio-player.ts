export interface AudioAnalysis {
    /** RMS volume amplitude (0-1) */
    amplitude: number;
    /** Low-frequency energy ~200-800Hz (0-1) — wider/rounder mouth shapes */
    lowFreqEnergy: number;
    /** High-frequency energy ~1500-4000Hz (0-1) — narrower/tighter mouth shapes */
    highFreqEnergy: number;
}

export class AudioStreamManager {
    private audioContext: AudioContext;
    private analyser: AnalyserNode;
    private audioElement: HTMLAudioElement;
    private mediaElementSource: MediaElementAudioSourceNode;
    private appendQueue: Uint8Array[] = [];
    private mediaSource: MediaSource | null = null;
    private sourceBuffer: SourceBuffer | null = null;
    private objectUrl: string | null = null;
    private streamEnded = false;
    private playbackStarted = false;
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

        this.ensureStream();
        this.appendQueue.push(chunk);
        this.flushAppendQueue();
    }

    public finishStream() {
        this.streamEnded = true;
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

        this.mediaSource = null;
        this.sourceBuffer = null;
        this.appendQueue = [];
        this.streamEnded = false;
        this.playbackStarted = false;
        this._isPlaying = false;

        this.stopAnalysis();

        this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
        this.broadcastPlayState(false);
    }

    public clearQueue() {
        this.stop();
    }

    private ensureStream() {
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
