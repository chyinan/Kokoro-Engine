// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useChatTextReveal } from "./use-chat-text-reveal";
import type { ChatPanelMessage } from "../../../lib/chat/turn-state";
vi.mock("../../hooks", () => {
  let text = "";
  return { useTypingReveal: ({ onReveal }: { onReveal: (text: string) => void }) => ({
    reset: () => { text = ""; }, flush: () => {},
    pushDelta: (delta: string) => { text += delta; onReveal(text); },
  }) };
});
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("animates only assistant text while preserving context and tools sharing the turn", async () => {
  let projected: ChatPanelMessage[] = [];
  function Display({ text }: { text: string }) {
    projected = useChatTextReveal([
      { role: "context", text: "camera evidence", turnId: "turn" },
      { role: "kokoro", text, turnId: "turn", tools: [{ tool: "clock", text: "12:00" }] },
    ], "turn");
    return null;
  }
  const container = document.createElement("div"), root = createRoot(container);
  await act(async () => root.render(createElement(Display, { text: "restored" })));
  expect(projected[1].text).toBe("restored");
  await act(async () => root.render(createElement(Display, { text: "restored plus delta" })));
  expect(projected[0].text).toBe("camera evidence");
  expect(projected[1]).toMatchObject({ text: "restored plus delta", tools: [{ text: "12:00" }] });
  await act(async () => root.unmount());
});
