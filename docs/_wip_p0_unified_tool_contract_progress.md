# P0 统一工具契约改造进度

## 当前目标
实现 Unified Tool Contract（UTC）P0：统一 builtin / MCP / native / pseudo 工具契约，优先解决 canonical tool id、工具开关键、执行路径与目录展示一致性。

## 已完成
- 完成架构启示文档：`docs/claude-code-architecture-inspirations.md`
- 完成 P0 实施计划并获批准
- 建立本临时进度文件
- 后端 `ActionRegistry` 已改为 canonical tool id + `source` / `server_name` 元信息
- MCP 注册已改用 canonical id，`clear_mcp_tools()` 只按 MCP tool id 删除
- `ToolSettings.enabled_tools` 已切到按 canonical id 存储
- 应用启动时会将旧 builtin 裸名工具配置迁移到 canonical id 并回写
- 已抽取共享工具执行器：`src-tauri/src/actions/executor.rs`
- `chat.rs` 与 `telegram/bot.rs` 已接入共享执行器
- `commands/actions.rs` 已按 canonical tool id 解析、校验和执行工具
- 前端 bridge 的 `ActionInfo` / `ToolCallEvent` / `ToolTraceItem` 类型已补充：
  - `id`
  - `source`
  - `server_name`
  - `needs_feedback`
  - `tool_id`
  - `permission_level`
  - `risk_tags`
- 聊天实时 trace、历史回放、tool result metadata 已贯通：
  - `tool_id`
  - `tool_name`
  - `source`
  - `server_name`
  - `needs_feedback`
  - `permission_level`
  - `risk_tags`
- `assistant_tool_calls` / `tool_result` 持久化 metadata 已补齐 canonical tool identity 字段
- `McpTab.tsx` 已切换为读取统一工具目录 `listActions()`，并按 `tool.id` 保存开关状态
- 现有 MCP 设置页已按 builtin / MCP / server_name 分组展示工具，并显示来源、风险标签、权限级别
- direct action / chat / Telegram 输入边界已统一解析到 canonical `tool_id`，执行核心不再依赖裸 `name`
- 已新增统一权限决策模块：`src-tauri/src/actions/permission.rs`
- `executor.rs` / `commands/actions.rs` / `chat.rs` 已开始消费结构化 `PermissionDecision`
- pending approval 触发已不再依赖纯字符串前缀，改为读取执行结果上的结构化权限决策
- 已新增第一版审计结构：`src-tauri/src/actions/audit.rs`
- 审计事件已最小接入 chat 工具执行链，当前先以日志输出为主，尚未持久化入库
- 已完成提交：
    - `a33061c` `feat: add tool audit model and permission visibility`
    - `5c81d14` `refactor: drive approval flow from permission decisions`
    - `25ea9fe` `refactor: centralize tool permission decisions`
    - `eff5228` `refactor: resolve tool inputs to canonical ids at boundaries`
    - `d1ba65d` `feat: group tool directory in MCP settings`
    - `ba38da5` `feat: enrich chat tool trace payloads with identity`
    - `e4e8721` `feat: persist canonical tool identity metadata`
    - `0d99ad7` `test: add tool identity coverage for chat history`

## 关键决策
- 真实工具身份使用 canonical tool id，而不是裸 `name`
- builtin 与 MCP 保留来源元信息，允许同名共存
- pseudo / native tool calling 统一落到共享执行路径，P0 不移除 pseudo 兼容层
- 前端工具目录直接消费后端统一目录
- 旧工具开关配置只自动迁移 builtin 裸名，不对冲突 alias 做隐式猜测

## 已验证
- `npx vitest run src/ui/widgets/chat-history.test.ts`
- `npx vitest run src/ui/widgets/chat-history.test.ts src/ui/widgets/settings/mcpToolDisplay.test.ts`
- `npx vitest run src/ui/widgets/settings/mcpToolDisplay.test.ts`
- `npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test llm::messages --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`
- `cd src-tauri && cargo test telegram::bot --no-run`
- `cd src-tauri && cargo test actions::permission --no-run`
- `cd src-tauri && cargo test actions::executor --no-run`
- `cd src-tauri && cargo test --lib --no-run`

## 验证备注
- 运行级 Rust 单测在当前 Windows 环境仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`；本轮以编译级验证与前端单测为主。
- `cargo check` 与各目标 `--no-run` 均已通过，说明本轮代码与测试编译链路稳定。
- 最终收口验证已通过：
  - `npx tsc --noEmit`
  - `cd src-tauri && cargo check`
  - `cd src-tauri && cargo check --tests`
  - `cd src-tauri && cargo test --lib --no-run`
  - `cd src-tauri && cargo clippy -- -D warnings`
- 最终阶段补充完成：
  - 第一版 `ToolAuditEvent` 结构已落地
  - 审计事件已最小接入 chat 工具执行链（当前先日志输出，不入库）
  - 现有 MCP 设置页已满足 Phase 1 最小权限可视化目标
  - 进度文档已与当前代码状态对齐
  - 最近新增提交：
    - `a33061c` `feat: add tool audit model and permission visibility`
    - `5c81d14` `refactor: drive approval flow from permission decisions`
    - `25ea9fe` `refactor: centralize tool permission decisions`
    - `eff5228` `refactor: resolve tool inputs to canonical ids at boundaries`
    - `d1ba65d` `feat: group tool directory in MCP settings`
    - `ba38da5` `feat: enrich chat tool trace payloads with identity`
    - `e4e8721` `feat: persist canonical tool identity metadata`
    - `0d99ad7` `test: add tool identity coverage for chat history`

## 如需继续下一阶段
1. 为 `assistant_tool_calls` / `tool_result` metadata 补充 `tool_id`、`source`、`server_name`
2. 将前端设置页继续细化为按 `source` / `server_name` 分组展示
3. 继续把 pseudo/native 输入适配进一步收口

## Claude Code 精髓最小收口检查项
- [x] Hook modify strict/permissive 策略已落地
- [x] MOD 声明式 capabilities 最小模型已落地
- [x] chat deny kind 结构化优先已收口

## 备注
- 这是当前实现过程专用的临时进度文件，后续每个关键里程碑都要更新。
