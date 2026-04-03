import type { ActionInfo } from '../../../lib/kokoro-bridge';

type TranslateFn = (key: string, options?: { defaultValue?: string }) => string;

export function getToolDisplayDescription(tool: ActionInfo, t: TranslateFn): string {
    if (tool.source === 'builtin') {
        return t(`settings.mcp.builtin_tools.items.${tool.name}.description`, {
            defaultValue: tool.description,
        });
    }

    return t(`settings.mcp.mcp_tools.items.${tool.id}.description`, {
        defaultValue: tool.description,
    });
}

export function getToolSourceLabel(tool: ActionInfo, t: TranslateFn): string {
    if (tool.source === 'mcp') {
        const mcpLabel = t('settings.mcp.builtin_tools.source_mcp', { defaultValue: 'MCP' });
        return tool.server_name ? `${mcpLabel} · ${tool.server_name}` : mcpLabel;
    }

    return t('settings.mcp.builtin_tools.source_builtin', { defaultValue: 'Built-in' });
}
