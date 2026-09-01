# 进度追踪：删除设置分组小标题

> 创建时间：2026-08-31 | 状态：进行中

## 目标
删除设置弹窗中 `SETTINGS.GROUPS.BASIC` 和 `SETTINGS.GROUPS.ADVANCED` 两个小标题，保留设置导航与绘图页功能。

## 成功标准
设置弹窗不再显示这两个小标题，其他设置导航和绘图页不被误删；构建/相关测试通过。

## 已读文件
- `src/ui/widgets/settings/settings-groups.ts` — 两个分组仅通过 `label` 字段提供标题，分组标签仍负责保留设置项导航
- `src/ui/widgets/settings/settings-groups.test.ts` — 已覆盖分组归属与所有设置项不丢失
- `src/ui/widgets/SettingsPanel.tsx` — 分组标题由 `t(group.label)` 节点统一渲染，绘图页由独立 `ImageGenSettings` 节点渲染
- `package.json` — 前端验证使用 `npm test` 与 `npm run build`
- `src/ui/locales/*.json` — 未发现 `settings.groups.basic/advanced` 的有效本地化条目，原始键因此会直接显示

## 当前进度
完整前端测试、生产构建与差异检查均已通过

## 下一步
等待用户实际打开设置弹窗确认视觉结果

## 发现的关键信息
- 标题键只在 `settings-groups.ts` 与 `SettingsPanel.tsx` 中使用；绘图页组件不依赖标题字段。
- `npm test`：45 个测试文件、242 个测试通过。
- `npm run build`：TypeScript 检查与 Vite 生产构建通过；仅有既有分包警告。
- `git diff --check`：无空白错误；标题键在设置模块中无残留引用。
