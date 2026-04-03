import { describe, it, expect } from 'vitest';
import type { ActionInfo } from '../../../lib/kokoro-bridge';
import { getToolDisplayDescription, getToolSourceLabel } from './mcpToolDisplay';

function buildAction(overrides: Partial<ActionInfo>): ActionInfo {
    return {
        id: 'builtin__get_time',
        name: 'get_time',
        source: 'builtin',
        description: 'Get the current date and time',
        parameters: [],
        needs_feedback: true,
        risk_tags: [],
        permission_level: 'safe',
        ...overrides,
    };
}

type MockT = (key: string, options?: { defaultValue?: string }) => string;

describe('mcpToolDisplay', () => {
    it('uses localized description for built-in tool when available', () => {
        const tool = buildAction({ source: 'builtin', name: 'get_time' });
        const t: MockT = (key, options) => {
            if (key === 'settings.mcp.builtin_tools.items.get_time.description') {
                return '获取当前日期和时间。';
            }
            return options?.defaultValue ?? key;
        };

        expect(getToolDisplayDescription(tool, t)).toBe('获取当前日期和时间。');
    });

    it('falls back to backend description when built-in translation is missing', () => {
        const tool = buildAction({ source: 'builtin', name: 'unknown_tool', description: 'backend description' });
        const t: MockT = (_key, options) => options?.defaultValue ?? '';

        expect(getToolDisplayDescription(tool, t)).toBe('backend description');
    });

    it('uses localized description for MCP tool when available', () => {
        const tool = buildAction({
            source: 'mcp',
            id: 'mcp__time__convert_time',
            name: 'convert_time',
            description: 'Convert time between timezones',
        });
        const t: MockT = (key, options) => {
            if (key === 'settings.mcp.mcp_tools.items.mcp__time__convert_time.description') {
                return '在时区之间转换时间。';
            }
            return options?.defaultValue ?? key;
        };

        expect(getToolDisplayDescription(tool, t)).toBe('在时区之间转换时间。');
    });

    it('falls back to backend description when MCP translation is missing', () => {
        const tool = buildAction({ source: 'mcp', id: 'mcp__foo__bar', description: 'remote tool description' });
        const t: MockT = (_key, options) => options?.defaultValue ?? '';

        expect(getToolDisplayDescription(tool, t)).toBe('remote tool description');
    });

    it('localizes built-in source label', () => {
        const tool = buildAction({ source: 'builtin' });
        const t: MockT = (key) => {
            if (key === 'settings.mcp.builtin_tools.source_builtin') {
                return '内置';
            }
            return key;
        };

        expect(getToolSourceLabel(tool, t)).toBe('内置');
    });

    it('appends server name to MCP source label', () => {
        const tool = buildAction({ source: 'mcp', server_name: 'filesystem' });
        const t: MockT = (key) => {
            if (key === 'settings.mcp.builtin_tools.source_mcp') {
                return 'MCP';
            }
            return key;
        };

        expect(getToolSourceLabel(tool, t)).toBe('MCP · filesystem');
    });
});
