/**
 * VoiceInterruptService — Detects user speech via microphone to interrupt TTS playback.
 *
 * Uses Web Audio API AnalyserNode for real-time RMS level detection.
 * When the mic level exceeds a threshold during TTS playback, triggers an interrupt.
 */

type InterruptCallback = () => void;

export class VoiceInterruptService {
    private stream: MediaStream | null = null;
    private audioCtx: AudioContext | null = null;
    private analyser: AnalyserNode | null = null;
    private sourceNode: MediaStreamAudioSourceNode | null = null;
    private rafId: number | null = null;
    private active = false;

    // VAD parameters
    private threshold = 0.04;        // RMS threshold for speech detection
    private holdTimeMs = 300;         // Debounce: must stay above threshold for this duration
    private cooldownMs = 1000;        // Minimum time between interrupts

    // State
    private aboveThresholdSince: number | null = null;
    private lastInterruptTime = 0;
    private listeners: InterruptCallback[] = [];

    /**
     * Start listening for voice activity on the microphone.
     * Call this when TTS starts playing.
     */
    async start(): Promise<void> {
        if (this.active) return;

        try {
            this.stream = await navigator.mediaDevices.getUserMedia({
                audio: {
                    echoCancellation: true,
                    noiseSuppression: true,
                    autoGainControl: true,
                },
            });
        } catch (err) {
            console.warn("[VoiceInterrupt] Microphone access denied:", err);
            return;
        }

        this.audioCtx = new AudioContext();
        this.analyser = this.audioCtx.createAnalyser();
        this.analyser.fftSize = 512;
        this.analyser.smoothingTimeConstant = 0.3;

        this.sourceNode = this.audioCtx.createMediaStreamSource(this.stream);
        this.sourceNode.connect(this.analyser);
        // Do NOT connect analyser to destination — we don't want to play mic audio

        this.active = true;
        this.aboveThresholdSince = null;
        this.detectLoop();

        console.log("[VoiceInterrupt] Started listening");
    }

    /**
     * Stop listening and release microphone.
     * Call this when TTS finishes or is interrupted.
     */
    stop(): void {
        this.active = false;

        if (this.rafId != null) {
            cancelAnimationFrame(this.rafId);
            this.rafId = null;
        }

        if (this.sourceNode) {
            this.sourceNode.disconnect();
            this.sourceNode = null;
        }

        if (this.stream) {
            this.stream.getTracks().forEach(t => t.stop());
            this.stream = null;
        }

        if (this.audioCtx) {
            this.audioCtx.close().catch(() => { });
            this.audioCtx = null;
        }

        this.analyser = null;
        this.aboveThresholdSince = null;

        console.log("[VoiceInterrupt] Stopped");
    }

    /**
     * Register a callback for when voice interruption is detected.
     */
    onInterrupt(callback: InterruptCallback): () => void {
        this.listeners.push(callback);
        return () => {
            this.listeners = this.listeners.filter(l => l !== callback);
        };
    }

    /**
     * Update the detection threshold (0.01 ~ 0.2).
     */
    setThreshold(value: number): void {
        this.threshold = Math.max(0.01, Math.min(0.2, value));
    }

    get isActive(): boolean {
        return this.active;
    }

    // ── Internal ───────────────────────────────────────

    private detectLoop = (): void => {
        if (!this.active || !this.analyser) return;

        const dataArray = new Uint8Array(this.analyser.frequencyBinCount);
        this.analyser.getByteTimeDomainData(dataArray);

        // Calculate RMS
        let sum = 0;
        for (let i = 0; i < dataArray.length; i++) {
            const v = (dataArray[i] - 128) / 128.0;
            sum += v * v;
        }
        const rms = Math.sqrt(sum / dataArray.length);

        const now = Date.now();

        if (rms > this.threshold) {
            if (this.aboveThresholdSince === null) {
                this.aboveThresholdSince = now;
            } else if (now - this.aboveThresholdSince >= this.holdTimeMs) {
                // Sustained speech detected — check cooldown
                if (now - this.lastInterruptTime >= this.cooldownMs) {
                    this.lastInterruptTime = now;
                    this.aboveThresholdSince = null;
                    this.triggerInterrupt();
                }
            }
        } else {
            this.aboveThresholdSince = null;
        }

        this.rafId = requestAnimationFrame(this.detectLoop);
    };

    private triggerInterrupt(): void {
        console.log("[VoiceInterrupt] Speech detected — interrupting TTS");
        for (const cb of this.listeners) {
            try {
                cb();
            } catch (err) {
                console.error("[VoiceInterrupt] Callback error:", err);
            }
        }
    }
}
