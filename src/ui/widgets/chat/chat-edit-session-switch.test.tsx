// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement, forwardRef, type ComponentProps, type HTMLAttributes } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ChatPanel from "../ChatPanel";
import type { ChatMessage } from "../ChatMessage";
import * as bridge from "../../../lib/kokoro-bridge";

vi.mock("framer-motion", () => ({
  motion: {
    div: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(({ children, className, onClick }, ref) =>
      createElement("div", { ref, className, onClick }, children)),
    button: forwardRef<HTMLButtonElement, ComponentProps<"button">>(
      ({ children, onClick, disabled, title, "aria-label": ariaLabel }, ref) =>
        createElement("button", { ref, onClick, disabled, title, "aria-label": ariaLabel }, children)),
  },
  AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
}));

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../hooks", () => ({
  VoiceState: { Idle: "idle", Listening: "listening", Processing: "processing", Speaking: "speaking", Error: "error" },
  useVoiceInput: () => ({ state: "idle", volume: 0, partialText: "", start: vi.fn(), stop: vi.fn() }),
  useWakeWord: () => ({ state: "idle", isListening: false, start: vi.fn(), stop: vi.fn() }),
  useTypingReveal: () => ({ pushDelta: vi.fn(), flush: vi.fn(), reset: vi.fn() }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock("../../../core/services", () => ({ audioPlayer: { isPlaying: false } }));
vi.mock("../ChatMessage", () => ({
  ChatMessage: ({ message, onEdit }: ComponentProps<typeof ChatMessage>) =>
    createElement("article", { "data-message-id": "id" in message ? String(message.id) : undefined },
      createElement("span", null, message.text),
      createElement("button", { "data-edit-message": true, onClick: () => onEdit("edited in A") }, "Edit")),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function conversation(id: string): bridge.Conversation {
  return { id, character_id: "char-1", title: id, topic: "", pinned_state: "{}", created_at: "2026-09-12", updated_at: "2026-09-12" };
}

describe("ChatPanel edit result session ownership", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;
  let finishEdit: (result: bridge.EditConversationMessageResponse) => void;

  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    localStorage.setItem("kokoro_active_character_id", "char-1");
    vi.spyOn(bridge, "listConversations").mockResolvedValue([conversation("conv-A"), conversation("conv-B")]);
    vi.spyOn(bridge, "loadConversation").mockImplementation(async id => ({
      ...conversation(id),
      messages: [{ id: id === "conv-A" ? 101 : 201, role: "user", content: id === "conv-A" ? "original A" : "original B", created_at: "2026-09-12" }],
    }));
    vi.spyOn(bridge, "editConversationMessage").mockImplementation(() => new Promise(resolve => { finishEdit = resolve; }));
    vi.spyOn(bridge, "listCharacters").mockResolvedValue([]);
    vi.spyOn(bridge, "setVisionTextInputFocused").mockResolvedValue(undefined);
    vi.spyOn(bridge, "cancelChatTurn").mockResolvedValue(undefined);
    vi.spyOn(bridge, "onChatTurnAcknowledged").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnStart").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnDelta").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnFinish").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnTextComplete").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatError").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatWarning").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatFailure").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnTranslation").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onChatTurnTool").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onTelegramChatSync").mockResolvedValue(() => {});
    vi.spyOn(bridge, "onVisionObservation").mockResolvedValue(() => {});
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    localStorage.clear();
    sessionStorage.clear();
  });

  it("does not apply a delayed edit response to the same position in another conversation", async () => {
    await act(async () => root.render(createElement(ChatPanel)));
    expect(container.querySelector("article")?.textContent).toContain("original A");

    await act(async () => container.querySelector<HTMLButtonElement>("[data-edit-message]")?.click());
    expect(bridge.editConversationMessage).toHaveBeenCalledWith({
      conversation_id: "conv-A", message_id: 101, new_content: "edited in A",
    });

    await act(async () => {
      window.dispatchEvent(new CustomEvent("kokoro-character-runtime-changed", {
        detail: { runtime: { character_id: "char-1", character_name: "Character" }, target_conversation_id: "conv-B" },
      }));
    });
    expect(container.querySelector("article")?.textContent).toContain("original B");
    expect(container.querySelector("article")?.getAttribute("data-message-id")).toBe("201");

    await act(async () => finishEdit({ message_id: 101, updated_content: "edited in A" }));

    expect(container.querySelector("article")?.textContent).toContain("original B");
    expect(container.querySelector("article")?.getAttribute("data-message-id")).toBe("201");
  });

  it("sends without the previous conversation id after clearing history", async () => {
    vi.spyOn(bridge, "clearHistory").mockResolvedValue(undefined);
    vi.spyOn(bridge, "getMemoryEmbeddingModelStatus").mockResolvedValue({
      installed: true, repo_id: "test", download_url: "", install_dir: "", model_path: "", required_files: [], missing_files: [],
    });
    vi.spyOn(bridge, "streamChat").mockImplementation(() => new Promise(() => {}));
    await act(async () => root.render(createElement(ChatPanel)));
    expect(container.querySelector("article")?.textContent).toContain("original A");
    await act(async () => container.querySelector<HTMLButtonElement>('[title="chat.actions.clear"]')?.click());
    const confirm = Array.from(container.querySelectorAll("button")).find(button => button.textContent === "chat.actions.confirm_clear_button");
    expect(confirm).toBeDefined();
    await act(async () => confirm?.click());
    expect(bridge.clearHistory).toHaveBeenCalledOnce();
    expect(container.querySelector("article")).toBeNull();

    const input = container.querySelector("textarea");
    expect(input).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(input, "first new message");
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => container.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(bridge.streamChat).toHaveBeenCalledOnce();
    expect(vi.mocked(bridge.streamChat).mock.calls[0][0].conversation_id).toBeUndefined();
  });
});
