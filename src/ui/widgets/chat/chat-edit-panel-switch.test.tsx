// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import ChatPanel from "../ChatPanel";
import { chatViewFixture as chat } from "../../../lib/chat/view-test-fixture";
import { turn } from "../../../lib/chat/__tests__/harness";

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

it("keeps tool details expanded when the completion event arrives before the saved history", async () => {
    chat.reset();
    const snapshot = {
        ...turn("search", 2), character_id: "char-1", conversation_id: "conv-1",
        tools: [{ turn_id: "turn_search", tool: "search", result: { message: "found result" }, tool_trace_id: "0:0" }],
    };
    const user = { id: 1, role: "user", content: "question", created_at: "now" };
    chat.set({ revision: 2, busy: true, messages: [user], turns: [snapshot] });
    const completed = { ...snapshot, revision: 3, status: "completed" as const, assistant_message_id: 3 };
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const details = () => [...container.querySelectorAll("button")].find(button => button.textContent?.includes("chat.tools.header"));
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
        await act(async () => root.render(<ChatPanel />));
        const toggle = details();
        expect(toggle).toBeDefined();
        await act(async () => toggle!.click());
        expect(container.textContent).toContain("found result");

        // A terminal event updates the turn while the cached history is unchanged.
        await act(async () => chat.set({ busy: false, turns: [completed] }));
        expect(details()).toBe(toggle);
        expect(container.textContent).toContain("found result");

        await act(async () => chat.set({
            revision: 3,
            messages: [user, {
                id: 2, role: "tool", content: "saved result", created_at: "now",
                metadata: JSON.stringify({ turn_id: snapshot.turn_id, tool_name: "search", tool_trace_id: "0:0" }),
            }, {
                id: 3, role: "assistant", content: "answer", created_at: "now",
                metadata: JSON.stringify({ turn_id: snapshot.turn_id }),
            }],
        }));
        expect(details()).toBe(toggle);
        expect(container.textContent).toContain("saved result");
        expect(container.textContent).not.toContain("found result");
    } finally {
        await act(async () => root.unmount());
        scrollTo.mockRestore();
        container.remove();
    }
});
