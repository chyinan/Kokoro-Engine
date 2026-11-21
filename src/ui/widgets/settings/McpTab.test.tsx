// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActionInfo, McpServerStatus, ToolSettings } from "../../../lib/kokoro-bridge";
import McpTab from "./McpTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("react-i18next", () => ({
    useTranslation: () => ({
        t: (key: string, options?: { defaultValue?: string }) =>
            key === "settings.mcp.status.tools" ? "tools" : options?.defaultValue ?? key,
    }),
}));

const initialServer: McpServerStatus = {
    name: "fixture",
    enabled: true,
    connected: true,
    status: "connected",
    tool_count: 2,
    server_version: null,
    error: null,
};

const initialTools: ActionInfo[] = ["search", "lookup"].map(name => ({
    id: `mcp__fixture__${name}`,
    name,
    source: "mcp",
    server_name: initialServer.name,
    description: "",
    parameters: [],
    needs_feedback: false,
    risk_tags: [],
    permission_level: "safe",
}));

const settings: ToolSettings = {
    max_tool_rounds: 10,
    enabled_tools: {},
    max_permission_level: "elevated",
    blocked_risk_tags: [],
};

describe("McpTab refresh", () => {
    let container: HTMLDivElement;
    let root: Root;
    let server: McpServerStatus;
    let tools: ActionInfo[];
    let delayedRead: Promise<void> | undefined;
    let refreshFails: boolean;
    let readsFail: boolean;

    beforeEach(() => {
        container = document.createElement("div");
        document.body.append(container);
        root = createRoot(container);
        server = initialServer;
        tools = initialTools;
        delayedRead = undefined;
        refreshFails = true;
        readsFail = false;
        invoke.mockImplementation(async command => {
            switch (command) {
                case "list_mcp_servers": {
                    if (readsFail) throw new Error("Server status unavailable");
                    const servers = [server];
                    await delayedRead;
                    return servers;
                }
                case "list_actions": {
                    if (readsFail) throw new Error("Tool directory unavailable");
                    const actions = tools;
                    await delayedRead;
                    return actions;
                }
                case "get_tool_settings":
                    return settings;
                case "refresh_mcp_tools":
                    server = { ...server, tool_count: 0 };
                    tools = [];
                    if (refreshFails) throw new Error("MCP tool discovery failed");
                    return null;
                default:
                    throw new Error(`Unexpected IPC command ${command}`);
            }
        });
    });

    afterEach(() => {
        act(() => root.unmount());
        container.remove();
        invoke.mockReset();
    });

    async function refresh() {
        const button = container.querySelector<HTMLButtonElement>(
            'button[title="settings.mcp.refresh_tooltip"]',
        );
        expect(button).not.toBeNull();
        await act(async () => button!.click());
    }

    it("updates the server count and tool list and announces a refresh failure", async () => {
        await act(async () => root.render(<McpTab />));
        expect(container.textContent).toContain("2 tools");
        for (const tool of initialTools) {
            expect(container.querySelector(`[aria-label="${tool.id}"]`)).not.toBeNull();
        }

        await refresh();

        expect(container.textContent).toContain("0 tools");
        for (const tool of initialTools) {
            expect(container.querySelector(`[aria-label="${tool.id}"]`)).toBeNull();
        }
        expect(container.querySelector('[role="alert"]')?.textContent).toContain("settings.mcp.refresh_failed");

        refreshFails = false;
        await refresh();
        expect(container.querySelector('[role="alert"]')).toBeNull();
    });

    it("ignores status and tool reads started before the refresh reload", async () => {
        refreshFails = false;
        await act(async () => root.render(<McpTab />));
        let releaseRead!: () => void;
        delayedRead = new Promise<void>(resolve => { releaseRead = resolve; });
        invoke.mockClear();
        await act(async () => root.render(<McpTab initialServers={[initialServer]} />));
        expect(invoke.mock.calls.map(([command]) => command)).toEqual(
            expect.arrayContaining(["list_mcp_servers", "list_actions"]),
        );
        delayedRead = undefined;

        await refresh();
        expect(container.textContent).toContain("0 tools");

        await act(async () => releaseRead());

        expect(container.textContent).toContain("0 tools");
        expect(container.textContent).not.toContain("2 tools");
        expect(container.querySelector(`[aria-label="${initialTools[0].id}"]`)).toBeNull();
    });

    it("shows unavailable catalog state when the status and tool rereads also fail", async () => {
        await act(async () => root.render(<McpTab />));
        expect(container.textContent).toContain("2 tools");
        readsFail = true;

        await refresh();

        expect(container.querySelector('[role="alert"]')?.textContent).toContain("settings.mcp.refresh_failed");
        expect(container.textContent).toContain("settings.mcp.server_status_unavailable");
        expect(container.textContent).toContain("settings.mcp.tool_list_unavailable");
        expect(container.textContent).not.toContain("2 tools");
        expect(container.textContent).not.toContain("0 tools");
        expect(container.querySelector(`[aria-label="${initialTools[0].id}"]`)).toBeNull();

        readsFail = false;
        refreshFails = false;
        await refresh();
        expect(container.textContent).toContain("0 tools");
        expect(container.textContent).not.toContain("settings.mcp.server_status_unavailable");
        expect(container.textContent).not.toContain("settings.mcp.tool_list_unavailable");
    });
});
