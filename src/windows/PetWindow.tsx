import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import Live2DViewer from "../features/live2d/Live2DViewer";
import { Live2DController } from "../features/live2d/Live2DController";
import PetContextMenu from "../features/pet/PetContextMenu";
import { usePetChat } from "../features/pet/usePetChat";
import type { Live2DViewerHandle } from "../features/live2d/Live2DViewer";
import "../ui/i18n";
import { live2dUrl } from "../lib/utils";
import { BUILTIN_LIVE2D_MODEL_PATH, setActiveLive2dModel } from "../lib/kokoro-bridge";

type ResizeDirection = "East" | "North" | "NorthEast" | "NorthWest" | "South" | "SouthEast" | "SouthWest" | "West";

interface PetConfig {
    enabled: boolean;
    position_x: number;
    position_y: number;
    shortcut: string;
    model_url: string | null;
    window_width: number;
    window_height: number;
    model_scale?: number;
    render_fps?: number;
}

interface Live2dSelectionEvent {
    modelPath: string;
    customModelPath: string | null;
    modelUrl: string;
}

export default function PetWindow() {
    const currentWindow = getCurrentWindow();
    const getModelSelection = () => {
        const savedPath = localStorage.getItem("kokoro_custom_model_path");
        return {
            modelPath: savedPath ?? BUILTIN_LIVE2D_MODEL_PATH,
            modelUrl: savedPath ? live2dUrl(savedPath) : "/live2d/haru/haru_greeter_t03.model3.json",
        };
    };
    const [{ modelUrl, modelPath }, setModelSelection] = useState(getModelSelection);

    useEffect(() => {
        const onStorage = (e: StorageEvent) => {
            if (e.key === "kokoro_custom_model_path") {
                setModelSelection(getModelSelection());
            }
        };
        window.addEventListener("storage", onStorage);
        return () => window.removeEventListener("storage", onStorage);
    }, []);

    useEffect(() => {
        const unlisten = listen<Live2dSelectionEvent>("live2d-model-selection-updated", (event) => {
            setModelSelection({
                modelPath: event.payload.modelPath,
                modelUrl: event.payload.modelUrl,
            });
        });

        return () => {
            unlisten.then(fn => fn()).catch(console.error);
        };
    }, []);

    useEffect(() => {
        setActiveLive2dModel(modelPath).catch((error) => {
            console.error("[PetWindow] Failed to sync active Live2D model:", error);
        });
    }, [modelPath]);
    const [isDragMode, setIsDragMode] = useState(false);
    const [isResizeMode, setIsResizeMode] = useState(false);
    const [contextMenu, setContextMenu] = useState<{ visible: boolean; x: number; y: number }>({
        visible: false, x: 0, y: 0,
    });
    const [chatInputVisible, setChatInputVisible] = useState(false);
    const [chatInput, setChatInput] = useState("");
    const [canvasSize, setCanvasSize] = useState<{ width: number; height: number } | null>(null);
    const viewerRef = useRef<Live2DViewerHandle>(null);
    const controllerRef = useRef<Live2DController>(new Live2DController());
    const inputRef = useRef<HTMLInputElement>(null);
    const isDragModeRef = useRef(false);
    const isResizeModeRef = useRef(false);
    const rightClickStartRef = useRef<{ x: number; y: number } | null>(null);
    const userScaleMultiplierRef = useRef(1);
    const hasSavedSizeRef = useRef(false);
    const [configLoaded, setConfigLoaded] = useState(false);
    const [scaleMultiplier, setScaleMultiplier] = useState(1);
    const [renderFps, setRenderFps] = useState(60);
    const { isStreaming, sendMessage } = usePetChat();

    // On mount: restore saved window size from config
    useEffect(() => {
        invoke<PetConfig>("get_pet_config").then(cfg => {
            if (cfg.window_width >= 100 && cfg.window_height >= 100) {
                hasSavedSizeRef.current = true;
                setCanvasSize({ width: cfg.window_width, height: cfg.window_height });
                invoke("resize_pet_window", { width: cfg.window_width, height: cfg.window_height }).catch(console.error);
            }
            const savedMultiplier = cfg.model_scale && cfg.model_scale > 0 ? cfg.model_scale : 1;
            userScaleMultiplierRef.current = savedMultiplier;
            setScaleMultiplier(savedMultiplier);
            setRenderFps(typeof cfg.render_fps === "number" ? cfg.render_fps : 60);
            setConfigLoaded(true);
        }).catch(() => {
            setConfigLoaded(true); // 即使失败也要允许渲染
        });
    }, []);

    useEffect(() => {
        const unlisten = listen<PetConfig>("pet-config-updated", (event) => {
            const cfg = event.payload;

            if (typeof cfg.model_scale === "number" && cfg.model_scale > 0) {
                userScaleMultiplierRef.current = cfg.model_scale;
                setScaleMultiplier(cfg.model_scale);
            }

            if (typeof cfg.render_fps === "number") {
                setRenderFps(cfg.render_fps);
            }
        });

        return () => {
            unlisten.then(fn => fn()).catch(console.error);
        };
    }, []);

    // Handle model loaded - auto-fit only on first launch (no saved size)
    const handleModelLoaded = useCallback(async (bounds: { width: number; height: number }) => {
        console.log("[PetWindow] Model loaded with natural bounds:", bounds);

        // Skip auto-fit if user already has a saved size
        if (hasSavedSizeRef.current) {
            console.log("[PetWindow] Saved size exists, skipping auto-fit");
            return;
        }

        const padding = 30;
        const newWidth = Math.ceil(bounds.width + padding);
        const newHeight = Math.ceil(bounds.height + padding);

        console.log("[PetWindow] First launch, setting canvas size to:", newWidth, "x", newHeight);
        setCanvasSize({ width: newWidth, height: newHeight });

        try {
            await invoke("resize_pet_window", { width: newWidth, height: newHeight });
            // Save initial size
            invoke<PetConfig>("get_pet_config").then(cfg => {
                invoke("save_pet_config", { config: { ...cfg, window_width: newWidth, window_height: newHeight } }).catch(console.error);
            }).catch(console.error);
        } catch (e) {
            console.error("[PetWindow] Failed to resize window:", e);
        }
    }, []);

    // Keep ref in sync for use in native event listeners
    useEffect(() => {
        isDragModeRef.current = isDragMode;
        isResizeModeRef.current = isResizeMode;
    }, [isDragMode, isResizeMode]);

    // Resize mode: drag edges to resize window + Ctrl+Wheel to fine-tune model scale
    useEffect(() => {
        if (!isResizeMode) return;
        currentWindow.setResizable(true).catch(console.error);

        const EDGE_SIZE = 10; // px from edge to detect resize

        const detectEdge = (e: MouseEvent): 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw' | null => {
            const rect = document.body.getBoundingClientRect();
            const x = e.clientX;
            const y = e.clientY;

            const nearTop = y < EDGE_SIZE;
            const nearBottom = y > rect.height - EDGE_SIZE;
            const nearLeft = x < EDGE_SIZE;
            const nearRight = x > rect.width - EDGE_SIZE;

            if (nearTop && nearLeft) return 'nw';
            if (nearTop && nearRight) return 'ne';
            if (nearBottom && nearLeft) return 'sw';
            if (nearBottom && nearRight) return 'se';
            if (nearTop) return 'n';
            if (nearBottom) return 's';
            if (nearLeft) return 'w';
            if (nearRight) return 'e';
            return null;
        };

        const getCursor = (edge: ReturnType<typeof detectEdge>): string => {
            if (!edge) return 'default';
            const cursors = {
                n: 'ns-resize',
                s: 'ns-resize',
                e: 'ew-resize',
                w: 'ew-resize',
                ne: 'nesw-resize',
                nw: 'nwse-resize',
                se: 'nwse-resize',
                sw: 'nesw-resize',
            };
            return cursors[edge];
        };

        const getResizeDirection = (edge: NonNullable<ReturnType<typeof detectEdge>>): ResizeDirection => {
            const directions: Record<NonNullable<ReturnType<typeof detectEdge>>, ResizeDirection> = {
                n: "North", s: "South", e: "East", w: "West",
                ne: "NorthEast", nw: "NorthWest", se: "SouthEast", sw: "SouthWest",
            };
            return directions[edge];
        };

        const handleMouseMove = (e: MouseEvent) => {
            const edge = detectEdge(e);
            document.body.style.cursor = getCursor(edge);
        };

        const handleMouseDown = async (e: MouseEvent) => {
            const edge = detectEdge(e);
            if (edge) {
                e.preventDefault();
                try {
                    await currentWindow.startResizeDragging(getResizeDirection(edge));
                } catch (err) {
                    console.error("[PetWindow] startResizeDragging failed:", err);
                }
            }
        };

        const handleMouseUp = () => {
            document.body.style.cursor = 'default';
        };

        const handleWheel = (e: WheelEvent) => {
            if (e.ctrlKey) {
                e.preventDefault();

                const delta = e.deltaY > 0 ? -0.02 : 0.02;
                const nextMultiplier = Math.max(0.5, Math.min(2.5, userScaleMultiplierRef.current + delta));
                userScaleMultiplierRef.current = nextMultiplier;
                setScaleMultiplier(nextMultiplier);

                invoke<PetConfig>("get_pet_config").then(cfg => {
                    invoke("save_pet_config", {
                        config: { ...cfg, model_scale: nextMultiplier }
                    }).catch(console.error);
                }).catch(console.error);
            }
        };

        document.addEventListener('mousemove', handleMouseMove);
        document.addEventListener('mousedown', handleMouseDown);
        document.addEventListener('mouseup', handleMouseUp);
        document.addEventListener('wheel', handleWheel, { passive: false });

        return () => {
            currentWindow.setResizable(false).catch(console.error);
            document.removeEventListener('mousemove', handleMouseMove);
            document.removeEventListener('mousedown', handleMouseDown);
            document.removeEventListener('mouseup', handleMouseUp);
            document.removeEventListener('wheel', handleWheel);
            document.body.style.cursor = 'default';
        };
    }, [currentWindow, isResizeMode]);

    // Save window size after native resize completes
    useEffect(() => {
        if (!isResizeMode) return;

        let saveTimer: ReturnType<typeof setTimeout> | null = null;
        let unlisten: (() => void) | undefined;

        currentWindow.onResized(({ payload: size }) => {
            setCanvasSize({ width: size.width, height: size.height });
            if (saveTimer) clearTimeout(saveTimer);
            saveTimer = setTimeout(() => {
                invoke<PetConfig>("get_pet_config").then(cfg => {
                    invoke("save_pet_config", {
                        config: { ...cfg, window_width: size.width, window_height: size.height }
                    }).catch(console.error);
                }).catch(console.error);
            }, 150);
        }).then(fn => { unlisten = fn; }).catch(console.error);

        return () => {
            if (saveTimer) clearTimeout(saveTimer);
            unlisten?.();
        };
    }, [currentWindow, isResizeMode]);

    // Right-click: short click for menu, drag for window move
    useEffect(() => {
        let dragStartPos: { x: number; y: number } | null = null;
        let windowStartPos: { x: number; y: number } | null = null;
        let hasMoved = false;
        let pendingWindowPos: { x: number; y: number } | null = null;
        let moveRafId: number | null = null;
        let moveInFlight = false;

        const flushWindowMove = async () => {
            moveRafId = null;
            if (!pendingWindowPos || moveInFlight) return;

            const nextPos = pendingWindowPos;
            pendingWindowPos = null;
            moveInFlight = true;

            try {
                await currentWindow.setPosition(new PhysicalPosition(nextPos.x, nextPos.y));
            } catch (e) {
                console.error("[PetWindow] Failed to set position:", e);
            } finally {
                moveInFlight = false;
                if (pendingWindowPos && isDragModeRef.current && moveRafId === null) {
                    moveRafId = requestAnimationFrame(() => {
                        void flushWindowMove();
                    });
                }
            }
        };

        const scheduleWindowMove = (x: number, y: number) => {
            pendingWindowPos = { x, y };
            if (moveRafId !== null || moveInFlight) return;

            moveRafId = requestAnimationFrame(() => {
                void flushWindowMove();
            });
        };

        const handleMouseDown = async (e: MouseEvent) => {
            if (e.button === 2) { // Right button
                e.preventDefault();
                e.stopPropagation();
                dragStartPos = { x: e.screenX, y: e.screenY };
                hasMoved = false;
                rightClickStartRef.current = { x: e.clientX, y: e.clientY };

                // Get current window position
                try {
                    const pos = await currentWindow.outerPosition();
                    windowStartPos = { x: pos.x, y: pos.y };
                } catch (e) {
                    console.error("[PetWindow] Failed to get window position:", e);
                }

                // Enter drag mode
                setIsDragMode(true);
                isDragModeRef.current = true;
            }
        };

        const handleMouseMove = (e: MouseEvent) => {
            if (dragStartPos && windowStartPos && isDragModeRef.current) {
                const dx = e.screenX - dragStartPos.x;
                const dy = e.screenY - dragStartPos.y;

                // If moved more than 5px, consider it a drag
                if (Math.abs(dx) > 5 || Math.abs(dy) > 5) {
                    hasMoved = true;

                    // Move window
                    const newX = windowStartPos.x + dx;
                    const newY = windowStartPos.y + dy;
                    scheduleWindowMove(newX, newY);
                }
            }
        };

        const handleMouseUp = async (e: MouseEvent) => {
            if (e.button === 2) {
                e.preventDefault();
                e.stopPropagation();

                if (!hasMoved && rightClickStartRef.current) {
                    // No movement — show menu
                    const pos = rightClickStartRef.current;
                    setContextMenu({ visible: true, x: pos.x, y: pos.y });
                } else if (hasMoved && windowStartPos) {
                    const dx = e.screenX - dragStartPos!.x;
                    const dy = e.screenY - dragStartPos!.y;
                    const finalX = windowStartPos.x + dx;
                    const finalY = windowStartPos.y + dy;

                    if (moveRafId !== null) {
                        cancelAnimationFrame(moveRafId);
                        moveRafId = null;
                    }
                    pendingWindowPos = null;

                    try {
                        await currentWindow.setPosition(new PhysicalPosition(finalX, finalY));
                    } catch (err) {
                        console.error("[PetWindow] Failed to finalize position:", err);
                    }

                    invoke<PetConfig>("get_pet_config").then(cfg => {
                        invoke("save_pet_config", {
                            config: { ...cfg, position_x: finalX, position_y: finalY }
                        }).catch(console.error);
                    }).catch(console.error);
                }

                // Exit drag mode
                setIsDragMode(false);
                isDragModeRef.current = false;
                dragStartPos = null;
                windowStartPos = null;
                pendingWindowPos = null;
                rightClickStartRef.current = null;
            }
        };

        const handleContextMenu = (e: MouseEvent) => {
            e.preventDefault();
            e.stopPropagation();
        };

        // Use capture phase to intercept before canvas
        document.addEventListener("mousedown", handleMouseDown, true);
        document.addEventListener("mousemove", handleMouseMove, true);
        document.addEventListener("mouseup", handleMouseUp, true);
        document.addEventListener("contextmenu", handleContextMenu, true);

        return () => {
            if (moveRafId !== null) {
                cancelAnimationFrame(moveRafId);
            }
            document.removeEventListener("mousedown", handleMouseDown, true);
            document.removeEventListener("mousemove", handleMouseMove, true);
            document.removeEventListener("mouseup", handleMouseUp, true);
            document.removeEventListener("contextmenu", handleContextMenu, true);
        };
    }, [currentWindow]);

    // Native pointerup for drag exit — removed, now handled in enterDragMode
    // useEffect(() => {
    //     const handler = () => {
    //         if (isDragModeRef.current) {
    //             setTimeout(exitDragMode, 100);
    //         }
    //     };
    //     document.addEventListener("pointerup", handler);
    //     return () => document.removeEventListener("pointerup", handler);
    // }, []);

    // Make PIXI canvas transparent via CSS injection + disable canvas pointer events for right-click
    useEffect(() => {
        const style = document.createElement("style");
        style.textContent = `
            * { margin: 0; padding: 0; box-sizing: border-box; }
            html, body, #root {
                width: 100%;
                height: 100%;
                background: transparent !important;
                overflow: hidden;
            }
            canvas {
                background: transparent !important;
                display: block;
            }
        `;
        document.head.appendChild(style);

        // Disable pointer events on canvas after it's created
        const checkCanvas = setInterval(() => {
            const canvas = document.querySelector("canvas");
                if (canvas) {
                    (canvas as HTMLCanvasElement).style.pointerEvents = "none";
                    clearInterval(checkCanvas);
                }
        }, 100);

        return () => {
            document.head.removeChild(style);
            clearInterval(checkCanvas);
        };
    }, []);

    // Listen for toggle-chat-input from global shortcut
    useEffect(() => {
        const unlisten = listen("toggle-chat-input", () => {
            setChatInputVisible(v => !v);
            setTimeout(() => inputRef.current?.focus(), 50);
        });
        return () => { unlisten.then(fn => fn()); };
    }, []);

    // exitDragMode removed — now handled inline in long press handler

    const handleSendChat = useCallback(async () => {
        const text = chatInput.trim();
        if (!text) return;
        setChatInput("");
        setChatInputVisible(false);
        await sendMessage(text);
    }, [chatInput, sendMessage]);

    const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
        if (e.key === "Enter") handleSendChat();
        if (e.key === "Escape") setChatInputVisible(false);
    }, [handleSendChat]);

    const contextMenuItems = useMemo(() => [
        {
            label: isResizeMode ? "退出调整大小模式" : "调整窗口大小",
            onClick: () => {
                setIsResizeMode(!isResizeMode);
                setContextMenu(m => ({ ...m, visible: false }));
            }
        },
        { label: "隐藏悬浮模型", onClick: () => invoke("hide_pet_window").catch(console.error) },
        {
            label: "打开主界面",
            onClick: () => getCurrentWindow().emit("show-main-window", {}).catch(console.error),
        },
    ], [isResizeMode]);

    return (
        <div style={{ width: "100vw", height: "100vh", position: "relative", overflow: "hidden", background: "transparent" }}>
            {/* Drag overlay — always present but only visible in drag mode */}
            <div
                data-tauri-drag-region
                style={{
                    position: "absolute",
                    inset: 0,
                    zIndex: isDragMode ? 100 : -1,
                    pointerEvents: isDragMode ? "auto" : "none",
                    cursor: isDragMode ? "grab" : "default",
                    background: "transparent",
                }}
            >
                {isDragMode && (
                    <div style={{
                        position: "absolute",
                        bottom: "16px",
                        left: "50%",
                        transform: "translateX(-50%)",
                        background: "rgba(0,0,0,0.6)",
                        color: "#fff",
                        borderRadius: "8px",
                        padding: "6px 14px",
                        fontSize: "12px",
                        backdropFilter: "blur(8px)",
                        pointerEvents: "none",
                    }}>
                        拖动中 · 松手退出
                    </div>
                )}
            </div>

            {/* Resize mode indicator and border */}
            {isResizeMode && (
                <>
                    {/* Red border overlay */}
                    <div style={{
                        position: "absolute",
                        inset: 0,
                        border: "2px solid rgba(255, 0, 0, 0.6)",
                        pointerEvents: "none",
                        zIndex: 150,
                    }} />

                    {/* Instruction tooltip */}
                    <div style={{
                        position: "absolute",
                        top: "16px",
                        left: "50%",
                        transform: "translateX(-50%)",
                        background: "rgba(0,0,0,0.7)",
                        color: "#fff",
                        borderRadius: "8px",
                        padding: "8px 16px",
                        fontSize: "12px",
                        backdropFilter: "blur(8px)",
                        pointerEvents: "none",
                        zIndex: 200,
                        textAlign: "center",
                    }}>
                        <div>调整大小模式</div>
                        <div style={{ fontSize: "11px", marginTop: "4px", opacity: 0.8 }}>
                            拖动边缘调整窗口大小 · Ctrl+滚轮微调模型
                        </div>
                    </div>
                </>
            )}

            {/* Live2D layer — only render after config is loaded to avoid scale race condition */}
            <div style={{ position: "absolute", inset: 0, pointerEvents: isDragMode ? "none" : "auto" }}>
                {configLoaded && <Live2DViewer
                    ref={viewerRef}
                    modelUrl={modelUrl}
                    modelPath={modelPath}
                    // Live2DViewer owns the chat-cue subscriptions
                    // and drives this shared controller directly.
                    controller={controllerRef.current}
                    backgroundAlpha={0}
                    displayMode="full"
                    gazeTracking={true}
                    fixedSize={canvasSize || undefined}
                    scaleMultiplier={scaleMultiplier}
                    maxFps={renderFps}
                    onModelLoaded={handleModelLoaded}
                />}
            </div>

            {/* Chat input overlay */}
            {chatInputVisible && (
                <div
                    style={{
                        position: "fixed", inset: 0, zIndex: 60,
                        display: "flex", alignItems: "flex-end",
                        justifyContent: "center", paddingBottom: "20px",
                    }}
                    onClick={(e) => { if (e.target === e.currentTarget) setChatInputVisible(false); }}
                >
                    <div style={{
                        display: "flex", gap: "8px",
                        width: "calc(100% - 24px)",
                        background: "rgba(20,20,30,0.92)", borderRadius: "12px",
                        padding: "10px", border: "1px solid rgba(255,255,255,0.12)",
                        backdropFilter: "blur(12px)", boxSizing: "border-box",
                    }}>
                        <input
                            ref={inputRef}
                            value={chatInput}
                            onChange={e => setChatInput(e.target.value)}
                            onKeyDown={handleKeyDown}
                            placeholder="说点什么..."
                            disabled={isStreaming}
                            style={{
                                flex: 1, minWidth: 0, background: "transparent", border: "none", outline: "none",
                                color: "#fff", fontSize: "13px",
                            }}
                        />
                        <button
                            onClick={handleSendChat}
                            disabled={isStreaming || !chatInput.trim()}
                            style={{
                                flexShrink: 0,
                                background: "rgba(255,255,255,0.15)", border: "none", borderRadius: "8px",
                                color: "#fff", padding: "4px 12px", cursor: "pointer", fontSize: "12px",
                            }}
                        >
                            发送
                        </button>
                    </div>
                </div>
            )}

            {/* Context menu */}
            <PetContextMenu
                visible={contextMenu.visible}
                x={contextMenu.x}
                y={contextMenu.y}
                onClose={() => setContextMenu(m => ({ ...m, visible: false }))}
                items={contextMenuItems}
            />
        </div>
    );
}
