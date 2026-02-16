/**
 * Live2DViewer — React component wrapping PixiJS + pixi-live2d-display
 *
 * Features:
 * - Auto-resize canvas to fill container
 * - Mouse/touch gaze tracking
 * - Hit area click detection with callback
 * - Expression and motion control via ref
 */
import { useEffect, useRef, useCallback, forwardRef, useImperativeHandle } from "react";
import * as PIXI from "pixi.js";
import { Live2DModel } from "pixi-live2d-display/cubism4";
import { Live2DController, type EmotionState, type ActionIntent, type IdleBehavior } from "./Live2DController";
import { onChatExpression, onChatAction } from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";
import { interactionService } from "../../core/services/interaction-service";

// Register PIXI to the global window for pixi-live2d-display internals
const win = window as Window & { PIXI?: typeof PIXI };
win.PIXI = PIXI;
PIXI.utils.skipHello();

// ── Types ──────────────────────────────────────────

export interface Live2DViewerHandle {
    /** @deprecated Use controller directly */
    setExpression: (name: string) => void;
    /** @deprecated Use controller directly */
    setMouthOpen: (val: number) => void;
    /** @deprecated Use controller directly */
    playMotion: (group: string, index?: number) => void;
    getModel: () => Live2DModel | null;
    getController: () => Live2DController | undefined;
}

export type Live2DDisplayMode = "full" | "upper" | "upper-thigh";

export interface Live2DViewerProps {
    /** URL to the .model3.json file */
    modelUrl: string;
    /** Optional controller instance to manage the model state */
    controller?: Live2DController;
    /** Called when a hit area on the model is tapped (legacy) */
    onHitAreaTap?: (hitArea: string) => void;
    /** CSS class for the container */
    className?: string;
    /** Background alpha (0-1), default 0 */
    backgroundAlpha?: number;
    /** Display mode: full body, upper body, upper body + thighs */
    displayMode?: Live2DDisplayMode;
}

// ── Component ──────────────────────────────────────

const Live2DViewer = forwardRef<Live2DViewerHandle, Live2DViewerProps>(
    ({ modelUrl, controller, onHitAreaTap, className, backgroundAlpha = 0, displayMode = "full" }, ref) => {
        const containerRef = useRef<HTMLDivElement>(null);
        const appRef = useRef<PIXI.Application | null>(null);
        const modelRef = useRef<Live2DModel | null>(null);

        // Internal controller if none provided
        const internalControllerRef = useRef<Live2DController | null>(null);

        const getActiveController = useCallback(() => {
            return controller || internalControllerRef.current;
        }, [controller]);

        // Initialize internal controller if needed
        useEffect(() => {
            if (!controller && !internalControllerRef.current) {
                internalControllerRef.current = new Live2DController();
            }

            return () => {
                internalControllerRef.current?.destroy();
            };
        }, [controller]);

        // Expose control methods to parent
        useImperativeHandle(ref, () => {
            const ctrl = getActiveController();
            return {
                setExpression(name: string) {
                    ctrl?.setEmotion(name as EmotionState);
                },
                playMotion(group: string, index = 0) {
                    ctrl?.playMotion(group, index);
                },
                setMouthOpen(val: number) {
                    // Manual override through handle is discouraged but supported
                    // This might conflict with the LipSyncProcessor in the controller
                    // We can manually push to the processor just in case
                    ctrl?.getLipSync().updateAudio(val); // Hacky adapter
                },
                getModel() {
                    return modelRef.current;
                },
                getController() {
                    return ctrl || undefined;
                }
            };
        });

        // Listen for LLM expression events and apply to Live2D model
        useEffect(() => {
            let unlisten: (() => void) | undefined;

            onChatExpression((data) => {
                const ctrl = getActiveController();
                if (ctrl) {
                    ctrl.setEmotion(data.expression as EmotionState);
                }
            }).then(fn => { unlisten = fn; });

            return () => { unlisten?.(); };
        }, [getActiveController]);

        // Listen for LLM action/motion events and apply to Live2D model
        useEffect(() => {
            let unlisten: (() => void) | undefined;

            onChatAction((data) => {
                const ctrl = getActiveController();
                if (ctrl) {
                    ctrl.playActionMotion(data.action as ActionIntent);
                }
            }).then(fn => { unlisten = fn; });

            return () => { unlisten?.(); };
        }, [getActiveController]);

        // Listen for idle behavior events
        useEffect(() => {
            let unlisten: (() => void) | undefined;

            listen<any>("idle-behavior", (event) => {
                const ctrl = getActiveController();
                if (ctrl && event.payload && event.payload.behavior) {
                    ctrl.playIdleBehavior(event.payload.behavior as IdleBehavior);
                }
            }).then(fn => { unlisten = fn; });

            return () => { unlisten?.(); };
        }, [getActiveController]);

        // Gaze tracking: model eyes follow cursor
        const handlePointerMove = useCallback((e: PIXI.InteractionEvent) => {
            const model = modelRef.current;
            if (!model) return;
            model.focus(e.data.global.x, e.data.global.y);
        }, []);

        useEffect(() => {
            const container = containerRef.current;
            if (!container) return;

            // Create PixiJS application
            const app = new PIXI.Application({
                backgroundAlpha: backgroundAlpha,
                resizeTo: container,
                antialias: true,
                autoStart: true,
            });
            appRef.current = app;
            container.appendChild(app.view as HTMLCanvasElement);

            // Enable interaction
            app.stage.interactive = true;
            app.stage.hitArea = app.screen;
            app.stage.on("pointermove", handlePointerMove);

            // Load the Live2D model
            let cancelled = false;

            const loadModel = async () => {
                try {
                    const model = await Live2DModel.from(modelUrl, {
                        autoInteract: false, // We handle interaction manually via controller
                    });

                    if (cancelled) {
                        model.destroy();
                        return;
                    }

                    modelRef.current = model;
                    const ctrl = getActiveController();
                    if (ctrl) {
                        ctrl.setModel(model);
                    }

                    // Capture original dimensions for consistent scaling
                    const originalWidth = model.width;
                    const originalHeight = model.height;

                    // Scale model to fit container based on display mode
                    const fitModel = () => {
                        const scaleX = app.screen.width / originalWidth;
                        const scaleY = app.screen.height / originalHeight;

                        // Mode-specific scaling
                        let scale: number;

                        switch (displayMode) {
                            case "upper":
                                // Modest zoom, anchor top of model near top of screen
                                scale = Math.min(scaleX, scaleY) * 1.5;
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                // Place model so head is visible: top-align with small margin
                                model.y = app.screen.height * 0.05;
                                break;
                            case "upper-thigh":
                                // Slight zoom, show most of the body
                                scale = Math.min(scaleX, scaleY) * 1.25;
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                model.y = app.screen.height * 0.03;
                                break;
                            default: // "full"
                                scale = Math.min(scaleX, scaleY);
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                model.y = (app.screen.height - model.height) / 2;
                                break;
                        }
                    };
                    fitModel();

                    // Handle resize
                    app.renderer.on('resize', fitModel);

                    // Ensure model is interactive
                    model.interactive = true;

                    // Hit area tap events → interaction reactions
                    model.on("hit", (hitAreas: string[]) => {
                        hitAreas.forEach((area) => {
                            onHitAreaTap?.(area);
                            // Feed to interaction service for emotion/motion/line reactions
                            const ctrl = getActiveController();
                            if (ctrl) {
                                interactionService.handleTouch(area, ctrl);
                            }
                        });
                    });

                    app.stage.addChild(model as unknown as PIXI.DisplayObject);

                    // Add update loop
                    app.ticker.add((delta) => {
                        const ctrl = getActiveController();
                        if (ctrl) {
                            // Delta in Pixi is frame-dependent (1 = 60fps). 
                            // Controller might expect seconds or ms?
                            // let's assume controller.update expects delta 
                            // We'll pass the raw delta for now (frames)
                            ctrl.update(delta);
                        }
                    });

                } catch (err) {
                    console.error("[Live2DViewer] Failed to load model:", err);
                }
            };

            loadModel();

            // Cleanup
            return () => {
                cancelled = true;
                const model = modelRef.current;
                if (model) {
                    model.destroy();
                    modelRef.current = null;
                }

                app.stage.off("pointermove", handlePointerMove);
                try {
                    app.destroy(true, { children: true, texture: true });
                } catch (e) {
                    // Ignore destroy errors
                }
                appRef.current = null;
            };
        }, [modelUrl, backgroundAlpha, displayMode, handlePointerMove, onHitAreaTap, getActiveController]);

        return (
            <div
                ref={containerRef}
                className={className}
                style={{ width: "100%", height: "100%", overflow: "hidden" }}
            />
        );
    }
);

Live2DViewer.displayName = "Live2DViewer";
export default Live2DViewer;
