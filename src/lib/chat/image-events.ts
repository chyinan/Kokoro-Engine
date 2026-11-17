import type { ChatImageGenEvent, ChatViewSnapshot, ImageGenResult } from "../kokoro-bridge";
import { chatEventMatchesView } from "./projection";

interface ImageEventDependencies {
  getView(): ChatViewSnapshot;
  generate(prompt: string): Promise<ImageGenResult>;
  showImage(result: ImageGenResult): void;
  selectGeneratedBackground(): void;
  onError(error: unknown): void;
}

/** Used by App's real listeners; recheck ownership after asynchronous generation. */
export function createImageEventHandlers(deps: ImageEventDependencies) {
  return {
    onImageDone(result: ImageGenResult): void {
      if (chatEventMatchesView(deps.getView(), result)) deps.showImage(result);
    },
    async onBackgroundRequested(event: ChatImageGenEvent): Promise<void> {
      try {
        const result = await deps.generate(event.prompt);
        if (!chatEventMatchesView(deps.getView(), event)) return;
        deps.showImage(result);
        deps.selectGeneratedBackground();
      } catch (error) {
        deps.onError(error);
      }
    },
  };
}
