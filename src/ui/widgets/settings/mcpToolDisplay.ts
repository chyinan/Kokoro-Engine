import type { ActionInfo } from '../../../lib/kokoro-bridge';

type TranslateFn = (key: string, options?: { defaultValue?: string }) => string;

export type ToolGroup = {
    key: string;
    title: string;
    tools: Array<ActionInfo>;
};

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

export function getToolRiskTagsLabel(tool: ActionInfo, t: TranslateFn): string {
    if (tool.risk_tags.length === 0) {
        return t('settings.mcp.builtin_tools.risk_tags.none', { defaultValue: '无' });
    }

    return tool.risk_tags
        .map((tag) => t(`settings.mcp.builtin_tools.risk_tags.${tag}`, { defaultValue: tag }))
        .join(' · ');
}

export function getToolPermissionLevelLabel(tool: ActionInfo, t: TranslateFn): string {
    return t(`settings.mcp.builtin_tools.permission_levels.${tool.permission_level}`, {
        defaultValue: tool.permission_level,
    });
}

export function groupToolsForDisplay(tools: Array<ActionInfo>): Array<ToolGroup> {
    const builtinTools = tools.filter((tool) => tool.source === 'builtin');
    const mcpGroups = new Map<string, Array<ActionInfo>>();

    for (const tool of tools) {
        if (tool.source !== 'mcp') {
            continue;
        }

        const serverName = tool.server_name || 'unnamed';
        const groupKey = `mcp:${serverName}`;
        const current = mcpGroups.get(groupKey) || [];
        current.push(tool);
        mcpGroups.set(groupKey, current);
    }

    const groups: Array<ToolGroup> = [];
    if (builtinTools.length > 0) {
        groups.push({
            key: 'builtin',
            title: 'Built-in',
            tools: builtinTools,
        });
    }

    for (const [key, groupedTools] of mcpGroups.entries()) {
        groups.push({
            key,
            title: key === 'mcp:unnamed' ? 'Unnamed MCP Server' : groupedTools[0]?.server_name || 'Unnamed MCP Server',
            tools: groupedTools,
        });
    }

    return groups;
}

export function getToolGroupTitle(group: ToolGroup, t: TranslateFn): string {
    if (group.key === 'builtin') {
        return t('settings.mcp.builtin_tools.groups.builtin', { defaultValue: '内置工具' });
    }

    if (group.key === 'mcp:unnamed') {
        return t('settings.mcp.builtin_tools.groups.unnamed_mcp', { defaultValue: '未命名 MCP 服务' });
    }

    return t('settings.mcp.builtin_tools.groups.mcp_server', { defaultValue: group.title });
}

export function getToolGroupDescription(group: ToolGroup, t: TranslateFn): string | null {
    if (group.key === 'builtin') {
        return t('settings.mcp.builtin_tools.groups.builtin_desc', { defaultValue: 'Kokoro 内置工具' });
    }

    return null;
}

export function getToolEnabled(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return enabledTools[toolId] ?? true;
}

export function getToolGroupKey(tool: ActionInfo): string {
    return tool.source === 'builtin' ? 'builtin' : `mcp:${tool.server_name || 'unnamed'}`;
}

export function getToolServerNameLabel(tool: ActionInfo, t: TranslateFn): string | null {
    if (tool.source !== 'mcp') {
        return null;
    }

    return tool.server_name || t('settings.mcp.builtin_tools.groups.unnamed_mcp', { defaultValue: '未命名 MCP 服务' });
}

export function getToolNameLabel(tool: ActionInfo): string {
    return tool.name;
}

export function getToolIdLabel(tool: ActionInfo): string {
    return tool.id;
}

export function getToolGroupCountLabel(group: ToolGroup, t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.groups.count', { defaultValue: `${group.tools.length} tools` });
}

export function sortToolGroups(groups: Array<ToolGroup>): Array<ToolGroup> {
    return [...groups].sort((left, right) => {
        if (left.key === 'builtin') return -1;
        if (right.key === 'builtin') return 1;
        return left.key.localeCompare(right.key);
    });
}

export function sortToolsForDisplay(tools: Array<ActionInfo>): Array<ActionInfo> {
    return [...tools].sort((left, right) => left.id.localeCompare(right.id));
}

export function buildSortedToolGroups(tools: Array<ActionInfo>): Array<ToolGroup> {
    return sortToolGroups(
        groupToolsForDisplay(tools).map((group) => ({
            ...group,
            tools: sortToolsForDisplay(group.tools),
        }))
    );
}

export function getToolPermissionBadgeClass(tool: ActionInfo): string {
    return tool.permission_level === 'elevated'
        ? 'bg-amber-500/10 text-amber-300 border-amber-500/20'
        : 'bg-emerald-500/10 text-emerald-300 border-emerald-500/20';
}

export function getToolSourceBadgeClass(tool: ActionInfo): string {
    return tool.source === 'mcp'
        ? 'border-cyan-500/20 text-cyan-300'
        : 'border-[var(--color-border)] text-[var(--color-text-muted)]';
}

export function getToolRiskBadgeClass(): string {
    return 'border border-[var(--color-border)] text-[var(--color-text-muted)]';
}

export function getToolCardContainerClass(): string {
    return 'rounded-xl border border-[var(--color-border)] bg-[var(--color-surface-1)]/80 px-3 py-3';
}

export function getToolGroupContainerClass(): string {
    return 'space-y-2';
}

export function getToolGroupHeaderClass(): string {
    return 'flex items-center justify-between gap-3';
}

export function getToolMetaTextClass(): string {
    return 'text-[11px] text-[var(--color-text-muted)]';
}

export function getToolDescriptionClass(): string {
    return 'mt-1 text-xs text-[var(--color-text-muted)]';
}

export function getToolBadgeRowClass(): string {
    return 'mt-2 flex items-center gap-2 flex-wrap';
}

export function getToolBadgeClass(): string {
    return 'rounded-full border px-2 py-0.5 text-[10px] uppercase tracking-wide';
}

export function getToolSwitchContainerClass(): string {
    return 'w-10 h-5 rounded-full transition-colors relative shrink-0';
}

export function getToolSwitchEnabledClass(enabled: boolean): string {
    return enabled ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]';
}

export function getToolSwitchThumbX(enabled: boolean): number {
    return enabled ? 20 : 2;
}

export function getToolSwitchThumbClass(): string {
    return 'absolute top-0.5 w-4 h-4 rounded-full bg-white';
}

export function getToolCardHeaderTitleClass(): string {
    return 'text-sm font-medium text-[var(--color-text-primary)]';
}

export function getToolGroupTitleClass(): string {
    return 'text-xs font-heading font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]';
}

export function getToolGroupDescClass(): string {
    return 'text-[11px] text-[var(--color-text-muted)]';
}

export function getToolIdTextClass(): string {
    return 'mt-1 text-[11px] text-[var(--color-text-muted)] break-all';
}

export function getToolPrimaryRowClass(): string {
    return 'flex items-start justify-between gap-4';
}

export function getToolPrimaryInfoClass(): string {
    return 'min-w-0';
}

export function getToolTitleRowClass(): string {
    return 'flex items-center gap-2 flex-wrap';
}

export function getToolToggleAriaLabel(tool: ActionInfo): string {
    return tool.id;
}

export function getToolToggleTitle(enabled: boolean, t: TranslateFn): string {
    return enabled
        ? t('settings.mcp.builtin_tools.disable', { defaultValue: 'Disable tool' })
        : t('settings.mcp.builtin_tools.enable', { defaultValue: 'Enable tool' });
}

export function getToolRiskLine(tool: ActionInfo, t: TranslateFn): string {
    return `${t('settings.mcp.builtin_tools.risk_tags_label', { defaultValue: '风险' })}: ${getToolRiskTagsLabel(tool, t)}`;
}

export function getToolPermissionLine(tool: ActionInfo, t: TranslateFn): string {
    return `${t('settings.mcp.builtin_tools.permission_label', { defaultValue: '权限' })}: ${getToolPermissionLevelLabel(tool, t)}`;
}

export function getToolServerLine(tool: ActionInfo, t: TranslateFn): string | null {
    const serverName = getToolServerNameLabel(tool, t);
    if (!serverName) {
        return null;
    }

    return `${t('settings.mcp.builtin_tools.server_label', { defaultValue: '服务' })}: ${serverName}`;
}

export function getToolSourceLine(tool: ActionInfo, t: TranslateFn): string {
    return `${t('settings.mcp.builtin_tools.source_label', { defaultValue: '来源' })}: ${getToolSourceLabel(tool, t)}`;
}

export function getToolMetaLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [
        getToolSourceLine(tool, t),
        getToolServerLine(tool, t),
        getToolRiskLine(tool, t),
        getToolPermissionLine(tool, t),
    ].filter((line): line is string => Boolean(line));
}

export function getToolGroupHeadingLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupSubheadingLabel(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolGroupDisplayData(tools: Array<ActionInfo>, t: TranslateFn): Array<ToolGroup & { titleLabel: string; descriptionLabel: string | null }> {
    return buildSortedToolGroups(tools).map((group) => ({
        ...group,
        titleLabel: getToolGroupHeadingLabel(group, t),
        descriptionLabel: getToolGroupSubheadingLabel(group, t),
    }));
}

export function getToolCountSummary(tools: Array<ActionInfo>, t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.count_summary', { defaultValue: `${tools.length} tools` });
}

export function hasAnyTools(tools: Array<ActionInfo>): boolean {
    return tools.length > 0;
}

export function getEmptyToolsLabel(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.empty', { defaultValue: 'No tools available' });
}

export function shouldShowToolServerName(tool: ActionInfo): boolean {
    return tool.source === 'mcp';
}

export function shouldShowToolDescription(tool: ActionInfo): boolean {
    return Boolean(tool.description);
}

export function shouldShowToolRiskTags(tool: ActionInfo): boolean {
    return tool.risk_tags.length > 0;
}

export function shouldShowToolPermissionLevel(): boolean {
    return true;
}

export function shouldShowToolGroupDescription(group: ToolGroup): boolean {
    return group.key === 'builtin';
}

export function shouldShowToolsSection(tools: Array<ActionInfo>): boolean {
    return tools.length > 0;
}

export function getToolGroupAriaLabel(group: ToolGroup, t: TranslateFn): string {
    return `${getToolGroupTitle(group, t)} (${group.tools.length})`;
}

export function getToolGroupServerName(group: ToolGroup): string | null {
    return group.key.startsWith('mcp:') ? group.title : null;
}

export function getToolSourceKey(tool: ActionInfo): string {
    return tool.source;
}

export function getToolPermissionKey(tool: ActionInfo): string {
    return tool.permission_level;
}

export function getToolRiskKeys(tool: ActionInfo): Array<string> {
    return [...tool.risk_tags];
}

export function normalizeToolServerName(serverName?: string): string {
    return serverName || 'unnamed';
}

export function createToolGroupKey(tool: ActionInfo): string {
    return tool.source === 'builtin' ? 'builtin' : `mcp:${normalizeToolServerName(tool.server_name)}`;
}

export function getToolGroupDisplayTitle(tool: ActionInfo): string {
    return tool.source === 'builtin' ? 'Built-in' : normalizeToolServerName(tool.server_name);
}

export function groupTools(tools: Array<ActionInfo>): Map<string, Array<ActionInfo>> {
    const groups = new Map<string, Array<ActionInfo>>();
    for (const tool of tools) {
        const key = createToolGroupKey(tool);
        const current = groups.get(key) || [];
        current.push(tool);
        groups.set(key, current);
    }
    return groups;
}

export function mapGroupsToDisplay(groups: Map<string, Array<ActionInfo>>): Array<ToolGroup> {
    return Array.from(groups.entries()).map(([key, groupedTools]) => ({
        key,
        title: key === 'builtin' ? 'Built-in' : groupedTools[0]?.server_name || 'Unnamed MCP Server',
        tools: groupedTools,
    }));
}

export function buildToolGroups(tools: Array<ActionInfo>): Array<ToolGroup> {
    return sortToolGroups(mapGroupsToDisplay(groupTools(tools)));
}

export function getToolGroupsForDisplay(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildToolGroups(tools).map((group) => ({
        ...group,
        tools: sortToolsForDisplay(group.tools),
    }));
}

export function getToolBadges(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [
        getToolSourceLabel(tool, t),
        getToolPermissionLevelLabel(tool, t),
        getToolRiskTagsLabel(tool, t),
    ];
}

export function getToolBadgesVisible(): boolean {
    return true;
}

export function getToolServerBadge(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolDescription(tool: ActionInfo, t: TranslateFn): string {
    return getToolDisplayDescription(tool, t);
}

export function getToolTitle(tool: ActionInfo): string {
    return tool.name;
}

export function getToolIdentifier(tool: ActionInfo): string {
    return tool.id;
}

export function getToolCardLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [
        getToolIdentifier(tool),
        ...getToolMetaLines(tool, t),
    ];
}

export function isToolEnabledByDefault(): boolean {
    return true;
}

export function resolveToolEnabled(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolGroupSectionKey(group: ToolGroup): string {
    return group.key;
}

export function getToolSectionTitle(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolSectionDescription(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolSectionTools(group: ToolGroup): Array<ActionInfo> {
    return group.tools;
}

export function getToolSwitchState(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return resolveToolEnabled(toolId, enabledTools);
}

export function getToolSectionCount(group: ToolGroup): number {
    return group.tools.length;
}

export function getToolSectionCountLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupCountLabel(group, t);
}

export function getToolSections(tools: Array<ActionInfo>): Array<ToolGroup> {
    return getToolGroupsForDisplay(tools);
}

export function getToolDisplayData(tools: Array<ActionInfo>, t: TranslateFn) {
    return {
        groups: getToolSections(tools),
        totalLabel: getToolCountSummary(tools, t),
    };
}

export function hasToolDisplayData(tools: Array<ActionInfo>): boolean {
    return hasAnyTools(tools);
}

export function getToolEmptyStateLabel(t: TranslateFn): string {
    return getEmptyToolsLabel(t);
}

export function shouldShowToolEmptyState(tools: Array<ActionInfo>): boolean {
    return !hasAnyTools(tools);
}

export function getToolGroupTitleText(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupDescText(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolEnabledState(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function buildToolMetaSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolMetaLines(tool, t).join(' · ');
}

export function getToolGroupDisplayName(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupDisplayDescription(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolServerGroupName(tool: ActionInfo): string | null {
    return tool.source === 'mcp' ? normalizeToolServerName(tool.server_name) : null;
}

export function getToolDisplayServerName(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolRiskSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolPermissionSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolMetaSummaryLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolGroupSummary(group: ToolGroup, t: TranslateFn): string {
    return `${getToolGroupTitle(group, t)} · ${getToolGroupCountLabel(group, t)}`;
}

export function getToolTooltips(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolDisplayGroups(tools: Array<ActionInfo>): Array<ToolGroup> {
    return getToolSections(tools);
}

export function shouldRenderToolGroups(tools: Array<ActionInfo>): boolean {
    return shouldShowToolsSection(tools);
}

export function getToolSourceAndServerLabel(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolRiskAndPermissionLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [getToolRiskLine(tool, t), getToolPermissionLine(tool, t)];
}

export function getToolDisplayItems(tools: Array<ActionInfo>): Array<ActionInfo> {
    return sortToolsForDisplay(tools);
}

export function getToolDisplayGroupsSorted(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildSortedToolGroups(tools);
}

export function getToolCardDescription(tool: ActionInfo, t: TranslateFn): string {
    return getToolDisplayDescription(tool, t);
}

export function getToolCardMeta(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolCardBadges(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolBadges(tool, t);
}

export function getToolCardEnabled(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolCardGroupKey(tool: ActionInfo): string {
    return getToolGroupKey(tool);
}

export function getToolCardServer(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolCardPermission(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolCardRisks(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolGroupTitleDefault(group: ToolGroup): string {
    return group.title;
}

export function getToolGroupTools(group: ToolGroup): Array<ActionInfo> {
    return group.tools;
}

export function getToolCardSource(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolCardId(tool: ActionInfo): string {
    return tool.id;
}

export function getToolCardName(tool: ActionInfo): string {
    return tool.name;
}

export function getToolCardSwitchState(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolBadgeLabels(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolBadges(tool, t);
}

export function getToolMetaDisplay(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolPermissionAndRiskSummary(tool: ActionInfo, t: TranslateFn): string {
    return `${getToolPermissionLevelLabel(tool, t)} · ${getToolRiskTagsLabel(tool, t)}`;
}

export function getToolGroupHeader(group: ToolGroup, t: TranslateFn): { title: string; description: string | null } {
    return {
        title: getToolGroupTitle(group, t),
        description: getToolGroupDescription(group, t),
    };
}

export function getToolCardHeader(tool: ActionInfo): { name: string; id: string } {
    return {
        name: tool.name,
        id: tool.id,
    };
}

export function getToolCardDetails(tool: ActionInfo, t: TranslateFn): { description: string; meta: Array<string> } {
    return {
        description: getToolDisplayDescription(tool, t),
        meta: getToolMetaLines(tool, t),
    };
}

export function getToolGroupDisplayOrder(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildSortedToolGroups(tools);
}

export function getToolMetaBadges(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolBadges(tool, t);
}

export function getToolSummary(tool: ActionInfo, t: TranslateFn): string {
    return `${tool.name} · ${getToolPermissionLevelLabel(tool, t)}`;
}

export function getToolFullSummary(tool: ActionInfo, t: TranslateFn): string {
    return `${tool.id} · ${buildToolMetaSummary(tool, t)}`;
}

export function getToolGroupList(tools: Array<ActionInfo>): Array<ToolGroup> {
    return getToolDisplayGroupsSorted(tools);
}

export function getToolGroupDisplayMap(tools: Array<ActionInfo>): Array<ToolGroup> {
    return getToolGroupList(tools);
}

export function getToolDisplayState(tools: Array<ActionInfo>, t: TranslateFn) {
    return getToolDisplayData(tools, t);
}

export function getToolCardLabels(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [tool.name, tool.id, ...getToolMetaLines(tool, t)];
}

export function getToolGroupLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupMeta(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolServerFallbackLabel(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.groups.unnamed_mcp', { defaultValue: '未命名 MCP 服务' });
}

export function getToolSourceFallbackLabel(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.source_builtin', { defaultValue: 'Built-in' });
}

export function getToolRiskFallbackLabel(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.risk_tags.none', { defaultValue: '无' });
}

export function getToolPermissionFallbackLabel(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.permission_levels.safe', { defaultValue: 'safe' });
}

export function getToolMetaFallbacks(t: TranslateFn): Array<string> {
    return [
        getToolSourceFallbackLabel(t),
        getToolRiskFallbackLabel(t),
        getToolPermissionFallbackLabel(t),
    ];
}

export function getToolGroupNames(tools: Array<ActionInfo>): Array<string> {
    return getToolGroupList(tools).map((group) => group.key);
}

export function getToolSourceType(tool: ActionInfo): 'builtin' | 'mcp' {
    return tool.source;
}

export function getToolPermissionType(tool: ActionInfo): 'safe' | 'elevated' {
    return tool.permission_level;
}

export function getToolRiskTypeList(tool: ActionInfo): Array<'read' | 'write' | 'external' | 'sensitive'> {
    return [...tool.risk_tags];
}

export function getToolRiskCount(tool: ActionInfo): number {
    return tool.risk_tags.length;
}

export function getToolGroupCount(group: ToolGroup): number {
    return group.tools.length;
}

export function getToolServerOrFallback(tool: ActionInfo, t: TranslateFn): string {
    return getToolServerNameLabel(tool, t) || getToolServerFallbackLabel(t);
}

export function getToolMetaForCard(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolDescForCard(tool: ActionInfo, t: TranslateFn): string {
    return getToolDisplayDescription(tool, t);
}

export function getToolToggleState(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolSectionAriaLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupAriaLabel(group, t);
}

export function getToolSectionHeader(group: ToolGroup, t: TranslateFn) {
    return getToolGroupHeader(group, t);
}

export function getToolSectionBody(group: ToolGroup): Array<ActionInfo> {
    return group.tools;
}

export function getToolPrimaryMeta(tool: ActionInfo, t: TranslateFn): string {
    return buildToolMetaSummary(tool, t);
}

export function getToolSecondaryMeta(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionAndRiskSummary(tool, t);
}

export function getToolSwitchPosition(enabled: boolean): number {
    return getToolSwitchThumbX(enabled);
}

export function getToolSwitchBaseClass(enabled: boolean): string {
    return `${getToolSwitchContainerClass()} ${getToolSwitchEnabledClass(enabled)}`;
}

export function getToolCardMetaText(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolCardText(tool: ActionInfo, t: TranslateFn): { title: string; id: string; description: string } {
    return {
        title: getToolTitle(tool),
        id: getToolIdLabel(tool),
        description: getToolDisplayDescription(tool, t),
    };
}

export function getToolMcpServerLabel(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolCardData(tool: ActionInfo, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return {
        title: getToolTitle(tool),
        id: getToolIdLabel(tool),
        description: getToolDisplayDescription(tool, t),
        metaLines: getToolMetaLines(tool, t),
        enabled: getToolEnabled(tool.id, enabledTools),
    };
}

export function getToolSectionData(group: ToolGroup, t: TranslateFn) {
    return {
        key: group.key,
        title: getToolGroupTitle(group, t),
        description: getToolGroupDescription(group, t),
        count: group.tools.length,
    };
}

export function getToolDisplayModel(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return buildSortedToolGroups(tools).map((group) => ({
        ...getToolSectionData(group, t),
        tools: group.tools.map((tool) => getToolCardData(tool, t, enabledTools)),
    }));
}

export function getToolSourceDescription(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolPermissionDescription(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolRiskDescription(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolLineItems(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolGroupItems(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return getToolDisplayModel(tools, t, enabledTools);
}

export function hasGroupedTools(tools: Array<ActionInfo>): boolean {
    return tools.length > 0;
}

export function getToolSettingsHeader(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.title', { defaultValue: '工具列表' });
}

export function getToolSettingsDescription(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.desc', { defaultValue: '选择允许模型调用哪些工具服务。' });
}

export function getToolSettingsSavingLabel(isSaving: boolean, t: TranslateFn): string {
    return isSaving
        ? t('settings.mcp.builtin_tools.saving', { defaultValue: '保存中...' })
        : t('settings.mcp.builtin_tools.saved_auto', { defaultValue: '已自动保存' });
}

export function getToolSourceTitle(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.source_label', { defaultValue: '来源' });
}

export function getToolServerTitle(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.server_label', { defaultValue: '服务' });
}

export function getToolRiskTitle(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.risk_tags_label', { defaultValue: '风险' });
}

export function getToolPermissionTitle(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.permission_label', { defaultValue: '权限' });
}

export function getToolSummaryTitle(t: TranslateFn): string {
    return t('settings.mcp.builtin_tools.summary_label', { defaultValue: '摘要' });
}

export function getToolGroupTitleOrFallback(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupDescOrNull(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolRows(tools: Array<ActionInfo>): Array<ActionInfo> {
    return sortToolsForDisplay(tools);
}

export function getToolRowMeta(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolRowBadges(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolBadges(tool, t);
}

export function getToolRowEnabled(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolSectionRows(group: ToolGroup): Array<ActionInfo> {
    return group.tools;
}

export function buildToolSections(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildSortedToolGroups(tools);
}

export function buildToolRows(group: ToolGroup): Array<ActionInfo> {
    return sortToolsForDisplay(group.tools);
}

export function getToolGroupHeaderLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolGroupHeaderMeta(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolGroupHeaderCount(group: ToolGroup): number {
    return group.tools.length;
}

export function getToolGroupHeaderCountText(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupCountLabel(group, t);
}

export function isMcpTool(tool: ActionInfo): boolean {
    return tool.source === 'mcp';
}

export function isBuiltinTool(tool: ActionInfo): boolean {
    return tool.source === 'builtin';
}

export function getToolServerGroupLabel(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolPermissionBadgeLabel(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolRiskBadgeLabel(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolSourceBadgeLabel(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolBadgeSet(tool: ActionInfo, t: TranslateFn): Array<string> {
    return [
        getToolSourceBadgeLabel(tool, t),
        getToolPermissionBadgeLabel(tool, t),
        getToolRiskBadgeLabel(tool, t),
    ];
}

export function getToolMetadataLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolCardDisplay(tool: ActionInfo, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return getToolCardData(tool, t, enabledTools);
}

export function getToolSectionDisplay(group: ToolGroup, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return {
        header: getToolSectionData(group, t),
        rows: group.tools.map((tool) => getToolCardData(tool, t, enabledTools)),
    };
}

export function getToolSettingsDisplay(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return buildSortedToolGroups(tools).map((group) => getToolSectionDisplay(group, t, enabledTools));
}

export function getToolCardSummary(tool: ActionInfo, t: TranslateFn): string {
    return `${tool.name} · ${getToolSourceLabel(tool, t)}`;
}

export function getToolCardSecondarySummary(tool: ActionInfo, t: TranslateFn): string {
    return `${getToolPermissionLevelLabel(tool, t)} · ${getToolRiskTagsLabel(tool, t)}`;
}

export function getToolDisplaySummary(tools: Array<ActionInfo>, t: TranslateFn): string {
    return `${getToolSettingsHeader(t)} · ${getToolCountSummary(tools, t)}`;
}

export function getToolHeaderLabels(t: TranslateFn) {
    return {
        title: getToolSettingsHeader(t),
        description: getToolSettingsDescription(t),
        source: getToolSourceTitle(t),
        server: getToolServerTitle(t),
        risk: getToolRiskTitle(t),
        permission: getToolPermissionTitle(t),
    };
}

export function getToolSectionList(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildSortedToolGroups(tools);
}

export function getToolSectionRowsSorted(group: ToolGroup): Array<ActionInfo> {
    return sortToolsForDisplay(group.tools);
}

export function getToolServerDisplay(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerNameLabel(tool, t);
}

export function getToolPermissionDisplay(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolRiskDisplay(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolSourceDisplay(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolCardLinesForDisplay(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolSectionsForDisplay(tools: Array<ActionInfo>): Array<ToolGroup> {
    return buildSortedToolGroups(tools);
}

export function getToolRowsForDisplay(group: ToolGroup): Array<ActionInfo> {
    return sortToolsForDisplay(group.tools);
}

export function getToolCardFields(tool: ActionInfo, t: TranslateFn) {
    return {
        name: getToolTitle(tool),
        id: getToolIdLabel(tool),
        description: getToolDisplayDescription(tool, t),
        meta: getToolMetaLines(tool, t),
    };
}

export function getToolSectionFields(group: ToolGroup, t: TranslateFn) {
    return {
        title: getToolGroupTitle(group, t),
        description: getToolGroupDescription(group, t),
        count: group.tools.length,
    };
}

export function getToolSectionModels(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return buildSortedToolGroups(tools).map((group) => ({
        ...getToolSectionFields(group, t),
        tools: sortToolsForDisplay(group.tools).map((tool) => ({
            ...getToolCardFields(tool, t),
            enabled: getToolEnabled(tool.id, enabledTools),
        })),
    }));
}

export function getToolDisplayModels(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return getToolSectionModels(tools, t, enabledTools);
}

export function getToolServerTitleText(t: TranslateFn): string {
    return getToolServerTitle(t);
}

export function getToolPermissionTitleText(t: TranslateFn): string {
    return getToolPermissionTitle(t);
}

export function getToolRiskTitleText(t: TranslateFn): string {
    return getToolRiskTitle(t);
}

export function getToolSourceTitleText(t: TranslateFn): string {
    return getToolSourceTitle(t);
}

export function getToolCardToggleTitle(enabled: boolean, t: TranslateFn): string {
    return getToolToggleTitle(enabled, t);
}

export function getToolCardSwitchThumbX(enabled: boolean): number {
    return getToolSwitchThumbX(enabled);
}

export function getToolCardSwitchThumbClass(): string {
    return getToolSwitchThumbClass();
}

export function getToolCardSwitchClass(enabled: boolean): string {
    return `${getToolSwitchContainerClass()} ${getToolSwitchEnabledClass(enabled)}`;
}

export function getToolCardBadgeClass(): string {
    return getToolBadgeClass();
}

export function getToolCardBadgeRowClass(): string {
    return getToolBadgeRowClass();
}

export function getToolCardTitleClass(): string {
    return getToolCardHeaderTitleClass();
}

export function getToolCardMetaClass(): string {
    return getToolMetaTextClass();
}

export function getToolCardDescClass(): string {
    return getToolDescriptionClass();
}

export function getToolSectionTitleClassName(): string {
    return getToolGroupTitleClass();
}

export function getToolSectionDescClassName(): string {
    return getToolGroupDescClass();
}

export function getToolSectionContainerClassName(): string {
    return getToolGroupContainerClass();
}

export function getToolSectionHeaderClassName(): string {
    return getToolGroupHeaderClass();
}

export function getToolCardContainerClassName(): string {
    return getToolCardContainerClass();
}

export function getToolCardPrimaryRowClassName(): string {
    return getToolPrimaryRowClass();
}

export function getToolCardPrimaryInfoClassName(): string {
    return getToolPrimaryInfoClass();
}

export function getToolCardTitleRowClassName(): string {
    return getToolTitleRowClass();
}

export function getToolCardIdClassName(): string {
    return getToolIdTextClass();
}

export function getToolCardMetaLines(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolMetaLines(tool, t);
}

export function getToolCardServerLine(tool: ActionInfo, t: TranslateFn): string | null {
    return getToolServerLine(tool, t);
}

export function getToolCardSourceLine(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLine(tool, t);
}

export function getToolCardRiskLine(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskLine(tool, t);
}

export function getToolCardPermissionLine(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLine(tool, t);
}

export function getToolCardBadgeLabels(tool: ActionInfo, t: TranslateFn): Array<string> {
    return getToolBadgeSet(tool, t);
}

export function getToolCardToggleAria(tool: ActionInfo): string {
    return getToolToggleAriaLabel(tool);
}

export function getToolCardEnabledState(toolId: string, enabledTools: Record<string, boolean>): boolean {
    return getToolEnabled(toolId, enabledTools);
}

export function getToolSectionLabel(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupTitle(group, t);
}

export function getToolSectionDescLabel(group: ToolGroup, t: TranslateFn): string | null {
    return getToolGroupDescription(group, t);
}

export function getToolSectionCountSummary(group: ToolGroup, t: TranslateFn): string {
    return getToolGroupCountLabel(group, t);
}

export function getToolSectionsData(tools: Array<ActionInfo>, t: TranslateFn, enabledTools: Record<string, boolean>) {
    return getToolDisplayModels(tools, t, enabledTools);
}

export function getToolTopSummary(tools: Array<ActionInfo>, t: TranslateFn): string {
    return getToolDisplaySummary(tools, t);
}

export function getToolGroupingKeys(tools: Array<ActionInfo>): Array<string> {
    return buildSortedToolGroups(tools).map((group) => group.key);
}

export function getToolGroupingTitles(tools: Array<ActionInfo>, t: TranslateFn): Array<string> {
    return buildSortedToolGroups(tools).map((group) => getToolGroupTitle(group, t));
}

export function getToolGroupingCounts(tools: Array<ActionInfo>): Array<number> {
    return buildSortedToolGroups(tools).map((group) => group.tools.length);
}

export function getToolCardMetaSummary(tool: ActionInfo, t: TranslateFn): string {
    return buildToolMetaSummary(tool, t);
}

export function getToolCardSourceSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolSourceLabel(tool, t);
}

export function getToolCardPermissionSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolPermissionLevelLabel(tool, t);
}

export function getToolCardRiskSummary(tool: ActionInfo, t: TranslateFn): string {
    return getToolRiskTagsLabel(tool, t);
}

export function getToolSectionEmptyLabel(t: TranslateFn): string {
    return getEmptyToolsLabel(t);
}

export function getToolSectionShouldRender(tools: Array<ActionInfo>): boolean {
    return shouldShowToolsSection(tools);
}

export function getToolCardShouldRenderDescription(tool: ActionInfo): boolean {
    return shouldShowToolDescription(tool);
}

export function getToolCardShouldRenderServer(tool: ActionInfo): boolean {
    return shouldShowToolServerName(tool);
}

export function getToolCardShouldRenderRisks(tool: ActionInfo): boolean {
    return shouldShowToolRiskTags(tool);
}

export function getToolCardShouldRenderPermission(): boolean {
    return shouldShowToolPermissionLevel();
}

export function getToolSectionShouldRenderDescription(group: ToolGroup): boolean {
    return shouldShowToolGroupDescription(group);
}
