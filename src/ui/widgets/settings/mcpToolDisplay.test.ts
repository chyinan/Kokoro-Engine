import { describe, it, expect } from 'vitest';
import type { ActionInfo } from '../../../lib/kokoro-bridge';
import { getToolDisplayDescription, getToolPermissionLevelLabel, getToolRiskTagsLabel, getToolSourceLabel, groupToolsForDisplay } from './mcpToolDisplay';

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

    it('groups built-in tools separately and MCP tools by server', () => {
        const groups = groupToolsForDisplay([
            buildAction({ id: 'builtin__get_time', name: 'get_time', source: 'builtin' }),
            buildAction({ id: 'mcp__filesystem__read_file', name: 'read_file', source: 'mcp', server_name: 'filesystem' }),
            buildAction({ id: 'mcp__filesystem__write_file', name: 'write_file', source: 'mcp', server_name: 'filesystem' }),
            buildAction({ id: 'mcp__memory__search', name: 'search', source: 'mcp', server_name: 'memory' }),
        ]);

        expect(groups.map((group) => group.key)).toEqual([
            'builtin',
            'mcp:filesystem',
            'mcp:memory',
        ]);
        expect(groups[0]?.tools.map((tool) => tool.id)).toEqual(['builtin__get_time']);
        expect(groups[1]?.tools.map((tool) => tool.id)).toEqual([
            'mcp__filesystem__read_file',
            'mcp__filesystem__write_file',
        ]);
        expect(groups[2]?.tools.map((tool) => tool.id)).toEqual(['mcp__memory__search']);
    });

    it('falls back to unnamed label when MCP server name is missing', () => {
        const groups = groupToolsForDisplay([
            buildAction({ id: 'mcp__unknown__lookup', name: 'lookup', source: 'mcp', server_name: undefined }),
        ]);

        expect(groups).toHaveLength(1);
        expect(groups[0]?.key).toBe('mcp:unnamed');
    });

    it('renders localized risk tags and permission labels', () => {
        const tool = buildAction({
            source: 'mcp',
            risk_tags: ['read', 'external'],
            permission_level: 'elevated',
        });
        const t: MockT = (key, options) => {
            if (key === 'settings.mcp.builtin_tools.risk_tags.read') return '读取';
            if (key === 'settings.mcp.builtin_tools.risk_tags.external') return '外部';
            if (key === 'settings.mcp.builtin_tools.permission_levels.elevated') return '高权限';
            return options?.defaultValue ?? key;
        };

        expect(getToolRiskTagsLabel(tool, t)).toBe('读取 · 外部');
        expect(getToolPermissionLevelLabel(tool, t)).toBe('高权限');
    });
});
