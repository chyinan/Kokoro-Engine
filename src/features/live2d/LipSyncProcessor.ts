export interface LipSyncConfig {
    /** Multiplier for the input RMS amplitude (default: 2.0) */
    gain: number;
    /** Smoothing factor — higher = snappier, lower = smoother (0.0 - 1.0) */
    smoothing: number;
    /** Threshold below which mouth is closed (default: 0.01) */
    threshold: number;
    /** Micro-movement amplitude during speech (0-1, default: 0.05) */
    microMovement: number;
}

const DEFAULT_CONFIG: LipSyncConfig = {
    gain: 2.0,
    smoothing: 0.25,
    threshold: 0.01,
    microMovement: 0.05,
};

/** Smoothed output values for driving Live2D mouth parameters */
export interface LipSyncOutput {
    /** Jaw open amount (0-1) → ParamMouthOpenY */
    mouthOpenY: number;
    /** Mouth shape: -1 = wide/smile, 0 = neutral, +1 = round/O → ParamMouthForm */
    mouthForm: number;
}

export class LipSyncProcessor {
    private config: LipSyncConfig;

    // Smoothed output values
    private currentOpenY: number = 0;
    private targetOpenY: number = 0;
    private currentForm: number = 0;
    private targetForm: number = 0;

    // Micro-movement state
    private microPhase: number = 0;

    constructor(config: Partial<LipSyncConfig> = {}) {
        this.config = { ...DEFAULT_CONFIG, ...config };
    }

    /**
     * Update the processor with new audio analysis data.
     * @param amplitude RMS amplitude (0-1)
     * @param lowFreqEnergy Energy in 200-800Hz band (0-1)
     * @param highFreqEnergy Energy in 1500-4000Hz band (0-1)
     */
    public updateAudio(amplitude: number, lowFreqEnergy: number = 0, highFreqEnergy: number = 0) {
        // Apply gain and clamp
        let amplified = amplitude * this.config.gain;

        // Noise gate
        if (amplified < this.config.threshold) {
            amplified = 0;
        }

        this.targetOpenY = Math.min(amplified, 1.0);

        // Estimate mouth form from frequency balance:
        // High low energy + low high energy → round/open vowel (あ, お) → positive mouthForm
        // Low low energy + high high energy → tight/narrow (い, え) → negative mouthForm
        if (amplified > this.config.threshold) {
            const total = lowFreqEnergy + highFreqEnergy + 0.001; // avoid div/0
            const ratio = (lowFreqEnergy - highFreqEnergy) / total;
            // ratio ranges from -1 (all high) to +1 (all low)
            // Map: positive ratio → round mouth (+1), negative → wide/smile (-1)
            this.targetForm = ratio * 0.8; // Scale down for subtlety
        } else {
            this.targetForm = 0;
        }
    }

    /**
     * Get the current smoothed mouth parameters.
     * Should be called every frame in the render loop.
     * @param dt Delta time in seconds (for framerate-independent smoothing)
     */
    public getValues(dt: number = 1 / 60): LipSyncOutput {
        // Framerate-independent exponential smoothing
        // factor = 1 - e^(-smoothing * dt * 60)
        const factor = 1 - Math.exp(-this.config.smoothing * dt * 60);

        this.currentOpenY = this.lerp(this.currentOpenY, this.targetOpenY, factor);
        this.currentForm = this.lerp(this.currentForm, this.targetForm, factor * 0.6); // Form changes slower

        // Snap to zero if very close
        if (this.currentOpenY < 0.001) this.currentOpenY = 0;
        if (Math.abs(this.currentForm) < 0.001) this.currentForm = 0;

        // Add micro-movement during speech for realism
        let microOffset = 0;
        if (this.currentOpenY > 0.05) {
            this.microPhase += dt * 12; // ~12Hz oscillation
            microOffset = Math.sin(this.microPhase) * this.config.microMovement * this.currentOpenY;
        } else {
            this.microPhase = 0;
        }

        return {
            mouthOpenY: Math.min(Math.max(this.currentOpenY + microOffset, 0), 1),
            mouthForm: Math.max(Math.min(this.currentForm, 1), -1),
        };
    }

    /** @deprecated Use getValues() instead */
    public getValue(): number {
        return this.getValues().mouthOpenY;
    }

    public updateConfig(newConfig: Partial<LipSyncConfig>) {
        this.config = { ...this.config, ...newConfig };
    }

    private lerp(v0: number, v1: number, t: number): number {
        return v0 * (1 - t) + v1 * t;
    }
}
