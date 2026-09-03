// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi, beforeEach } from "vitest";
import ConversationSidebar from "../ConversationSidebar";
import { listConversations, deleteConversation } from "../../../lib/kokoro-bridge";
import type { Conversation } from "../../../lib/kokoro-bridge";

vi.mock("framer-motion", () => ({
    motion: {
        div: forwardRef(({ children, className, onClick, ...props }: any, ref: any) => {
            const domProps: any = {};
            for (const [key, value] of Object.entries(props)) {
                if (key.startsWith("data-") || key === "aria-hidden") {
                    domProps[key] = value;
                }
            }
            return createElement("div", { ref, className, onClick, ...domProps }, children);
        }),
    },
    AnimatePresence: ({ children }: any) => children,
}));

vi.mock("react-i18next", () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("../../../lib/kokoro-bridge", () => ({
    listConversations: vi.fn(async () => []),
    deleteConversation: vi.fn(async () => undefined),
    renameConversation: vi.fn(async () => undefined),
    updateConversationState: vi.fn(async () => undefined),
    getConversationDisplayTitle: vi.fn((c: any) => c.title || "Untitled"),
    hasPinnedConversationState: vi.fn(() => false),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const sampleConversation: Conversation = {
    id: "conv-1",
    character_id: "char-1",
    title: "Test Conversation Title",
    topic: "",
    pinned_state: "{}",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
};

describe("ConversationSidebar", () => {
    let container: HTMLDivElement;
    let root: ReturnType<typeof createRoot>;

    beforeEach(() => {
        vi.clearAllMocks();
        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
        return () => {
            root.unmount();
            container.remove();
        };
    });

    const renderSidebar = async (
        onClose = vi.fn(),
        activeConversationId: string | null = null,
        onSelectConversation = vi.fn(async () => undefined),
    ) => {
        await act(async () => {
            root.render(
                createElement(ConversationSidebar, {
                    open: true,
                    onClose,
                    characterId: "char-1",
                    activeConversationId,
                    onStartEmptyConversation: vi.fn(),
                    onSelectConversation,
                })
            );
        });
        await act(async () => {
            await new Promise(resolve => setTimeout(resolve, 0));
        });
        return { onClose, onSelectConversation };
    };

    describe("click-outside and dismissal", () => {
        it("calls onClose when clicking outside the sidebar drawer (on document)", async () => {
            const { onClose } = await renderSidebar();

            const backdrop = container.querySelector('[data-testid="conversation-sidebar-backdrop"]');
            expect(backdrop).not.toBeNull();

            await act(async () => {
                document.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
            });

            expect(onClose).toHaveBeenCalled();
        });

        it("does NOT call onClose when clicking inside the sidebar drawer", async () => {
            const { onClose } = await renderSidebar();

            const newChatBtn = container.querySelector("button");
            expect(newChatBtn).not.toBeNull();

            await act(async () => {
                newChatBtn?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
            });

            expect(onClose).not.toHaveBeenCalled();
        });

        it("calls onClose when pressing the Escape key", async () => {
            const { onClose } = await renderSidebar();

            await act(async () => {
                window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
            });

            expect(onClose).toHaveBeenCalled();
        });

        it("does NOT call onClose from outside click if clicking the history toggle button", async () => {
            const toggleBtn = document.createElement("button");
            toggleBtn.setAttribute("data-chat-history-toggle", "true");
            document.body.appendChild(toggleBtn);

            const { onClose } = await renderSidebar();

            await act(async () => {
                toggleBtn.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
            });

            expect(onClose).not.toHaveBeenCalled();
            toggleBtn.remove();
        });

        it("calls onClose when clicking on the backdrop overlay", async () => {
            const { onClose } = await renderSidebar();

            const backdrop = container.querySelector('[data-testid="conversation-sidebar-backdrop"]');
            expect(backdrop).not.toBeNull();

            await act(async () => {
                backdrop?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            });

            expect(onClose).toHaveBeenCalled();
        });
    });

    describe("delete confirmation logic", () => {
        beforeEach(() => {
            vi.mocked(listConversations).mockResolvedValue([sampleConversation]);
        });

        it("does NOT immediately delete conversation when trash icon is clicked; opens confirmation modal", async () => {
            await renderSidebar();

            const deleteBtn = container.querySelector('button[title="chat.history.delete"]');
            expect(deleteBtn).not.toBeNull();

            await act(async () => {
                deleteBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            });

            // deleteConversation should NOT have been called yet!
            expect(deleteConversation).not.toHaveBeenCalled();

            // The confirmation modal must be open
            expect(container.textContent).toContain("chat.history.confirmDelete");
            expect(container.textContent).toContain("Test Conversation Title");
        });

        it("cancels deletion when clicking the cancel button in the confirmation modal", async () => {
            await renderSidebar();

            const deleteBtn = container.querySelector('button[title="chat.history.delete"]');
            await act(async () => {
                deleteBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            });

            expect(container.textContent).toContain("chat.history.confirmDelete");

            // Find cancel button in the modal
            const cancelBtn = Array.from(container.querySelectorAll("button")).find(
                b => b.textContent?.includes("chat.actions.cancel")
            );
            expect(cancelBtn).toBeDefined();

            await act(async () => {
                cancelBtn?.click();
                await new Promise(resolve => setTimeout(resolve, 0));
            });

            // Modal should be closed
            expect(container.textContent).not.toContain("chat.history.confirmDelete");
            expect(deleteConversation).not.toHaveBeenCalled();
        });

        it("cancels deletion on Escape key without closing the sidebar", async () => {
            const onClose = vi.fn();
            await renderSidebar(onClose);

            const deleteBtn = container.querySelector('button[title="chat.history.delete"]');
            await act(async () => {
                deleteBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            });

            expect(container.textContent).toContain("chat.history.confirmDelete");

            await act(async () => {
                window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
            });

            // Modal closed, but sidebar remains open!
            expect(container.textContent).not.toContain("chat.history.confirmDelete");
            expect(onClose).not.toHaveBeenCalled();
            expect(deleteConversation).not.toHaveBeenCalled();
        });

        it("executes delete and clears active conversation when confirmed", async () => {
            const onSelectConversation = vi.fn(async () => undefined);
            await renderSidebar(vi.fn(), "conv-1", onSelectConversation);

            const deleteBtn = container.querySelector('button[title="chat.history.delete"]');
            await act(async () => {
                deleteBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
            });

            // Find confirm delete button in modal
            const confirmBtn = Array.from(container.querySelectorAll("button")).find(
                b => b.textContent?.trim() === "chat.history.delete"
            );
            expect(confirmBtn).toBeDefined();

            await act(async () => {
                confirmBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
                await new Promise(resolve => setTimeout(resolve, 0));
            });

            expect(deleteConversation).toHaveBeenCalledWith("conv-1");
            expect(onSelectConversation).toHaveBeenCalledWith(null);
            expect(listConversations).toHaveBeenCalledTimes(2); // Initial + refresh
        });
    });
});
