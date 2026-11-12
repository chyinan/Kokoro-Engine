import { useEffect, useRef, useState } from "react";
import { useTypingReveal } from "../../hooks";
import type { ChatPanelMessage } from "../../../lib/chat/turn-state";

/** A display-only animation. Neither completion nor cancellation waits for it. */
export function useChatTextReveal(messages: ChatPanelMessage[], activeTurnId?: string | null): ChatPanelMessage[] {
  const fullText = messages.find(message => message.role === "kokoro" && message.turnId === activeTurnId)?.text ?? "";
  const previous = useRef<{ id: string | null; text: string; base: string }>({ id: null, text: "", base: "" });
  const [display, setDisplay] = useState<{ id: string | null; text: string }>({ id: null, text: "" });
  const reveal = useTypingReveal({
    active: Boolean(activeTurnId),
    onReveal: text => setDisplay({ id: previous.current.id, text: previous.current.base + text }),
  });
  const revealRef = useRef(reveal); revealRef.current = reveal;
  useEffect(() => {
    const id = activeTurnId ?? null;
    if (previous.current.id !== id) {
      revealRef.current.reset();
      // A late subscriber starts at the complete recovered text, then animates
      // only subsequent additions. A fresh request starts with an empty base.
      previous.current = { id, text: fullText, base: fullText };
      setDisplay({ id, text: fullText });
    } else if (fullText !== previous.current.text) {
      if (fullText.startsWith(previous.current.text)) revealRef.current.pushDelta(fullText.slice(previous.current.text.length));
      else {
        revealRef.current.reset(); previous.current.base = fullText;
        setDisplay({ id, text: fullText });
      }
      previous.current.text = fullText;
    }
    if (!id) revealRef.current.flush();
  }, [activeTurnId, fullText]);
  if (!activeTurnId || display.id !== activeTurnId) return messages;
  return messages.map(message => message.role === "kokoro" && message.turnId === activeTurnId ? { ...message, text: display.text } : message);
}
