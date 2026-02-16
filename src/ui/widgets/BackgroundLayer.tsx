import { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";

interface BackgroundLayerProps {
    imageUrl: string | null;
    blur: boolean;
    blurAmount: number;
}

export default function BackgroundLayer({ imageUrl, blur, blurAmount }: BackgroundLayerProps) {
    // Track previous image for crossfade
    const [displayUrl, setDisplayUrl] = useState<string | null>(imageUrl);
    const [fadeKey, setFadeKey] = useState(0);

    useEffect(() => {
        if (imageUrl !== displayUrl) {
            setFadeKey(prev => prev + 1);
            setDisplayUrl(imageUrl);
        }
    }, [imageUrl]);

    if (!displayUrl) return null;

    return (
        <div className="absolute inset-0 z-0 overflow-hidden">
            <AnimatePresence mode="sync">
                <motion.div
                    key={fadeKey}
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 1.2, ease: "easeInOut" }}
                    className="absolute inset-0"
                >
                    <img
                        src={displayUrl}
                        alt=""
                        className={clsx(
                            "w-full h-full object-cover",
                            blur && "transition-[filter] duration-500"
                        )}
                        style={{
                            filter: blur ? `blur(${blurAmount}px)` : "none",
                            // Scale up slightly when blurred to hide edge artifacts
                            transform: blur ? "scale(1.05)" : "scale(1)",
                        }}
                        draggable={false}
                    />
                    {/* Dark overlay so UI elements remain readable */}
                    <div className="absolute inset-0 bg-black/40" />
                </motion.div>
            </AnimatePresence>
        </div>
    );
}
