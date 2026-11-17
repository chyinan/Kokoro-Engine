import { describe, expect, it, vi } from "vitest";
import { emptyChatView } from "./application";
import { createImageEventHandlers } from "./image-events";
import type { ChatImageGenEvent, ChatViewSnapshot, ImageGenResult } from "../kokoro-bridge";

const scope = {
  character_id: "owner", conversation_id: "reserved", conversation_generation: 4,
  client_request_id: "request", turn_id: "turn",
};
const image: ImageGenResult = { image_url: "image.png", prompt: "garden", provider_id: "test" };
function fixture() {
  let view: ChatViewSnapshot = { ...emptyChatView, character_id: "owner", generation: 4 };
  const deps = {
    getView: () => view,
    generate: vi.fn(async () => image), showImage: vi.fn(),
    selectGeneratedBackground: vi.fn(), onError: vi.fn(),
  };
  return { deps, handlers: createImageEventHandlers(deps), set: (change: Partial<ChatViewSnapshot>) => { view = { ...view, ...change }; } };
}

describe("App image event handlers", () => {
  it("shows a reserved image before the journal or SQLite conversation arrives", () => {
    const { handlers, deps } = fixture();
    handlers.onImageDone({ ...image, ...scope });
    expect(deps.showImage).toHaveBeenCalledWith({ ...image, ...scope });
  });

  it("rejects later empty views, other roles and other conversations, but returns to a saved owner", () => {
    const { handlers, deps, set } = fixture();
    for (const next of [
      { generation: 5 },
      { character_id: "other", generation: 4 },
      { character_id: "owner", conversation_id: "other-session" },
    ]) {
      set(next);
      handlers.onImageDone({ ...image, ...scope });
    }
    expect(deps.showImage).not.toHaveBeenCalled();
    set({ character_id: "owner", conversation_id: "reserved", generation: 8 });
    handlers.onImageDone({ ...image, ...scope });
    expect(deps.showImage).toHaveBeenCalledOnce();
  });

  it("uses an existing journal owner without leaking it into a newer empty view", () => {
    const { handlers, deps, set } = fixture();
    set({ turns: [{ ...scope, revision: 1, user_message_id: null, assistant_message_id: null,
      message: "hidden", images: [], hidden: true, status: "streaming", text: "", translation: null,
      tools: [], error: null }] });
    handlers.onImageDone({ ...image, ...scope, conversation_generation: undefined });
    expect(deps.showImage).toHaveBeenCalledOnce();
    set({ generation: 5 });
    handlers.onImageDone({ ...image, ...scope });
    expect(deps.showImage).toHaveBeenCalledOnce();
  });

  it("keeps manual image and background actions available in an open conversation", async () => {
    const { handlers, deps, set } = fixture();
    set({ conversation_id: "open" });
    handlers.onImageDone({ ...image, character_id: "owner" });
    await handlers.onBackgroundRequested({ prompt: "garden", character_id: "owner" });
    expect(deps.showImage).toHaveBeenCalledTimes(2);
    expect(deps.selectGeneratedBackground).toHaveBeenCalledOnce();
    handlers.onImageDone({ ...image, character_id: "other" });
    await handlers.onBackgroundRequested({ prompt: "garden", character_id: "other" });
    expect(deps.showImage).toHaveBeenCalledTimes(2);
    expect(deps.generate).toHaveBeenCalledTimes(2);
  });

  it.each([false, true])("continues a background request received after switching away; return=%s", async returnToOwner => {
    const { handlers, deps, set } = fixture();
    set({ character_id: "other", conversation_id: "other-session" });
    let finish!: (result: ImageGenResult) => void;
    deps.generate.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const pending = handlers.onBackgroundRequested({ ...scope, prompt: "garden" });
    expect(deps.generate).toHaveBeenCalledWith("garden");
    if (returnToOwner) set({ character_id: "owner", conversation_id: "reserved", generation: 9 });
    finish(image);
    await pending;
    expect(deps.showImage).toHaveBeenCalledTimes(returnToOwner ? 1 : 0);
  });

  it.each(["stay", "new", "other", "return"])("rechecks background ownership after generation: %s", async mutation => {
    const { handlers, deps, set } = fixture();
    let finish!: (result: ImageGenResult) => void;
    deps.generate.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const event: ChatImageGenEvent = { ...scope, prompt: "garden" };
    if (mutation === "return") set({ conversation_id: "reserved" });
    const pending = handlers.onBackgroundRequested(event);
    if (mutation === "new") set({ generation: 5 });
    if (mutation === "other") set({ character_id: "other" });
    if (mutation === "return") {
      set({ conversation_id: "other" });
      set({ conversation_id: "reserved", generation: 9 });
    }
    finish(image);
    await pending;
    expect(deps.showImage).toHaveBeenCalledTimes(mutation === "stay" || mutation === "return" ? 1 : 0);
    expect(deps.selectGeneratedBackground).toHaveBeenCalledTimes(mutation === "stay" || mutation === "return" ? 1 : 0);
  });
});
