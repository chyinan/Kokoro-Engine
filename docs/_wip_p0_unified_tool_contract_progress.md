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
- 前端 bridge 的 `ActionInfo` / `ToolCallEvent` 类型已补充：
  - `id`
  - `source`
  - `server_name`
  - `needs_feedback`
  - `tool_id`
- `McpTab.tsx` 已切换为读取统一工具目录 `listActions()`，并按 `tool.id` 保存开关状态

## 关键决策
- 真实工具身份使用 canonical tool id，而不是裸 `name`
- builtin 与 MCP 保留来源元信息，允许同名共存
- pseudo / native tool calling 统一落到共享执行路径，P0 不移除 pseudo 兼容层
- 前端工具目录直接消费后端统一目录
- 旧工具开关配置只自动迁移 builtin 裸名，不对冲突 alias 做隐式猜测

## 已验证
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test --lib --no-run`
- `npx tsc --noEmit`

## 验证备注
- `cargo test actions::registry -- --nocapture` 在当前 Windows 环境运行测试二进制时出现 `STATUS_ENTRYPOINT_NOT_FOUND`，但测试编译本身通过；这更像是本地运行时 / DLL 环境问题，不是当前改动导致的编译错误。

## 如需继续下一阶段
1. 为 `assistant_tool_calls` / `tool_result` metadata 补充 `tool_id`、`source`、`server_name`
2. 将前端设置页继续细化为按 `source` / `server_name` 分组展示
3. 继续把 pseudo/native 输入适配进一步收口

## 备注
- 这是当前实现过程专用的临时进度文件，后续每个关键里程碑都要更新。
