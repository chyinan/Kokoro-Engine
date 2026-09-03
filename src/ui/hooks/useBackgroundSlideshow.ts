import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { db } from "../../lib/db";

export type RotationMode = "sequential" | "random";
export type BackgroundMode = "slideshow" | "generated" | "static";

export interface BackgroundConfig {
    enabled: boolean;
    blur: boolean;
    blurAmount: number; // px
    interval: number;   // seconds
    rotation: RotationMode;
    mode: BackgroundMode;
}

export const DEFAULT_BACKGROUND_CONFIG: BackgroundConfig = {
    enabled: true,
    blur: false,
    blurAmount: 8,
    interval: 30,
    rotation: "sequential",
    mode: "static",
};

const DEFAULT_BACKGROUND_URL = "/backgrounds/default-cozy-room.png";

export function normalizeBackgroundConfigForImageCount(config: BackgroundConfig, imageCount: number): BackgroundConfig {
    if (imageCount === 0 && config.mode === "slideshow") {
        return { ...config, mode: "static" };
    }

    return config;
}

function loadConfig(): BackgroundConfig {
    try {
        const raw = localStorage.getItem("kokoro_bg_config");
        if (raw) return { ...DEFAULT_BACKGROUND_CONFIG, ...JSON.parse(raw) };
    } catch { /* ignore */ }
    return { ...DEFAULT_BACKGROUND_CONFIG };
}

function saveConfig(config: BackgroundConfig) {
    localStorage.setItem("kokoro_bg_config", JSON.stringify(config));
}

export function useBackgroundSlideshow() {
    const [config, setConfigState] = useState<BackgroundConfig>(loadConfig);
    const [storedImages, setStoredImages] = useState<{ id: number, url: string }[]>([]);
    const [imagesLoaded, setImagesLoaded] = useState(false);

    // Derived URL string array for consumption
    const images = useMemo(() => storedImages.map(img => img.url), [storedImages]);

    const [currentIndex, setCurrentIndex] = useState(0);
    const [currentUrl, setCurrentUrl] = useState<string | null>(null);
    const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
    const indexRef = useRef(0);

    // Load images from DB on mount
    useEffect(() => {
        let active = true;

        async function loadImages() {
            try {
                const items = await db.getAllImages();
                if (!active) return;

                const loaded = items.map(item => ({
                    id: item.id,
                    url: URL.createObjectURL(item.blob)
                }));
                setStoredImages(loaded);
                setImagesLoaded(true);

                if (loaded.length === 0) {
                    setConfigState(prev => {
                        const next = normalizeBackgroundConfigForImageCount(prev, 0);
                        if (next === prev) return prev;
                        saveConfig(next);
                        return next;
                    });
                }
            } catch (error) {
                console.error("Failed to load background images:", error);
                if (active) {
                    setImagesLoaded(true);
                    setConfigState(prev => {
                        const next = normalizeBackgroundConfigForImageCount(prev, 0);
                        if (next === prev) return prev;
                        saveConfig(next);
                        return next;
                    });
                }
            }
        }

        loadImages();

        return () => {
            active = false;
        };
    }, []);

    // Persist config changes
    const setConfig = useCallback((update: Partial<BackgroundConfig>) => {
        setConfigState(prev => {
            const merged = { ...prev, ...update };
            const next = imagesLoaded
                ? normalizeBackgroundConfigForImageCount(merged, storedImages.length)
                : merged;
            saveConfig(next);
            return next;
        });
    }, [imagesLoaded, storedImages.length]);

    // Import files from <input type="file">
    const importFiles = useCallback(async (fileList: FileList) => {
        const validTypes = ["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp"];
        const validExts = [".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp"];
        const isValidImage = (file: File) => {
            if (validTypes.includes(file.type)) return true;
            const ext = file.name.toLowerCase().match(/\.[^.]+$/)?.[0];
            return ext ? validExts.includes(ext) : false;
        };
        // 在第一个 await 前先复制，防止 WebView2 重置 input 后 FileList 失效
        const files = Array.from(fileList);
        const newItems: { id: number, url: string }[] = [];

        for (const file of files) {
            if (isValidImage(file)) {
                try {
                    const id = await db.addImage(file);
                    const url = URL.createObjectURL(file);
                    newItems.push({ id, url });
                } catch (e) {
                    console.error("Failed to save image to DB:", e);
                }
            }
        }

        if (newItems.length > 0) {
            setStoredImages(prev => [...prev, ...newItems]);
        }

        return newItems.length;
    }, []);

    // Remove a single image
    const removeImage = useCallback(async (index: number) => {
        const item = storedImages[index];
        if (!item) {
            throw new Error(`Image not found at index ${index}`);
        }

        try {
            await db.deleteImage(item.id);
            URL.revokeObjectURL(item.url);

            setStoredImages(prev => prev.filter((_, i) => i !== index));
            if (storedImages.length <= 1) {
                setConfigState(prev => {
                    const next = normalizeBackgroundConfigForImageCount(prev, 0);
                    if (next === prev) return prev;
                    saveConfig(next);
                    return next;
                });
            }
        } catch (e) {
            console.error("Failed to delete image:", e);
            throw e;
        }
    }, [storedImages]);

    // Clear all images
    const clearImages = useCallback(async () => {
        try {
            await db.clearAll();
            storedImages.forEach(img => URL.revokeObjectURL(img.url));
            setStoredImages([]);
            setCurrentIndex(0);
            setCurrentUrl(null);
            setConfigState(prev => {
                const next = normalizeBackgroundConfigForImageCount(prev, 0);
                if (next === prev) return prev;
                saveConfig(next);
                return next;
            });
        } catch (e) {
            console.error("Failed to clear images:", e);
            throw e;
        }
    }, [storedImages]);

    // Pick next image
    const nextImage = useCallback(() => {
        if (images.length === 0) {
            setCurrentUrl(null);
            return;
        }

        if (images.length === 1) {
            setCurrentUrl(images[0]);
            return;
        }

        let nextIdx: number;
        if (config.rotation === "random") {
            // Avoid showing the same image twice in a row
            do {
                nextIdx = Math.floor(Math.random() * images.length);
            } while (nextIdx === indexRef.current && images.length > 1);
        } else {
            nextIdx = (indexRef.current + 1) % images.length;
        }

        indexRef.current = nextIdx;
        setCurrentIndex(nextIdx);
        setCurrentUrl(images[nextIdx]);
    }, [images, config.rotation]);

    // Set initial image when images change
    useEffect(() => {
        if (images.length > 0 && !currentUrl) {
            const startIdx = config.rotation === "random"
                ? Math.floor(Math.random() * images.length)
                : 0;
            indexRef.current = startIdx;
            setCurrentIndex(startIdx);
            setCurrentUrl(images[startIdx]);
        } else if (images.length === 0) {
            setCurrentUrl(null);
        }
    }, [images.length]);

    // Slideshow timer
    useEffect(() => {
        if (timerRef.current) {
            clearInterval(timerRef.current);
            timerRef.current = null;
        }

        if (config.enabled && config.mode === "slideshow" && images.length > 1 && config.interval > 0) {
            timerRef.current = setInterval(nextImage, config.interval * 1000);
        }

        return () => {
            if (timerRef.current) clearInterval(timerRef.current);
        };
    }, [config.enabled, config.mode, config.interval, images.length, nextImage]);

    return {
        config,
        setConfig,
        images,
        currentUrl: config.enabled ? (currentUrl ?? DEFAULT_BACKGROUND_URL) : null,
        currentIndex,
        importFiles,
        removeImage,
        clearImages,
        nextImage,
        imageCount: images.length,
    };
}
