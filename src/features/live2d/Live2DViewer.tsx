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
import { Live2DModel } from "pixi-live2d-display/cubism4";
import { Live2DController, type EmotionState, type ActionIntent, type IdleBehavior } from "./Live2DController";
import { drawableHitTest, estimateRegionByY, REGION_DESCRIPTIONS } from "./DrawableHitTest";
import { onChatExpression, onChatAction } from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";
import { interactionService, type GestureEvent } from "../../core/services/interaction-service";
import * as PIXI from "pixi.js";

PIXI.utils.skipHello();
Live2DModel.registerTicker(PIXI.Ticker);

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
    fitToView: () => void;
}

export type Live2DDisplayMode = "full" | "upper" | "upper-thigh";

export interface Live2DViewerProps {
    /** URL to the .model3.json file */
    modelUrl: string;
    /** Optional controller instance to manage the model state.
     * Chat expression/action events are routed through this controller when provided. */
    controller?: Live2DController;
    /** Called when a hit area on the model is tapped (legacy) */
    onHitAreaTap?: (hitArea: string) => void;
    /** CSS class for the container */
    className?: string;
    /** Background alpha (0-1), default 0 */
    backgroundAlpha?: number;
    /** Display mode: full body, upper body, upper body + thighs */
    displayMode?: Live2DDisplayMode;
    /** Whether the model's eyes follow the mouse cursor (default true) */
    gazeTracking?: boolean;
    /** Fixed canvas size (disables auto-resize), useful for pet window */
    fixedSize?: { width: number; height: number };
    /** Optional user scale multiplier applied on top of auto-fit */
    scaleMultiplier?: number;
    /** Max render FPS. Use 0 for unlimited. */
    maxFps?: number;
    /** Callback when model is loaded and sized */
    onModelLoaded?: (bounds: { width: number; height: number }) => void;
}

// ── Component ──────────────────────────────────────

const Live2DViewer = forwardRef<Live2DViewerHandle, Live2DViewerProps>(
    ({ modelUrl, controller, onHitAreaTap, className, backgroundAlpha = 0, displayMode = "full", gazeTracking = true, fixedSize, scaleMultiplier = 1, maxFps = 60, onModelLoaded }, ref) => {
        const containerRef = useRef<HTMLDivElement>(null);
        const appRef = useRef<PIXI.Application | null>(null);
        const modelRef = useRef<Live2DModel | null>(null);
        const gazeTrackingRef = useRef(gazeTracking);
        const fitModelRef = useRef<(() => void) | null>(null);
        const scaleMultiplierRef = useRef(scaleMultiplier);

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
                },
                fitToView() {
                    fitModelRef.current?.();
                }
            };
        });

        // Centralize chat-driven animation listeners here so both the main stage
        // and the floating pet window react through the same controller instance.
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

        // Keep action/motion routing colocated with expression routing to avoid
        // duplicate listeners in individual windows such as PetWindow.
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

        // Sync gazeTracking prop to ref (avoids recreating handlePointerMove)
        useEffect(() => {
            gazeTrackingRef.current = gazeTracking;
        }, [gazeTracking]);

        useEffect(() => {
            scaleMultiplierRef.current = scaleMultiplier;
            fitModelRef.current?.();
        }, [scaleMultiplier]);

        useEffect(() => {
            const app = appRef.current;
            if (!app) return;

            app.ticker.maxFPS = maxFps > 0 ? maxFps : 0;
        }, [maxFps]);

        // Gaze tracking: model eyes follow cursor
        const handlePointerMove = useCallback((e: PIXI.InteractionEvent) => {
            const model = modelRef.current;
            if (!model || !gazeTrackingRef.current) return;
            model.focus(e.data.global.x, e.data.global.y);
        }, []);

        useEffect(() => {
            const container = containerRef.current;
            if (!container) return;

            // Create PixiJS application
            const app = new PIXI.Application({
                backgroundAlpha: backgroundAlpha,
                resizeTo: fixedSize ? undefined : container,
                width: fixedSize?.width,
                height: fixedSize?.height,
                antialias: true,
                autoStart: true,
                powerPreference: fixedSize ? "low-power" : "high-performance",
            });
            app.ticker.maxFPS = maxFps > 0 ? maxFps : 0;
            appRef.current = app;
            container.appendChild(app.view as HTMLCanvasElement);

            // Enable interaction
            app.stage.interactive = true;
            app.stage.hitArea = app.screen;
            app.stage.on("pointermove", handlePointerMove);

            // Load the Live2D model
            let cancelled = false;
            let tick: ((delta: number) => void) | null = null;
            let syncTickerState: (() => void) | null = null;

            // Clear PIXI texture cache to avoid stale error results from previous loads
            PIXI.utils.clearTextureCache();

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

                        if (fixedSize) {
                            // Use model.width/height (not getBounds) to avoid animation padding
                            model.scale.set(1);
                            model.x = 0;
                            model.y = 0;
                            const naturalWidth = model.width;
                            const naturalHeight = model.height;
                            const fitScaleX = app.screen.width / naturalWidth;
                            const fitScaleY = app.screen.height / naturalHeight;
                            scale = Math.min(fitScaleX, fitScaleY) * scaleMultiplierRef.current;
                            model.scale.set(scale);
                            // Center both axes
                            model.x = (app.screen.width - model.width) / 2;
                            model.y = (app.screen.height - model.height) / 2;
                            return;
                        }

                        switch (displayMode) {
                            case "upper":
                                scale = Math.min(scaleX, scaleY) * 1.5 * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                model.y = app.screen.height * 0.05;
                                break;
                            case "upper-thigh":
                                scale = Math.min(scaleX, scaleY) * 1.25 * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                model.y = app.screen.height * 0.03;
                                break;
                            default:
                                scale = Math.min(scaleX, scaleY) * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                model.x = (app.screen.width - model.width) / 2;
                                model.y = (app.screen.height - model.height) / 2;
                                break;
                        }
                    };
                    fitModelRef.current = fitModel;
                    fitModel();

                    // Notify parent of model size (for pet window auto-sizing)
                    if (onModelLoaded) {
                        // Use the model's internal dimensions instead of getBounds()
                        // which includes extra space for animations
                        const modelWidth = model.width;
                        const modelHeight = model.height;

                        console.log("[Live2DViewer] Model dimensions:", {
                            width: modelWidth,
                            height: modelHeight,
                            scale: model.scale.x,
                            position: { x: model.x, y: model.y },
                            bounds: model.getBounds()
                        });

                        onModelLoaded({ width: modelWidth, height: modelHeight });
                    }

                    // Handle resize (only if not fixed size)
                    app.renderer.on('resize', fitModel);

                    // Ensure model is interactive
                    (model as any).interactive = true;

                    // ── Pointer-based gesture detection ──
                    // Replaces model.on("hit") with tap / long_press detection
                    const LONG_PRESS_MS = 600;
                    let pointerDownTime = 0;
                    let longPressTimer: ReturnType<typeof setTimeout> | null = null;
                    let longPressFired = false;

                    const hitTestFirst = (globalX: number, globalY: number): string | null => {
                        // Level 1: Drawable mesh hit test — front-most visible mesh wins.
                        // null = nothing hit; "unknown" = hit an unrecognised mesh (still on model).
                        const region = drawableHitTest(model, globalX, globalY, import.meta.env.DEV);
                        if (region !== null) {
                            return region === "unknown"
                                ? REGION_DESCRIPTIONS["body"]
                                : REGION_DESCRIPTIONS[region];
                        }

                        // Level 2: Original HitArea detection (for models that define them)
                        const hits = model.hitTest(globalX, globalY);
                        if (hits.length > 0) return hits[0];

                        // Level 3: Y-coordinate estimation — only inside model bounds
                        const bounds = model.getBounds();
                        const inBounds =
                            globalX >= bounds.x && globalX <= bounds.x + bounds.width &&
                            globalY >= bounds.y && globalY <= bounds.y + bounds.height;
                        if (!inBounds) return null;

                        const fallback = estimateRegionByY(model, globalY);
                        return REGION_DESCRIPTIONS[fallback];
                    };

                    const handleGesture = (hitArea: string, gesture: GestureEvent["gesture"]) => {
                        onHitAreaTap?.(hitArea);
                        const ctrl = getActiveController();
                        if (ctrl) {
                            const event: GestureEvent = {
                                hitArea,
                                gesture,
                                consecutiveTaps: 1,
                            };
                            interactionService.handleGesture(event, ctrl);
                        }
                    };

                    model.on("pointerdown", (e: PIXI.InteractionEvent) => {
                        if (e.data.button !== 0) return; // 只处理左键
                        pointerDownTime = Date.now();
                        longPressFired = false;
                        const { x, y } = e.data.global;

                        longPressTimer = setTimeout(() => {
                            longPressFired = true;
                            const area = hitTestFirst(x, y);
                            if (area) handleGesture(area, "long_press");
                        }, LONG_PRESS_MS);
                    });

                    model.on("pointerup", (e: PIXI.InteractionEvent) => {
                        if (e.data.button !== 0) return; // 只处理左键
                        if (longPressTimer) {
                            clearTimeout(longPressTimer);
                            longPressTimer = null;
                        }
                        // If long press already fired, ignore the up event
                        if (longPressFired) return;

                        const elapsed = Date.now() - pointerDownTime;
                        if (elapsed < LONG_PRESS_MS) {
                            const { x, y } = e.data.global;
                            const area = hitTestFirst(x, y);
                            if (area) handleGesture(area, "tap");
                        }
                    });

                    model.on("pointerupoutside", () => {
                        if (longPressTimer) {
                            clearTimeout(longPressTimer);
                            longPressTimer = null;
                        }
                    });

                    app.stage.addChild(model as unknown as PIXI.DisplayObject);

                    // Add update loop
                    tick = (delta: number) => {
                        const ctrl = getActiveController();
                        if (ctrl) {
                            ctrl.update(delta);
                        }
                    };
                    app.ticker.add(tick);

                    syncTickerState = () => {
                        if (document.hidden) {
                            app.stop();
                        } else {
                            app.start();
                        }
                    };

                    document.addEventListener("visibilitychange", syncTickerState);
                    syncTickerState();

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
                fitModelRef.current = null;

                app.stage.off("pointermove", handlePointerMove);
                if (syncTickerState) {
                    document.removeEventListener("visibilitychange", syncTickerState);
                }
                if (tick) {
                    app.ticker.remove(tick);
                }
                try {
                    app.destroy(true, { children: true, texture: true });
                } catch (e) {
                    // Ignore destroy errors
                }
                appRef.current = null;
            };
        }, [modelUrl, backgroundAlpha, displayMode, handlePointerMove, onHitAreaTap, getActiveController]);

        useEffect(() => {
            if (!fixedSize) return;

            const app = appRef.current;
            if (!app) return;

            app.renderer.resize(fixedSize.width, fixedSize.height);
        }, [fixedSize?.height, fixedSize?.width]);

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
