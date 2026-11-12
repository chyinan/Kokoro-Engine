// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import ChatPanel from "../ChatPanel";
import { chatViewFixture as chat } from "../../../lib/chat/view-test-fixture";

vi.mock("../../../lib/chat/application", async () => ({ chatApplication: (await import("../../../lib/chat/view-test-fixture")).chatViewFixture }));
vi.mock("@tauri-apps/api/core", async original => ({ ...await original<typeof import("@tauri-apps/api/core")>(), invoke: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("dismisses the old editor when another conversation replaces a message at the same position", async () => {
    chat.reset();
    const first = { id: 101, role: "user", content: "Message A", created_at: "now" };
    const second = { ...first, id: 201, content: "Message B" };
    chat.set({ messages: [first] });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const button = (action: string) => container.querySelector<HTMLButtonElement>(`button[title="chat.actions.${action}"]`);
    try {
        await act(async () => root.render(<ChatPanel />));
        await act(async () => button("edit")!.click());
        expect(button("save")).not.toBeNull();
        await act(async () => chat.set({ conversation_id: "conv-2", generation: 2, messages: [second] }));

        expect(button("save")).toBeNull();
        expect(container.textContent).toContain("Message B");
        expect(chat.edit).not.toHaveBeenCalled();

        await act(async () => button("edit")!.click());
        await act(async () => button("save")!.click());
        expect(chat.edit).toHaveBeenCalledExactlyOnceWith({ conversationId: "conv-2", generation: 2, messageId: 201 }, "Message B");
    } finally {
        await act(async () => root.unmount());
        container.remove();
    }
});
