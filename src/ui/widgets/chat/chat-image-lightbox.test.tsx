// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { ImageLightbox } from "../../components/ImageLightbox";
import { ChatMessage } from "../ChatMessage";

vi.mock("framer-motion", () => ({
    motion: {
        div: forwardRef(({ children, className, onClick, onWheel, style, ...props }: any, ref: any) => {
            const domProps: any = {};
            for (const [key, value] of Object.entries(props)) {
                if (key.startsWith("data-") || key === "aria-hidden") {
                    domProps[key] = value;
                }
            }
            return createElement("div", { ref, className, onClick, onWheel, style, ...domProps }, children);
        }),
    },
    AnimatePresence: ({ children }: any) => children,
}));

vi.mock("react-i18next", () => ({
    useTranslation: () => ({ t: (key: string, def?: string) => def || key }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("ImageLightbox Component", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;

    beforeEach(() => {
        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
        return () => {
            root.unmount();
            container.remove();
        };
    });

    const renderLightbox = async (imageUrl: string | null = "https://example.com/test.png", onClose = vi.fn()) => {
        await act(async () => {
            root.render(createElement(ImageLightbox, { imageUrl, onClose }));
        });
        return { onClose };
    };

    it("renders nothing when imageUrl is null", async () => {
        await renderLightbox(null);
        expect(container.querySelector('[data-testid="image-lightbox-img"]')).toBeNull();
    });

    it("renders image with preview URL when imageUrl is provided", async () => {
        await renderLightbox("https://example.com/photo.png");
        const img = container.querySelector('[data-testid="image-lightbox-img"]') as HTMLImageElement;
        expect(img).not.toBeNull();
        expect(img.src).toBe("https://example.com/photo.png");
    });

    it("calls onClose when clicking close button", async () => {
        const { onClose } = await renderLightbox();
        const closeBtn = Array.from(container.querySelectorAll("button")).find(
            b => b.title === "关闭" || b.title === "chat.actions.cancel"
        );
        expect(closeBtn).toBeDefined();

        await act(async () => {
            closeBtn?.click();
        });

        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("calls onClose when pressing Escape key", async () => {
        const { onClose } = await renderLightbox();

        await act(async () => {
            window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        });

        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("calls onClose when clicking backdrop overlay", async () => {
        const { onClose } = await renderLightbox();
        const backdrop = container.querySelector('[data-testid="image-lightbox-backdrop"]') as HTMLDivElement;
        expect(backdrop).not.toBeNull();

        await act(async () => {
            backdrop.click();
        });

        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("does NOT call onClose when clicking on image itself", async () => {
        const { onClose } = await renderLightbox();
        const img = container.querySelector('[data-testid="image-lightbox-img"]') as HTMLImageElement;

        await act(async () => {
            img.click();
        });

        expect(onClose).not.toHaveBeenCalled();
    });

    it("supports zoom in, zoom out, rotate, and reset buttons", async () => {
        await renderLightbox("https://example.com/diagram.png");
        const img = container.querySelector('[data-testid="image-lightbox-img"]') as HTMLImageElement;

        const zoomInBtn = Array.from(container.querySelectorAll("button")).find(b => b.title === "放大");
        const zoomOutBtn = Array.from(container.querySelectorAll("button")).find(b => b.title === "缩小");
        const rotateBtn = Array.from(container.querySelectorAll("button")).find(b => b.title === "旋转");
        const resetBtn = Array.from(container.querySelectorAll("button")).find(b => b.title === "重置");

        expect(zoomInBtn).toBeDefined();
        expect(zoomOutBtn).toBeDefined();
        expect(rotateBtn).toBeDefined();
        expect(resetBtn).toBeDefined();

        // Initial style
        expect(img.style.transform).toBe("scale(1) rotate(0deg)");

        // Click zoom in
        await act(async () => {
            zoomInBtn?.click();
        });
        expect(img.style.transform).toBe("scale(1.25) rotate(0deg)");

        // Click rotate
        await act(async () => {
            rotateBtn?.click();
        });
        expect(img.style.transform).toBe("scale(1.25) rotate(90deg)");

        // Click zoom out
        await act(async () => {
            zoomOutBtn?.click();
        });
        expect(img.style.transform).toBe("scale(1) rotate(90deg)");

        // Click reset
        await act(async () => {
            resetBtn?.click();
        });
        expect(img.style.transform).toBe("scale(1) rotate(0deg)");
    });
});

describe("ChatMessage Image Preview Trigger", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;

    beforeEach(() => {
        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
        return () => {
            root.unmount();
            container.remove();
        };
    });

    it("triggers onPreviewImage when image thumbnail is clicked in ChatMessage", async () => {
        const onPreviewImage = vi.fn();
        const testImageUrl = "https://example.com/screenshot.png";

        await act(async () => {
            root.render(
                createElement(ChatMessage, {
                    message: {
                        role: "user",
                        text: "Check this screenshot",
                        images: [testImageUrl],
                    },
                    index: 0,
                    isStreaming: false,
                    isTranslationExpanded: false,
                    onToggleTranslation: vi.fn(),
                    onEdit: vi.fn(),
                    onRegenerate: vi.fn(),
                    onContinueFrom: vi.fn(),
                    onApproveTool: vi.fn(),
                    onRejectTool: vi.fn(),
                    onPreviewImage,
                })
            );
        });

        const imgThumbnail = container.querySelector('img[src="https://example.com/screenshot.png"]');
        expect(imgThumbnail).not.toBeNull();

        const thumbnailWrapper = imgThumbnail?.closest(".cursor-zoom-in") as HTMLElement;
        expect(thumbnailWrapper).not.toBeNull();

        await act(async () => {
            thumbnailWrapper.click();
        });

        expect(onPreviewImage).toHaveBeenCalledWith(testImageUrl);
    });
});
