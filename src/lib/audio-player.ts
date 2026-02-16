export interface AudioAnalysis {
    /** RMS volume amplitude (0-1) */
    amplitude: number;
    /** Low-frequency energy ~200-800Hz (0-1) — wider/rounder mouth shapes */
    lowFreqEnergy: number;
    /** High-frequency energy ~1500-4000Hz (0-1) — narrower/tighter mouth shapes */
    highFreqEnergy: number;
}

export class AudioStreamManager {
    private currentSource: AudioBufferSourceNode | null = null;
    private audioContext: AudioContext;
    private analyser: AnalyserNode;
    private queue: AudioBuffer[] = [];
    private _isPlaying = false;
    private analysisListeners: ((data: AudioAnalysis) => void)[] = [];
    private playStateListeners: ((playing: boolean) => void)[] = [];
    private animationFrameId?: number;

    constructor() {
        this.audioContext = new AudioContext();
        this.analyser = this.audioContext.createAnalyser();
        this.analyser.fftSize = 256;
        this.analyser.connect(this.audioContext.destination);
    }

    public async resume() {
        if (this.audioContext.state === 'suspended') {
            await this.audioContext.resume();
        }
    }

    public async queueAudio(data: Uint8Array | number[]) {
        await this.resume();

        // Ensure we have an ArrayBuffer
        let buffer: ArrayBuffer;
        if (data instanceof Uint8Array) {
            buffer = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
        } else {
            buffer = new Uint8Array(data).buffer as ArrayBuffer;
        }

        try {
            const audioBuffer = await this.audioContext.decodeAudioData(buffer);
            this.queue.push(audioBuffer);
            if (!this._isPlaying) {
                this.playNext();
            }
        } catch (e) {
            console.error("Failed to decode audio:", e);
        }
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
        // Clear queue first
        this.queue = [];

        if (this.currentSource) {
            try {
                this.currentSource.stop();
                this.currentSource.disconnect();
            } catch (e) {
                // Ignore errors if already stopped
            }
            this.currentSource = null;
        }
        this._isPlaying = false;

        // Don't stop analysis loop, just let it report 0
        // Or stop it to save resources? 
        // Let's keep reporting 0 for a moment to ensure UI resets
        this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
        this.broadcastPlayState(false);
    }

    public clearQueue() {
        this.stop();
    }

    private playNext() {
        if (this.queue.length === 0) {
            this._isPlaying = false;
            this.broadcastAnalysis({ amplitude: 0, lowFreqEnergy: 0, highFreqEnergy: 0 });
            this.broadcastPlayState(false);
            return;
        }

        const buffer = this.queue.shift();
        if (!buffer) return;

        this._isPlaying = true;
        const source = this.audioContext.createBufferSource();
        source.buffer = buffer;
        this.currentSource = source;

        // Notify listeners that playback started
        this.broadcastPlayState(true);

        // Connect source -> analyser
        // Analyser is persistent and connected to destination
        source.connect(this.analyser);

        source.onended = () => {
            this.currentSource = null;
            this.playNext();
        };

        source.start();

        // Ensure analysis is running
        if (!this.animationFrameId) {
            this.startAnalysis();
        }
    }

    private startAnalysis() {
        const timeDomain = new Uint8Array(this.analyser.frequencyBinCount);
        const freqDomain = new Uint8Array(this.analyser.frequencyBinCount);
        const sampleRate = this.audioContext.sampleRate;
        const binCount = this.analyser.frequencyBinCount;
        const binHz = sampleRate / (binCount * 2); // Hz per FFT bin

        // Precompute bin ranges for frequency bands
        const lowStart = Math.floor(200 / binHz);
        const lowEnd = Math.min(Math.ceil(800 / binHz), binCount - 1);
        const highStart = Math.floor(1500 / binHz);
        const highEnd = Math.min(Math.ceil(4000 / binHz), binCount - 1);

        const update = () => {
            if (this.audioContext.state === 'suspended') {
                this.animationFrameId = requestAnimationFrame(update);
                return;
            }

            // Time-domain: RMS amplitude
            this.analyser.getByteTimeDomainData(timeDomain);
            let sum = 0;
            for (let i = 0; i < timeDomain.length; i++) {
                const v = (timeDomain[i] - 128) / 128.0;
                sum += v * v;
            }
            const rms = Math.sqrt(sum / timeDomain.length);
            const amplitude = Math.min(rms * 4.0, 1.0);

            // Frequency-domain: band energies
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

    private broadcastAnalysis(data: AudioAnalysis) {
        this.analysisListeners.forEach(listener => listener(data));
    }

    private broadcastPlayState(playing: boolean) {
        this.playStateListeners.forEach(listener => listener(playing));
    }
}
