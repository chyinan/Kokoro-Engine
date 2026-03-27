import { Live2DModel } from "pixi-live2d-display/cubism4";
import { LipSyncProcessor } from "./LipSyncProcessor";
import { audioPlayer } from "../../core/services";
import type { AudioAnalysis } from "../../lib/audio-player";
import { getLive2dModelProfile, type Live2dModelProfile } from "../../lib/kokoro-bridge";

export type AnimationState = "Idle" | "Speaking" | "Interactive";
export type CueName = string;

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

const MOTION_PRIORITY = {
    IDLE: 1,
    NORMAL: 2,
    FORCE: 3,
} as const;

type ExpressionDefinition = {
    Name?: string;
    name?: string;
    File?: string;
    file?: string;
};

export class Live2DController {
    private model: Live2DModel | null = null;
    private lipSync: LipSyncProcessor;
    private currentCue: CueName | null = null;
    private currentState: AnimationState = "Idle";
    private modelPath: string | null = null;
    private modelProfile: Live2dModelProfile | null = null;
    private profileRequestId = 0;

    public onModelLoaded?: (model: Live2DModel) => void;
    public onCueChange?: (cue: CueName | null) => void;

    private amplitudeHandler: (data: AudioAnalysis) => void;

    constructor() {
        this.lipSync = new LipSyncProcessor();
        this.amplitudeHandler = (data) => {
            this.lipSync.updateAudio(data.amplitude, data.lowFreqEnergy, data.highFreqEnergy);
            if (data.amplitude > 0.1 && this.currentState !== "Speaking") {
                this.startSpeaking();
            } else if (data.amplitude <= 0.01 && this.currentState === "Speaking") {
                this.stopSpeaking();
            }
        };
        audioPlayer.addAmplitudeListener(this.amplitudeHandler);
    }

    public setModel(model: Live2DModel) {
        this.model = model;
        this.onModelLoaded?.(model);
        console.log("[Live2DController] Available expressions:", this.getAvailableExpressions());
        console.log("[Live2DController] Available motion groups:", this.getAvailableMotionGroups());
        this.playIdleMotion();
    }

    public async loadProfileForModel(modelPath: string | null) {
        this.modelPath = modelPath;
        const requestId = ++this.profileRequestId;

        if (!modelPath) {
            this.modelProfile = null;
            return;
        }

        try {
            const profile = await getLive2dModelProfile(modelPath);
            if (requestId !== this.profileRequestId) return;
            this.modelProfile = profile;
        } catch (error) {
            if (requestId !== this.profileRequestId) return;
            this.modelProfile = null;
            console.warn(`[Live2DController] Failed to load model profile for '${modelPath}':`, error);
        }
    }

    public setProfile(profile: Live2dModelProfile | null) {
        this.modelProfile = profile;
        this.modelPath = profile?.model_path ?? this.modelPath;
    }

    public getModelProfile(): Live2dModelProfile | null {
        return this.modelProfile;
    }

    public getModel(): Live2DModel | null {
        return this.model;
    }

    public getLipSync(): LipSyncProcessor {
        return this.lipSync;
    }

    public getCurrentCue(): CueName | null {
        return this.currentCue;
    }

    public getAvailableExpressions(): string[] {
        if (this.modelProfile) {
            return this.modelProfile.available_expressions;
        }

        const definitions = this.getExpressionDefinitions();
        return definitions
            .map((definition) => definition.Name ?? definition.name)
            .filter((name): name is string => Boolean(name && name.trim()))
            .map((name) => name.trim());
    }

    public getAvailableMotionGroups(): string[] {
        return Object.keys(this.getAvailableMotionGroupCounts());
    }

    public getAvailableMotionGroupCounts(): Record<string, number> {
        if (this.modelProfile) {
            return this.modelProfile.available_motion_groups;
        }

        const groups: Record<string, number> = {};
        if (!this.model) return groups;

        try {
            const defs = this.model.internalModel?.motionManager?.definitions;
            if (defs) {
                for (const [group, motions] of Object.entries(defs)) {
                    groups[group] = Array.isArray(motions) ? motions.length : 0;
                }
            }
        } catch (error) {
            console.warn("[Live2DController] Failed to read motion groups:", error);
        }

        return groups;
    }

    public async playExpressionByName(name: string) {
        if (!this.model) return false;

        const trimmed = name.trim();
        const definitions = this.getExpressionDefinitions();
        const expressionIndex = definitions.findIndex((definition) => {
            const candidates = [
                definition.Name,
                definition.name,
                definition.File,
                definition.file,
            ]
                .filter((value): value is string => Boolean(value && value.trim()))
                .flatMap((value) => {
                    const normalized = value.trim();
                    const baseName = normalized.split("/").pop() ?? normalized;
                    return [
                        normalized,
                        baseName,
                        baseName.replace(/\.json$/i, ""),
                    ];
                });

            return candidates.includes(trimmed);
        });

        try {
            if (expressionIndex >= 0) {
                const playedByIndex = await this.model.expression(expressionIndex);
                if (playedByIndex) {
                    return true;
                }
            }

            return await this.model.expression(trimmed);
        } catch (error) {
            console.warn(`[Live2DController] Failed to play expression '${trimmed}':`, error);
            return false;
        }
    }

    public async playMotionGroupByName(group: string, index?: number, priority: number = MOTION_PRIORITY.NORMAL) {
        if (!this.model) return false;

        const count = this.getMotionCount(group);
        if (count <= 0) return false;

        const motionIndex = typeof index === "number"
            ? Math.max(0, Math.min(index, count - 1))
            : Math.floor(Math.random() * count);

        try {
            return await this.model.motion(group, motionIndex, priority);
        } catch (error) {
            console.warn(`[Live2DController] Failed to play motion group '${group}':`, error);
            return false;
        }
    }

    public async playCue(cue: string) {
        const trimmed = cue.trim();
        if (!trimmed) return false;

        this.currentCue = trimmed;
        this.onCueChange?.(trimmed);

        const binding = this.modelProfile?.cue_map?.[trimmed];
        const hasKnownBinding = Boolean(
            binding && (
                (binding.expression && this.getAvailableExpressions().includes(binding.expression)) ||
                (binding.motion_group && this.getAvailableMotionGroups().includes(binding.motion_group))
            )
        );
        let played = false;

        if (binding?.expression) {
            played = (await this.playExpressionByName(binding.expression)) || played;
        }

        if (binding?.motion_group) {
            played = (await this.playMotionGroupByName(binding.motion_group)) || played;
        }

        if (played) {
            return true;
        }

        if (hasKnownBinding) {
            return true;
        }

        const expressionPlayed = this.getAvailableExpressions().includes(trimmed)
            ? await this.playExpressionByName(trimmed)
            : false;
        const motionPlayed = this.getAvailableMotionGroups().includes(trimmed)
            ? await this.playMotionGroupByName(trimmed)
            : false;

        if (expressionPlayed || motionPlayed) {
            return true;
        }

        if (import.meta.env.DEV) {
            console.warn(`[Live2DController] Unmapped cue '${trimmed}'`, {
                modelPath: this.modelPath,
                configuredCues: Object.keys(this.modelProfile?.cue_map ?? {}),
                binding,
                availableExpressions: this.getAvailableExpressions(),
                availableMotionGroups: this.getAvailableMotionGroups(),
            });
        }

        return false;
    }

    public resolveSemanticCue(semanticKey: string): CueName | null {
        const normalizedKey = semanticKey.trim().toLowerCase();
        if (!normalizedKey) return null;

        const cue = this.modelProfile?.semantic_cue_map?.[normalizedKey];
        if (cue?.trim()) {
            return cue.trim();
        }
        return null;
    }

    public resolveInteractionSemanticCue(gesture: string, hitArea: string): CueName | null {
        const normalizedGesture = gesture.trim().toLowerCase();
        const normalizedArea = hitArea.trim().toLowerCase();
        const candidates = [
            `interaction:${normalizedGesture}_${normalizedArea}`,
            `interaction:${normalizedGesture}_*`,
            `interaction:*_${normalizedArea}`,
            "interaction:*_*",
        ];

        for (const semanticKey of candidates) {
            const cue = this.resolveSemanticCue(semanticKey);
            if (cue) {
                return cue;
            }
        }

        return null;
    }

    public startSpeaking() {
        this.currentState = "Speaking";
    }

    public stopSpeaking() {
        this.currentState = "Idle";
        this.playIdleMotion();
    }

    public update(dt: number) {
        if (!this.model) return;

        const dtSecs = dt / 60;
        const mouth = this.lipSync.getValues(dtSecs);
        const internalModel = this.model.internalModel as unknown as {
            coreModel?: {
                setParameterValueById: (id: string, val: number) => void;
            };
        };

        const coreModel = internalModel.coreModel;
        if (coreModel) {
            coreModel.setParameterValueById("ParamMouthOpenY", mouth.mouthOpenY);
            coreModel.setParameterValueById("ParamMouthForm", mouth.mouthForm);
        }
    }

    private playIdleMotion() {
        this.resetExpressionToDefault();
        const availableGroups = this.getAvailableMotionGroups();
        if (availableGroups.includes("Idle")) {
            void this.playMotionGroupByName("Idle", undefined, MOTION_PRIORITY.IDLE);
        }
    }

    public playMotion(group: string, index?: number, priority?: number) {
        void this.playMotionGroupByName(group, index, priority);
    }

    public playIdleBehavior(behavior: IdleBehavior) {
        if (!this.model) return;

        switch (behavior.type) {
            case "look_around":
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
                void this.playMotionGroupByName(group);
                return;
            }
        }
    }

    private getMotionCount(group: string): number {
        return this.getAvailableMotionGroupCounts()[group] ?? 0;
    }

    private getExpressionDefinitions(): ExpressionDefinition[] {
        if (!this.model) return [];

        try {
            const expressionManager = this.model.internalModel?.motionManager?.expressionManager;
            if (Array.isArray(expressionManager?.definitions)) {
                return expressionManager.definitions as ExpressionDefinition[];
            }
        } catch (error) {
            console.warn("[Live2DController] Failed to read expression definitions:", error);
        }

        return [];
    }

    private resetExpressionToDefault() {
        if (!this.model) return;

        try {
            this.model.internalModel?.motionManager?.expressionManager?.resetExpression?.();
            this.currentCue = null;
            this.onCueChange?.(null);
        } catch (error) {
            console.warn("[Live2DController] Failed to reset expression to default:", error);
        }
    }

    public destroy() {
        audioPlayer.removeAmplitudeListener(this.amplitudeHandler);
    }
}
