// pattern: Imperative Shell
import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, ZoomIn, ZoomOut, RotateCw, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";

export interface ImageLightboxProps {
    imageUrl: string | null;
    onClose: () => void;
}

export function ImageLightbox({ imageUrl, onClose }: ImageLightboxProps) {
    const { t } = useTranslation();
    const [scale, setScale] = useState(1);
    const [rotation, setRotation] = useState(0);

    const handleReset = useCallback(() => {
        setScale(1);
        setRotation(0);
    }, []);

    // 每次打开新图时重置缩放和旋转
    useEffect(() => {
        if (imageUrl) {
            handleReset();
        }
    }, [imageUrl, handleReset]);

    // 键盘 Esc 关闭
    useEffect(() => {
        if (!imageUrl) return;
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                onClose();
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [imageUrl, onClose]);

    // 鼠标滚轮缩放
    const handleWheel = (e: React.WheelEvent) => {
        e.stopPropagation();
        if (e.deltaY < 0) {
            setScale(prev => Math.min(prev + 0.15, 3.5));
        } else {
            setScale(prev => Math.max(prev - 0.15, 0.5));
        }
    };

    const handleZoomIn = (e: React.MouseEvent) => {
        e.stopPropagation();
        setScale(prev => Math.min(prev + 0.25, 3.5));
    };

    const handleZoomOut = (e: React.MouseEvent) => {
        e.stopPropagation();
        setScale(prev => Math.max(prev - 0.25, 0.5));
    };

    const handleRotate = (e: React.MouseEvent) => {
        e.stopPropagation();
        setRotation(prev => (prev + 90) % 360);
    };

    if (!imageUrl) return null;

    return (
        <AnimatePresence>
            <motion.div
                key="image-lightbox-backdrop"
                data-testid="image-lightbox-backdrop"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                onClick={onClose}
                onWheel={handleWheel}
                className="fixed inset-0 z-50 bg-black/85 backdrop-blur-md flex items-center justify-center p-4 select-none"
            >
                {/* 顶部悬浮工具条 */}
                <div
                    onClick={e => e.stopPropagation()}
                    className="absolute top-4 right-4 flex items-center gap-2 bg-slate-900/80 border border-white/10 rounded-full px-3 py-1.5 shadow-2xl backdrop-blur-md text-white/80 z-10"
                >
                    <button
                        type="button"
                        onClick={handleZoomIn}
                        className="p-1 hover:text-white hover:bg-white/10 rounded-full transition-colors"
                        title={t("chat.image.zoom_in", "放大")}
                    >
                        <ZoomIn size={16} />
                    </button>
                    <button
                        type="button"
                        onClick={handleZoomOut}
                        className="p-1 hover:text-white hover:bg-white/10 rounded-full transition-colors"
                        title={t("chat.image.zoom_out", "缩小")}
                    >
                        <ZoomOut size={16} />
                    </button>
                    <button
                        type="button"
                        onClick={handleRotate}
                        className="p-1 hover:text-white hover:bg-white/10 rounded-full transition-colors"
                        title={t("chat.image.rotate", "旋转")}
                    >
                        <RotateCw size={16} />
                    </button>
                    <button
                        type="button"
                        onClick={handleReset}
                        className="p-1 hover:text-white hover:bg-white/10 rounded-full transition-colors"
                        title={t("chat.image.reset", "重置")}
                    >
                        <RotateCcw size={16} />
                    </button>
                    <div className="w-px h-4 bg-white/20 my-auto" />
                    <button
                        type="button"
                        onClick={onClose}
                        className="p-1 hover:text-red-400 hover:bg-white/10 rounded-full transition-colors"
                        title={t("chat.actions.cancel", "关闭")}
                    >
                        <X size={16} />
                    </button>
                </div>

                {/* 图片视口 */}
                <motion.div
                    initial={{ scale: 0.9, opacity: 0 }}
                    animate={{ scale: 1, opacity: 1 }}
                    exit={{ scale: 0.9, opacity: 0 }}
                    onClick={e => e.stopPropagation()}
                    className="max-w-[90vw] max-h-[90vh] flex items-center justify-center overflow-hidden"
                >
                    <img
                        src={imageUrl}
                        alt="preview"
                        style={{
                            transform: `scale(${scale}) rotate(${rotation}deg)`,
                            transition: "transform 0.15s cubic-bezier(0.4, 0, 0.2, 1)",
                        }}
                        className="max-w-[90vw] max-h-[90vh] object-contain rounded-lg shadow-2xl pointer-events-auto cursor-grab active:cursor-grabbing"
                        data-testid="image-lightbox-img"
                    />
                </motion.div>
            </motion.div>
        </AnimatePresence>
    );
}
