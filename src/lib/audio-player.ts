// pattern: Imperative Shell
import type { TtsAudioFormat } from "../core/types/mod";

export interface AudioAnalysis {
    /** RMS volume amplitude (0-1) */
    amplitude: number;
    /** Low-frequency energy ~200-800Hz (0-1) — wider/rounder mouth shapes */
    lowFreqEnergy: number;
    /** High-frequency energy ~1500-4000Hz (0-1) — narrower/tighter mouth shapes */
    highFreqEnergy: number;
}

export type AudioContainer = "mp3" | "wav" | "pcm_s16_le" | "unknown";

const MAX_PCM_QUEUED_SECONDS = 8;

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
    // Decoding may finish out of order; retain the arrival sequence until playback.
    private wavQueue = new Map<number, AudioBuffer>();
    private nextWavSequence = 0;
    private nextWavPlaybackSequence = 0;
    private mediaSource: MediaSource | null = null;
    private sourceBuffer: SourceBuffer | null = null;
    private objectUrl: string | null = null;
    private streamEnded = false;
    private playbackStarted = false;
    private currentSource: AudioBufferSourceNode | null = null;
    private pcmSources = new Set<AudioBufferSourceNode>();
    private pcmRemainder = new Uint8Array(0);
    private nextPcmStartTime = 0;
    private streamMode: AudioContainer | null = null;
    private _isPlaying = false;
    private analysisListeners: ((data: AudioAnalysis) => void)[] = [];
    private playStateListeners: ((playing: boolean) => void)[] = [];
    private playbackErrorListeners: ((message: string) => void)[] = [];
    private animationFrameId?: number;
    private analysisActive = false;
    private playbackGeneration = 0;

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
            this.finishPlaybackState();
        };
    }

    public async resume() {
        if (this.audioContext.state === "suspended") {
            await this.audioContext.resume();
        }
    }

    public async queueAudio(data: Uint8Array | number[], format?: TtsAudioFormat | null) {
        const chunk = data instanceof Uint8Array ? data : new Uint8Array(data);
        if (chunk.byteLength === 0) {
            return;
        }

        const generation = this.playbackGeneration;
        const sequence = this.nextWavSequence++;
        await this.resume();
        if (generation !== this.playbackGeneration) return;

        const declaredMode: AudioContainer | null = format?.codec ?? null;
        if (!this.streamMode) {
            this.streamMode = declaredMode ?? detectAudioContainer(chunk);
        }

        if (this.streamMode === "pcm_s16_le" && format?.codec === "pcm_s16_le") {
            await this.queuePcmChunk(chunk, format, generation);
            return;
        }

        if (this.streamMode === "wav") {
            await this.queueWavChunk(chunk, generation, sequence);
            return;
        }

        this.ensureMpegStream();
        this.appendQueue.push(chunk);
        this.flushAppendQueue();
    }

    public finishStream() {
        this.streamEnded = true;

        if (this.streamMode === "pcm_s16_le") {
            if (this.pcmRemainder.length > 0) {
                console.warn("[Audio] Dropping an incomplete PCM sample at end of stream");
                this.pcmRemainder = new Uint8Array(0);
            }
            if (this.pcmSources.size === 0) {
                this.finishPlaybackState();
            }
            return;
        }

        if (this.streamMode === "wav") {
            if (!this.currentSource && this.wavQueue.size === 0) {
                this._isPlaying = false;
                this.stopAnalysis();
                this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
                this.broadcastPlayState(false);
            }
            return;
        }

        if (!this.streamMode) {
            this.finishPlaybackState();
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

    public onPlaybackError(callback: (message: string) => void): () => void {
        this.playbackErrorListeners.push(callback);
        return () => {
            this.playbackErrorListeners = this.playbackErrorListeners.filter(listener => listener !== callback);
        };
    }

    public get isPlaying(): boolean {
        return this._isPlaying;
    }

    public stop() {
        // Invalidate pending resume/decode operations before tearing down playback.
        this.playbackGeneration++;
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

        for (const source of this.pcmSources) {
            try {
                source.stop();
                source.disconnect();
            } catch {
                // Sources may already have ended while the queue is cleared.
            }
        }
        this.pcmSources.clear();
        this.pcmRemainder = new Uint8Array(0);
        this.nextPcmStartTime = this.audioContext.currentTime;

        this.mediaSource = null;
        this.sourceBuffer = null;
        this.appendQueue = [];
        this.wavQueue.clear();
        this.nextWavSequence = 0;
        this.nextWavPlaybackSequence = 0;
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

    private async queueWavChunk(chunk: Uint8Array, generation: number, sequence: number) {
        try {
            const buffer = chunk.buffer.slice(chunk.byteOffset, chunk.byteOffset + chunk.byteLength);
            const decoded = await this.audioContext.decodeAudioData(buffer);
            if (generation !== this.playbackGeneration) return;
            this.wavQueue.set(sequence, decoded);
            this.playNextWav();
        } catch (error) {
            if (generation !== this.playbackGeneration) return;
            console.error("[Audio] Failed to decode WAV chunk:", error);
            this.broadcastPlaybackError("Failed to decode WAV audio.");
            this.stop();
        }
    }

    private async queuePcmChunk(
        chunk: Uint8Array,
        format: Extract<TtsAudioFormat, { codec: "pcm_s16_le" }>,
        generation: number,
    ): Promise<void> {
        const { sample_rate: sampleRate, channels } = format;
        if (!Number.isFinite(sampleRate) || sampleRate <= 0 || !Number.isInteger(channels) || channels <= 0) {
            throw new Error("Invalid PCM audio format metadata");
        }

        const combined = new Uint8Array(this.pcmRemainder.length + chunk.length);
        combined.set(this.pcmRemainder);
        combined.set(chunk, this.pcmRemainder.length);
        const bytesPerFrame = channels * 2;
        const completeBytes = combined.length - (combined.length % bytesPerFrame);
        this.pcmRemainder = combined.slice(completeBytes);
        if (completeBytes === 0) return;

        const frameCount = completeBytes / bytesPerFrame;
        const maxFramesPerBuffer = Math.max(1, Math.floor(sampleRate * 2));
        for (let frameOffset = 0; frameOffset < frameCount; frameOffset += maxFramesPerBuffer) {
            const segmentFrames = Math.min(maxFramesPerBuffer, frameCount - frameOffset);
            const segmentDuration = segmentFrames / sampleRate;
            await this.waitForPcmCapacity(segmentDuration, generation);
            if (generation !== this.playbackGeneration) return;

            // BufferSource resamples to AudioContext.sampleRate without changing duration or pitch.
            const buffer = this.audioContext.createBuffer(channels, segmentFrames, sampleRate);
            const outputChannels = Array.from({ length: channels }, (_, channel) => buffer.getChannelData(channel));
            for (let frame = 0; frame < segmentFrames; frame++) {
                for (let channel = 0; channel < channels; channel++) {
                    const offset = (frameOffset + frame) * bytesPerFrame + channel * 2;
                    const sample = combined[offset] | (combined[offset + 1] << 8);
                    const signedSample = sample >= 0x8000 ? sample - 0x10000 : sample;
                    outputChannels[channel][frame] = signedSample / 32768;
                }
            }

            const source = this.audioContext.createBufferSource();
            source.buffer = buffer;
            source.connect(this.analyser);
            const now = this.audioContext.currentTime;
            const scheduledStart = Math.max(
                this.nextPcmStartTime,
                now + (this.pcmSources.size === 0 ? 0.02 : 0),
            );
            this.nextPcmStartTime = scheduledStart + segmentDuration;
            this.pcmSources.add(source);
            const sourceGeneration = generation;
            source.onended = () => {
                if (sourceGeneration !== this.playbackGeneration) return;
                this.pcmSources.delete(source);
                if (this.pcmSources.size === 0) {
                    this.nextPcmStartTime = this.audioContext.currentTime;
                    if (this.streamEnded) {
                        this.finishPlaybackState();
                    } else {
                        this._isPlaying = false;
                        this.stopAnalysis();
                        this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
                        this.broadcastPlayState(false);
                    }
                }
            };

            source.start(scheduledStart);
            if (!this._isPlaying) {
                this._isPlaying = true;
                this.broadcastPlayState(true);
            }
            this.startAnalysis();
        }
    }

    private async waitForPcmCapacity(incomingDuration: number, generation: number): Promise<void> {
        while (generation === this.playbackGeneration) {
            const queuedSeconds = Math.max(0, this.nextPcmStartTime - this.audioContext.currentTime);
            if (queuedSeconds + incomingDuration <= MAX_PCM_QUEUED_SECONDS) {
                return;
            }
            await new Promise<void>(resolve => window.setTimeout(resolve, 25));
        }
    }

    private playNextWav() {
        if (this.currentSource) {
            return;
        }

        const buffer = this.wavQueue.get(this.nextWavPlaybackSequence);
        if (!buffer) {
            return;
        }
        this.wavQueue.delete(this.nextWavPlaybackSequence);
        this.nextWavPlaybackSequence++;

        const source = this.audioContext.createBufferSource();
        source.buffer = buffer;
        source.connect(this.analyser);

        this.currentSource = source;
        this._isPlaying = true;
        this.broadcastPlayState(true);
        this.startAnalysis();

        source.onended = () => {
            if (this.currentSource !== source) return;
            this.currentSource = null;
            if (this.wavQueue.has(this.nextWavPlaybackSequence)) {
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
            this.broadcastPlaybackError("Failed to append streaming audio data.");
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

    private broadcastPlaybackError(message: string): void {
        this.playbackErrorListeners.forEach(listener => listener(message));
    }

    private finishPlaybackState(): void {
        this._isPlaying = false;
        this.stopAnalysis();
        this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
        this.broadcastPlayState(false);
    }
}
