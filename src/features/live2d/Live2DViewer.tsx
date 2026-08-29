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
import { Live2DController, type IdleBehavior } from "./Live2DController";
import { APP_SETTING_KEYS, readJsonSetting } from "../../lib/app-settings";
import type { AppliedCharacterCueProfile } from "../characters/character-cue-profile";
import { drawableHitTest, estimateRegionByY, REGION_DESCRIPTIONS } from "./DrawableHitTest";
import { onChatCue, type Live2dModelProfile } from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";
import { interactionService, type GestureEvent } from "../../core/services/interaction-service";
import * as PIXI from "pixi.js";

PIXI.utils.skipHello();
Live2DModel.registerTicker(PIXI.Ticker);

// ── Types ──────────────────────────────────────────

export interface Live2DViewerHandle {
    playCue: (cue: string) => void;
    /** @deprecated Use controller directly */
    setMouthOpen: (val: number) => void;
    /** @deprecated Use controller directly */
    playMotion: (group: string, index?: number) => void;
    getModel: () => Live2DModel | null;
    getController: () => Live2DController | undefined;
    fitToView: () => void;
}

export type Live2DDisplayMode = "full" | "upper" | "upper-thigh";

const RIGHT_DRAG_HOLD_MS = 250;
const RIGHT_DRAG_MOVE_THRESHOLD_PX = 3;

function readStoredHorizontalOffset(storageKey?: string): number {
    if (!storageKey || typeof window === "undefined") return 0;

    const value = Number(window.localStorage.getItem(storageKey));
    return Number.isFinite(value) ? value : 0;
}

function saveStoredHorizontalOffset(storageKey: string | undefined, offset: number) {
    if (!storageKey || typeof window === "undefined") return;

    window.localStorage.setItem(storageKey, String(Math.round(offset)));
}

export interface Live2DViewerProps {
    /** URL to the .model3.json file */
    modelUrl: string;
    /** Relative imported-model path used for cue profile loading. */
    modelPath?: string | null;
    /** Optional controller instance to manage the model state.
     * Chat cue events are routed through this controller when provided. */
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
    /** Enables right-button long-press horizontal model dragging. */
    enableHorizontalDrag?: boolean;
    /** Optional localStorage key used to persist horizontal drag offset. */
    horizontalOffsetStorageKey?: string;
}

// ── Component ──────────────────────────────────────

const Live2DViewer = forwardRef<Live2DViewerHandle, Live2DViewerProps>(
    ({ modelUrl, modelPath = null, controller, onHitAreaTap, className, backgroundAlpha = 0, displayMode = "full", gazeTracking = true, fixedSize, scaleMultiplier = 1, maxFps = 60, onModelLoaded, enableHorizontalDrag = false, horizontalOffsetStorageKey }, ref) => {
        const containerRef = useRef<HTMLDivElement>(null);
        const appRef = useRef<PIXI.Application | null>(null);
        const modelRef = useRef<Live2DModel | null>(null);
        const gazeTrackingRef = useRef(gazeTracking);
        const fitModelRef = useRef<(() => void) | null>(null);
        const scaleMultiplierRef = useRef(scaleMultiplier);
        const horizontalDragEnabledRef = useRef(enableHorizontalDrag);
        const horizontalOffsetStorageKeyRef = useRef(horizontalOffsetStorageKey);
        const horizontalOffsetRef = useRef(readStoredHorizontalOffset(horizontalOffsetStorageKey));

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

        useEffect(() => {
            const ctrl = getActiveController();
            if (!ctrl) return;
            void ctrl.loadProfileForModel(modelPath).then(() => {
                const cached = readJsonSetting<AppliedCharacterCueProfile>(
                    APP_SETTING_KEYS.characterCueProfileCache,
                    { cueMap: {}, semanticCueMap: {} },
                );
                const profile = ctrl.getModelProfile();
                if (!profile) return;
                ctrl.setProfile({
                    ...profile,
                    cue_map: { ...profile.cue_map, ...cached.cueMap },
                    semantic_cue_map: { ...profile.semantic_cue_map, ...cached.semanticCueMap },
                });
            });
        }, [getActiveController, modelPath]);

        useEffect(() => {
            const applyCharacterCues = (event: Event): void => {
                const ctrl = getActiveController();
                const profile = ctrl?.getModelProfile();
                if (!ctrl || !profile) return;
                const detail = (event as CustomEvent<AppliedCharacterCueProfile>).detail;
                ctrl.setProfile({
                    ...profile,
                    cue_map: { ...profile.cue_map, ...(detail?.cueMap ?? {}) },
                    semantic_cue_map: {
                        ...profile.semantic_cue_map,
                        ...(detail?.semanticCueMap ?? {}),
                    },
                });
            };
            window.addEventListener("kokoro-character-cue-profile-changed", applyCharacterCues);
            return () => window.removeEventListener(
                "kokoro-character-cue-profile-changed",
                applyCharacterCues,
            );
        }, [getActiveController]);

        useEffect(() => {
            let unlisten: (() => void) | undefined;

            listen<Live2dModelProfile>("live2d-profile-updated", (event) => {
                const ctrl = getActiveController();
                if (!ctrl || !modelPath) return;
                if (event.payload?.model_path !== modelPath) return;
                ctrl.setProfile(event.payload);
            }).then(fn => { unlisten = fn; });

            return () => { unlisten?.(); };
        }, [getActiveController, modelPath]);

        // Expose control methods to parent
        useImperativeHandle(ref, () => {
            const ctrl = getActiveController();
            return {
                playCue(cue: string) {
                    ctrl?.playCue(cue);
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

        // Centralize chat-driven cue listeners here so both the main stage
        // and the floating pet window react through the same controller instance.
        useEffect(() => {
            let unlisten: (() => void) | undefined;

            onChatCue((data) => {
                const ctrl = getActiveController();
                if (ctrl) {
                    void ctrl.playCue(data.cue);
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
            horizontalDragEnabledRef.current = enableHorizontalDrag;
            horizontalOffsetStorageKeyRef.current = horizontalOffsetStorageKey;
            horizontalOffsetRef.current = enableHorizontalDrag
                ? readStoredHorizontalOffset(horizontalOffsetStorageKey)
                : 0;
            fitModelRef.current?.();
        }, [enableHorizontalDrag, horizontalOffsetStorageKey]);

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
            let removeHorizontalDragListeners: (() => void) | null = null;

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
                    let fittedBaseX = 0;

                    const clampHorizontalOffset = (baseX: number, offset: number) => {
                        if (!horizontalDragEnabledRef.current) return 0;
                        if (app.screen.width <= 0 || model.width <= 0) return offset;

                        const minVisibleWidth = Math.min(
                            Math.max(app.screen.width * 0.12, 80),
                            model.width,
                            220
                        );
                        const minOffset = minVisibleWidth - model.width - baseX;
                        const maxOffset = app.screen.width - minVisibleWidth - baseX;
                        return Math.min(Math.max(offset, minOffset), maxOffset);
                    };

                    const setHorizontalOffset = (offset: number) => {
                        const nextOffset = clampHorizontalOffset(fittedBaseX, offset);
                        horizontalOffsetRef.current = nextOffset;
                        model.x = fittedBaseX + nextOffset;
                        return nextOffset;
                    };

                    const setModelBaseX = (baseX: number) => {
                        fittedBaseX = baseX;
                        setHorizontalOffset(horizontalOffsetRef.current);
                    };

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
                            setModelBaseX((app.screen.width - model.width) / 2);
                            model.y = (app.screen.height - model.height) / 2;
                            return;
                        }

                        switch (displayMode) {
                            case "upper":
                                scale = Math.min(scaleX, scaleY) * 1.5 * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                setModelBaseX((app.screen.width - model.width) / 2);
                                model.y = app.screen.height * 0.05;
                                break;
                            case "upper-thigh":
                                scale = Math.min(scaleX, scaleY) * 1.25 * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                setModelBaseX((app.screen.width - model.width) / 2);
                                model.y = app.screen.height * 0.03;
                                break;
                            default:
                                scale = Math.min(scaleX, scaleY) * scaleMultiplierRef.current;
                                model.scale.set(scale);
                                setModelBaseX((app.screen.width - model.width) / 2);
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
                        // Level 1: Prefer model-defined HitAreas when available.
                        const hits = model.hitTest(globalX, globalY);
                        if (hits.length > 0) {
                            if (import.meta.env.DEV) {
                                console.log(`[HitTest] source=hitarea | hits=${hits.join(", ")}`);
                            }
                            return hits[0];
                        }

                        // Level 2: Drawable mesh hit test — front-most visible mesh wins.
                        // null = nothing hit; "unknown" = hit an unrecognised mesh (still on model).
                        const region = drawableHitTest(model, globalX, globalY, import.meta.env.DEV);
                        if (region !== null && region !== "unknown") {
                            if (import.meta.env.DEV) {
                                console.log(`[HitTest] source=drawable | region=${region}`);
                            }
                            return REGION_DESCRIPTIONS[region];
                        }

                        // Level 3: Y-coordinate estimation — only inside model bounds.
                        // This is also the fallback for drawable="unknown" hits.
                        const bounds = model.getBounds();
                        const inBounds =
                            globalX >= bounds.x && globalX <= bounds.x + bounds.width &&
                            globalY >= bounds.y && globalY <= bounds.y + bounds.height;
                        if (!inBounds) return null;

                        const fallback = estimateRegionByY(model, globalY);
                        if (import.meta.env.DEV) {
                            const source = region === "unknown" ? "drawable-unknown" : "geometry-fallback";
                            console.log(`[HitTest] source=${source} | region=${fallback}`);
                        }
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

                    const canvas = app.view as HTMLCanvasElement;
                    const clientToGlobal = (clientX: number, clientY: number) => {
                        const rect = canvas.getBoundingClientRect();
                        if (rect.width <= 0 || rect.height <= 0) {
                            return { x: 0, y: 0 };
                        }

                        return {
                            x: ((clientX - rect.left) / rect.width) * app.screen.width,
                            y: ((clientY - rect.top) / rect.height) * app.screen.height,
                        };
                    };
                    const isPointOnModel = (clientX: number, clientY: number) => {
                        const { x, y } = clientToGlobal(clientX, clientY);
                        const bounds = model.getBounds();
                        return (
                            x >= bounds.x &&
                            x <= bounds.x + bounds.width &&
                            y >= bounds.y &&
                            y <= bounds.y + bounds.height
                        );
                    };

                    let rightDragPointerId: number | null = null;
                    let rightDragTimer: ReturnType<typeof setTimeout> | null = null;
                    let rightDragActive = false;
                    let rightDragStartClientX = 0;
                    let rightDragLatestClientX = 0;
                    let rightDragStartOffset = 0;

                    const clearRightDragTimer = () => {
                        if (rightDragTimer) {
                            clearTimeout(rightDragTimer);
                            rightDragTimer = null;
                        }
                    };
                    const applyRightDrag = (clientX: number) => {
                        setHorizontalOffset(rightDragStartOffset + clientX - rightDragStartClientX);
                    };
                    const activateRightDrag = (clientX: number) => {
                        if (rightDragActive) return;

                        clearRightDragTimer();
                        rightDragStartClientX = clientX;
                        rightDragStartOffset = horizontalOffsetRef.current;
                        rightDragActive = true;
                        container.style.cursor = "ew-resize";
                    };
                    const endRightDrag = (event: PointerEvent) => {
                        if (rightDragPointerId !== event.pointerId) return;

                        event.preventDefault();
                        event.stopPropagation();
                        clearRightDragTimer();

                        if (rightDragActive) {
                            applyRightDrag(event.clientX);
                            saveStoredHorizontalOffset(
                                horizontalOffsetStorageKeyRef.current,
                                horizontalOffsetRef.current
                            );
                        }

                        try {
                            if (container.hasPointerCapture(event.pointerId)) {
                                container.releasePointerCapture(event.pointerId);
                            }
                        } catch {
                            // Pointer capture can already be released by the browser.
                        }

                        rightDragPointerId = null;
                        rightDragActive = false;
                        container.style.cursor = "";
                    };
                    const handleRightDragPointerDown = (event: PointerEvent) => {
                        if (!horizontalDragEnabledRef.current || event.button !== 2 || rightDragPointerId !== null) return;
                        if (!isPointOnModel(event.clientX, event.clientY)) return;

                        event.preventDefault();
                        event.stopPropagation();

                        rightDragPointerId = event.pointerId;
                        rightDragStartClientX = event.clientX;
                        rightDragLatestClientX = event.clientX;
                        rightDragStartOffset = horizontalOffsetRef.current;

                        try {
                            container.setPointerCapture(event.pointerId);
                        } catch {
                            // Pointer capture is best-effort; document-level pointer events still bubble here.
                        }

                        rightDragTimer = setTimeout(() => {
                            rightDragTimer = null;
                            if (rightDragPointerId !== event.pointerId) return;

                            activateRightDrag(rightDragLatestClientX);
                        }, RIGHT_DRAG_HOLD_MS);
                    };
                    const handleRightDragPointerMove = (event: PointerEvent) => {
                        if (rightDragPointerId !== event.pointerId) return;

                        event.preventDefault();
                        event.stopPropagation();
                        rightDragLatestClientX = event.clientX;

                        if (!rightDragActive) {
                            const dx = event.clientX - rightDragStartClientX;
                            if (Math.abs(dx) < RIGHT_DRAG_MOVE_THRESHOLD_PX) return;

                            activateRightDrag(event.clientX);
                            return;
                        }

                        if (rightDragActive) {
                            applyRightDrag(event.clientX);
                        }
                    };
                    const handleRightDragContextMenu = (event: MouseEvent) => {
                        if (!horizontalDragEnabledRef.current) return;
                        if (rightDragPointerId === null && !isPointOnModel(event.clientX, event.clientY)) return;

                        event.preventDefault();
                        event.stopPropagation();
                    };

                    container.addEventListener("pointerdown", handleRightDragPointerDown, true);
                    container.addEventListener("pointermove", handleRightDragPointerMove, true);
                    container.addEventListener("pointerup", endRightDrag, true);
                    container.addEventListener("pointercancel", endRightDrag, true);
                    container.addEventListener("contextmenu", handleRightDragContextMenu, true);
                    removeHorizontalDragListeners = () => {
                        clearRightDragTimer();
                        container.style.cursor = "";
                        container.removeEventListener("pointerdown", handleRightDragPointerDown, true);
                        container.removeEventListener("pointermove", handleRightDragPointerMove, true);
                        container.removeEventListener("pointerup", endRightDrag, true);
                        container.removeEventListener("pointercancel", endRightDrag, true);
                        container.removeEventListener("contextmenu", handleRightDragContextMenu, true);
                    };

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
                removeHorizontalDragListeners?.();

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
