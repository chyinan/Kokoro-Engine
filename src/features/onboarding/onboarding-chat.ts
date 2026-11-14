import type { ChatApplication } from "../../lib/chat/application";

/** Onboarding's dismissal rule, separate from chat execution and from React. */
export class OnboardingChat {
  private pending: { id: string; resolve: (text: string) => void } | null = null;
  constructor(private readonly chat: Pick<ChatApplication, "send" | "cancel">) {}
  start(message: string, characterId?: string): Promise<string> {
    if (this.pending) return Promise.reject(new Error("a chat turn is already in progress"));
    const id = `onboarding_${crypto.randomUUID()}`;
    return new Promise((resolve, reject) => {
      this.pending = { id, resolve };
      void this.chat.send({ message, character_id: characterId, client_request_id: id }).then(response => {
        if (this.pending?.id !== id) return;
        this.pending = null;
        if (response.status !== "completed" || !response.text?.trim()) reject(new Error(`chat turn ${response.status ?? "empty"}`));
        else resolve(response.text);
      }, error => {
        if (this.pending?.id !== id) return;
        this.pending = null;
        reject(error);
      });
    });
  }
  dismiss(): void {
    const pending = this.pending;
    this.pending = null;
    if (!pending) return;
    // Cancellation is valid before turn_id exists; no UI handshake is needed.
    void this.chat.cancel(pending.id, "onboarding_dismissed").catch(() => {});
    pending.resolve("");
  }
}
