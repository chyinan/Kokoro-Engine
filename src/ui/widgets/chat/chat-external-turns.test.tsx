// @vitest-environment node
// The former Panel allowlist/handshake is retired. Admission belongs to Rust;
// every display accepts the authoritative projection regardless of sender.
import { describe, expect, it } from "vitest";
import { harness, turn } from "../../../lib/chat/__tests__/harness";
import { projectMessages } from "../../../lib/chat/projection";

describe("external turns are application capabilities", () => {
  it.each(["onboarding_1", "mod_1", "pet_1", "interaction_1", "untracked-valid-request"])("projects %s without Panel authorization or cancellation", async id => {
    const h = harness(); h.set({ revision: 1, busy: true, turns: [turn(id, 1)] });
    const stop = h.app.subscribe(() => {}); await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot()).slice(-1)[0]?.text).toBe("answer");
    expect(h.io.cancel).not.toHaveBeenCalled(); expect(h.io.send).not.toHaveBeenCalled(); stop();
  });
  it("hidden proactive/interaction requests do not add a synthetic user message", async () => {
    const h = harness(); h.set({ revision: 1, turns: [{ ...turn("proactive_1", 1), hidden: true }] });
    await h.app.refresh();
    expect(projectMessages(h.app.getSnapshot()).map(message => message.role)).toEqual(["kokoro"]);
  });
  it("onboarding obtains completion without a display and does not depend on event delivery", async () => {
    const h = harness();
    await expect(h.app.send({ message: "first reply", client_request_id: "onboarding_1" })).resolves.toMatchObject({ text: "answer", status: "completed" });
    expect(h.io.listen).not.toHaveBeenCalled();
  });
});
