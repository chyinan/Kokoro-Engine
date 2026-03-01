import { Live2DModel } from "pixi-live2d-display/cubism4";
import { LipSyncProcessor } from "./LipSyncProcessor";
import { audioPlayer } from "../../core/services";
import type { AudioAnalysis } from "../../lib/audio-player";

export type AnimationState = "Idle" | "Speaking" | "Interactive";
export type EmotionState =
    | "calm"
    | "happy"
    | "sad"
    | "angry"
    | "surprised"
    | "thinking"
    | "shy"
    | "smug"
    | "worried"
    | "excited";

export type ActionIntent =
    | "idle" | "nod" | "shake" | "wave" | "dance"
    | "shy" | "think" | "surprise" | "cheer" | "tap";

export type IdleBehavior =
    | { type: "look_around"; params: { direction: number; duration_ms: number } }
    | { type: "stretch"; params: Record<string, never> }
    | { type: "hum"; params: { melody_seed: number } }
    | { type: "sigh"; params: Record<string, never> }
    | { type: "fidget"; params: Record<string, never> };

export interface Live2DControllerConfig {
    autoIdle: boolean;
    idleMotionGroup: string;
}

// Default mapping: semantic action → candidate motion group names (try in order)
const ACTION_MOTION_MAP: Record<ActionIntent, string[]> = {
    idle: ["Idle"],
    nod: ["TapBody", "Tap", "Nod"],
    shake: ["Shake", "FlickHead", "Flick"],
    wave: ["Wave", "TapBody", "Tap"],
    dance: ["Dance", "TapBody", "Tap"],
    shy: ["Shy", "TapBody"],
    think: ["Think", "Idle"],
    surprise: ["Surprise", "FlickHead", "Flick"],
    cheer: ["Cheer", "TapBody", "Tap"],
    tap: ["TapBody", "Tap"],
};

// Emotion → candidate motion groups to play alongside expression
const EMOTION_MOTION_MAP: Partial<Record<EmotionState, string[]>> = {
    happy: ["TapBody", "Tap"],
    excited: ["TapBody", "Tap", "Dance"],
    surprised: ["FlickHead", "Flick"],
    shy: ["Shy", "TapBody"],
    angry: ["Shake", "FlickHead"],
};

// MotionPriority enum matching pixi-live2d-display
const MOTION_PRIORITY = {
    IDLE: 1,
    NORMAL: 2,
    FORCE: 3,
} as const;

export class Live2DController {
    private model: Live2DModel | null = null;
    private lipSync: LipSyncProcessor;
    private currentEmotion: EmotionState = "calm";
    private currentState: AnimationState = "Idle";

    // Callbacks for external systems (e.g. Mod hooks)
    public onModelLoaded?: (model: Live2DModel) => void;
    public onEmotionChange?: (emotion: EmotionState) => void;

    private amplitudeHandler: (data: AudioAnalysis) => void;

    constructor() {
        this.lipSync = new LipSyncProcessor();

        // Subscribe to audio player
        this.amplitudeHandler = (data) => {
            this.lipSync.updateAudio(data.amplitude, data.lowFreqEnergy, data.highFreqEnergy);
            if (data.amplitude > 0.1 && this.currentState !== "Speaking") {
                this.startSpeaking();
            } else if (data.amplitude <= 0.01 && this.currentState === "Speaking") {
                // Auto-return to idle after speech ends
                this.stopSpeaking();
            }
        };
        audioPlayer.addAmplitudeListener(this.amplitudeHandler);
    }

    public setModel(model: Live2DModel) {
        this.model = model;

        if (this.onModelLoaded) {
            this.onModelLoaded(model);
        }

        // Log available motion groups for debugging
        const groups = this.getAvailableMotionGroups();
        console.log("[Live2DController] Available motion groups:", groups);

        // Start idle motion
        this.playIdleMotion();
    }

    public getModel(): Live2DModel | null {
        return this.model;
    }

    public getLipSync(): LipSyncProcessor {
        return this.lipSync;
    }

    public getCurrentEmotion(): EmotionState {
        return this.currentEmotion;
    }

    /**
     * Returns the list of motion group names available in the loaded model.
     * Reads from the motionManager.definitions which mirrors the model3.json Motions section.
     */
    public getAvailableMotionGroups(): string[] {
        if (!this.model) return [];

        try {
            const motionManager = this.model.internalModel?.motionManager;
            if (motionManager?.definitions) {
                return Object.keys(motionManager.definitions);
            }
        } catch (e) {
            console.warn("[Live2DController] Failed to read motion groups:", e);
        }

        return [];
    }

    /**
     * Returns the number of motions in a given group, or 0 if the group doesn't exist.
     */
    private getMotionCount(group: string): number {
        if (!this.model) return 0;

        try {
            const motionManager = this.model.internalModel?.motionManager;
            const defs = motionManager?.definitions;
            if (defs && group in defs) {
                const arr = defs[group as keyof typeof defs];
                return Array.isArray(arr) ? arr.length : 0;
            }
        } catch {
            // Ignore
        }

        return 0;
    }

    /**
     * Plays a motion triggered by an AI action intent.
     * Finds the first matching motion group from the candidate list and plays a random motion from it.
     */
    public playActionMotion(action: ActionIntent) {
        if (!this.model) return;

        const candidates = ACTION_MOTION_MAP[action];
        if (!candidates) {
            console.warn(`[Live2DController] Unknown action intent: ${action}`);
            return;
        }

        const availableGroups = this.getAvailableMotionGroups();

        for (const groupName of candidates) {
            if (availableGroups.includes(groupName)) {
                const count = this.getMotionCount(groupName);
                if (count > 0) {
                    const randomIndex = Math.floor(Math.random() * count);
                    console.log(`[Live2DController] Playing action "${action}" → motion group "${groupName}" index ${randomIndex}`);
                    this.model.motion(groupName, randomIndex, MOTION_PRIORITY.NORMAL);
                    return;
                }
            }
        }

        console.log(`[Live2DController] No matching motion group found for action "${action}", available: [${availableGroups.join(", ")}]`);
    }

    public setEmotion(emotion: EmotionState) {
        if (this.currentEmotion === emotion) return;
        this.currentEmotion = emotion;

        // Notify listeners (e.g. FooterBar mood indicator)
        this.onEmotionChange?.(emotion);

        if (this.model) {
            // Map emotion to expression name (Cubism facial parameter blend)
            this.model.expression(emotion);

            // Also trigger an emotion-specific motion if available
            const candidates = EMOTION_MOTION_MAP[emotion];
            if (candidates) {
                const availableGroups = this.getAvailableMotionGroups();
                for (const groupName of candidates) {
                    if (availableGroups.includes(groupName)) {
                        const count = this.getMotionCount(groupName);
                        if (count > 0) {
                            const randomIndex = Math.floor(Math.random() * count);
                            console.log(`[Live2DController] Emotion "${emotion}" → motion group "${groupName}" index ${randomIndex}`);
                            this.model.motion(groupName, randomIndex, MOTION_PRIORITY.NORMAL);
                            break;
                        }
                    }
                }
            }

            // If in idle state, switch to emotion-specific idle if available
            if (this.currentState === "Idle") {
                this.playIdleMotion();
            }
        }
    }

    public startSpeaking() {
        this.currentState = "Speaking";
        // Optionally play a "Start Speaking" gesture
    }

    public stopSpeaking() {
        this.currentState = "Idle";
        this.playIdleMotion();
    }

    public update(dt: number) {
        if (!this.model) return;

        // Convert Pixi delta (frames @ 60fps) to seconds
        const dtSecs = dt / 60;

        // Update Lip Sync — multi-parameter
        const mouth = this.lipSync.getValues(dtSecs);

        // Pixi-Live2D-Display handles internal ticking, but we inject parameters manually
        const internalModel = this.model.internalModel as unknown as {
            coreModel?: {
                setParameterValueById: (id: string, val: number) => void;
            };
        };
        const coreModel = internalModel.coreModel;
        if (coreModel) {
            // ParamMouthOpenY: jaw open/close (0-1)
            coreModel.setParameterValueById("ParamMouthOpenY", mouth.mouthOpenY);
            // ParamMouthForm: mouth shape — wide smile (-1) ↔ round O (+1)
            coreModel.setParameterValueById("ParamMouthForm", mouth.mouthForm);
        }
    }

    private playIdleMotion() {
        if (!this.model) return;
        // Priority 1 (IDLE): Default Idle
        this.model.motion("Idle");
    }

    public playMotion(group: string, index?: number, priority?: number) {
        this.model?.motion(group, index, priority);
    }

    public playIdleBehavior(behavior: IdleBehavior) {
        if (!this.model) return;

        console.log("[Live2DController] Playing idle behavior:", behavior);

        switch (behavior.type) {
            case "look_around":
                // TODO: Implement procedural look around using internal model parameters
                // For now, just ensure we are in idle motion
                this.playIdleMotion();
                break;
            case "stretch":
                this.tryPlayMotionGroup(["Stretch", "Idle"]);
                break;
            case "hum":
                this.tryPlayMotionGroup(["Hum", "Sing", "Idle"]);
                break;
            case "sigh":
                this.tryPlayMotionGroup(["Sigh", "Sad", "Idle"]);
                break;
            case "fidget":
                this.tryPlayMotionGroup(["Fidget", "TapBody", "Idle"]);
                break;
        }
    }

    private tryPlayMotionGroup(groups: string[]) {
        const available = this.getAvailableMotionGroups();
        for (const group of groups) {
            if (available.includes(group)) {
                this.playMotion(group, undefined, MOTION_PRIORITY.NORMAL);
                return;
            }
        }
    }

    public destroy() {
        audioPlayer.removeAmplitudeListener(this.amplitudeHandler);
    }
}

