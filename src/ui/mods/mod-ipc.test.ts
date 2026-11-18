import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { invokeModCommand } from "./mod-ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
    vi.mocked(invoke).mockReset();
});

describe("MOD IPC boundary", () => {
    it("routes a known request through its generated command and keeps legacy defaults", async () => {
        await invokeModCommand("list_dream_proposals", { request: { character_id: "character" } });

        expect(invoke).toHaveBeenCalledWith("list_dream_proposals", {
            request: { character_id: "character", status: null },
        }, undefined);
    });

    it("rejects invalid input before sending it through a typed command", async () => {
        await expect(invokeModCommand("play_cue", { cue: 1 })).rejects.toThrow("invalid args");
        expect(invoke).not.toHaveBeenCalled();
    });

    it("keeps historical unknown commands from reaching the transport", async () => {
        await expect(invokeModCommand("play_motion", {})).rejects.toBe("Command play_motion not found");
        expect(invoke).not.toHaveBeenCalled();
    });

    it("uses the generated MOD endpoint for the legacy config request", async () => {
        const config = {
            active_provider: "local",
            providers: [{ id: "local", provider_type: "ollama", extra: { options: { max_tokens: 80 } } }],
        };
        await invokeModCommand("test_llm_connection", { config });

        expect(invoke).toHaveBeenCalledWith("test_mod_llm_connection", { config }, undefined);
    });

    it("does not expose the internal command name to the iframe", async () => {
        await expect(invokeModCommand("test_mod_llm_connection", {}))
            .rejects.toBe("Command test_mod_llm_connection not found");
        expect(invoke).not.toHaveBeenCalled();
    });
});
