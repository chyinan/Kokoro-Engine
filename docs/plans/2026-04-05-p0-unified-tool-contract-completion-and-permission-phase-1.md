# P0 Unified Tool Contract Completion and Permission Phase 1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 补完统一工具契约尾项，并在同一执行链上落地多层权限与安全边界主线的第一阶段。

**Architecture:** 先把工具身份模型彻底统一到 canonical `tool_id`，补齐消息/trace/history/MCP 设置页中的 `tool_id/source/server_name/needs_feedback` 语义，再把当前 executor 中散落的 policy / pending approval / fail-closed 规则抽离成统一权限决策层。前端继续接入现有 `McpTab`，不新增独立权限页，只做最小但稳定的权限可视化。

**Tech Stack:** Rust、Tauri v2、React、TypeScript、Vitest、现有 ActionRegistry / executor / chat trace 事件链

---

## 实施前说明

- 本计划默认在当前仓库内执行，不额外创建新功能目录。
- 所有工具身份主键统一使用 canonical `tool_id`。
- 所有步骤遵循“小步提交、先测后改、频繁验证”。
- Windows 下运行级 Rust 测试可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`，本轮以编译级验证和单元测试为主。
- 设置展示统一接入现有 `src/ui/widgets/settings/McpTab.tsx`，不新增独立权限页面。

---

## Task 1: 补齐前端工具 trace 类型与历史回放字段

**Files:**
- Modify: `src/lib/kokoro-bridge.ts:242-262`
- Modify: `src/ui/widgets/ChatPanel.tsx:94-118`
- Modify: `src/ui/widgets/ChatMessage.tsx:37-133`
- Modify: `src/ui/widgets/chat-history.ts:11-122`
- Test: `src/ui/widgets/settings/mcpToolDisplay.test.ts`
- Create: `src/ui/widgets/chat-history.test.ts`

**Step 1: 为 chat/history 新增失败测试，锁定需要贯通的工具身份字段**

在 `src/ui/widgets/chat-history.test.ts` 写至少两个测试：

1. `tool_result` metadata 中含 `tool_id/source/server_name/needs_feedback` 时，历史消息能正确恢复到 `ToolTraceItem`
2. 旧历史消息缺这些字段时，仍能回退到 `tool_name` / `tool` 文本展示

测试数据结构应覆盖：

```ts
const metadata = JSON.stringify({
  type: 'tool_result',
  turn_id: 'turn-1',
  tool_call_id: 'call-1',
  tool_id: 'mcp__filesystem__read_file',
  tool_name: 'read_file',
  source: 'mcp',
  server_name: 'filesystem',
  needs_feedback: true,
})
```

并断言 `buildChatMessagesFromConversation(...)` 产出的 `tools[0]` 至少包含：

```ts
{
  tool: 'read_file',
  toolId: 'mcp__filesystem__read_file',
  source: 'mcp',
  serverName: 'filesystem',
  needsFeedback: true,
}
```

**Step 2: 运行前端单测，确认新增测试先失败**

Run:

```bash
npx vitest run src/ui/widgets/chat-history.test.ts
```

Expected:
- FAIL，提示 `ToolTraceItem` 缺少新字段或 history 未解析 metadata

**Step 3: 扩展前端类型，补齐 ToolTraceItem / ChatTurnToolEvent 字段**

修改 `src/lib/kokoro-bridge.ts`：

- 为 `ToolTraceItem` 增加：
  - `toolId?: string`
  - `source?: "builtin" | "mcp"`
  - `serverName?: string`
  - `needsFeedback?: boolean`
  - `permissionLevel?: "safe" | "elevated"`
  - `riskTags?: ("read" | "write" | "external" | "sensitive")[]`
- 为 `ChatTurnToolEvent` 增加：
  - `tool_name?: string`
  - `source?: "builtin" | "mcp"`
  - `server_name?: string`
  - `needs_feedback?: boolean`
  - `permission_level?: "safe" | "elevated"`
  - `risk_tags?: ...`

保持现有字段兼容，不删除 `tool`。

**Step 4: 更新 ChatPanel / ChatMessage / chat-history 使用新字段**

具体要求：

- `ChatPanel.tsx` 的 `buildToolTraceItem(...)` 在收到事件时保存上述字段
- `chat-history.ts` 从 metadata 恢复这些字段
- `ChatMessage.tsx` 后续展示逻辑统一优先使用：
  - 展示名：`tool` 或 `toolName`
  - 身份：`toolId`
  - 来源：`source + serverName`

此步先只保存与透传，不要求完整 UI 排版优化。

**Step 5: 运行前端测试验证通过**

Run:

```bash
npx vitest run src/ui/widgets/chat-history.test.ts src/ui/widgets/settings/mcpToolDisplay.test.ts
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src/lib/kokoro-bridge.ts src/ui/widgets/ChatPanel.tsx src/ui/widgets/ChatMessage.tsx src/ui/widgets/chat-history.ts src/ui/widgets/chat-history.test.ts src/ui/widgets/settings/mcpToolDisplay.test.ts
git commit -m "test: add tool identity coverage for chat history"
```

---

## Task 2: 为 assistant_tool_calls 与 tool_result 持久化完整 metadata

**Files:**
- Modify: `src-tauri/src/commands/chat.rs:1527-1603`
- Modify: `src-tauri/src/llm/messages.rs:25-68`
- Modify: `src-tauri/src/commands/conversation.rs:120-180`（若需要同步过滤逻辑）
- Modify: `src-tauri/src/ai/memory.rs:820-850`（若需要同步技术行识别）
- Test: `src-tauri/src/commands/chat.rs`
- Test: `src-tauri/src/llm/messages.rs`

**Step 1: 在 Rust 侧新增失败测试，锁定 metadata 结构**

在 `src-tauri/src/commands/chat.rs` 增加测试，断言 assistant/tool 持久化 metadata 至少包含：

- assistant tool call metadata：
  - `type = assistant_tool_calls`
  - `turn_id`
  - 每个 tool call 的：`id/tool_id/tool_name/source/server_name/needs_feedback/arguments`
- tool result metadata：
  - `type = tool_result`
  - `turn_id`
  - `tool_call_id`
  - `tool_id`
  - `tool_name`
  - `source`
  - `server_name`
  - `needs_feedback`

同时在 `src-tauri/src/llm/messages.rs` 增加测试，验证旧 metadata 缺字段时仍能正常回放。

**Step 2: 运行 Rust 编译级测试，确认先失败**

Run:

```bash
cd src-tauri && cargo test commands::chat --no-run && cargo test llm::messages --no-run
```

Expected:
- 编译失败或测试断言代码尚未通过

**Step 3: 修改 chat 持久化逻辑，补齐 metadata**

修改 `src-tauri/src/commands/chat.rs`：

- 将 `continuation_tool_calls` 从三元组 `(tool_call_id, tool_name, arguments)` 升级为携带完整身份信息的结构
- 将 `persisted_native_tool_results` 从三元组升级为携带：
  - `tool_call_id`
  - `tool_id`
  - `tool_name`
  - `source`
  - `server_name`
  - `needs_feedback`
  - `permission_level`
  - `risk_tags`
  - `tool_message`
- `assistant_tool_calls` metadata 写入完整字段
- `tool_result` metadata 写入完整字段

保持 `assistant_tool_calls_message(...)` 仍向模型发送 `id/name/arguments`，但历史 metadata 要完整。

**Step 4: 调整 llm history 恢复逻辑，确保兼容新旧 metadata**

修改 `src-tauri/src/llm/messages.rs`：

- `history_message_to_chat_message(...)` 继续要求模型所需字段最小闭环
- 读取 `assistant_tool_calls` metadata 时，优先使用 `tool_name`，缺失时回退 `name`
- 对新增身份字段不报错、不阻塞旧消息恢复

必要时同步更新 `commands/conversation.rs`、`ai/memory.rs` 对技术行的识别，确保不会误把新增 metadata 当成普通消息。

**Step 5: 运行 Rust 验证**

Run:

```bash
cd src-tauri && cargo check && cargo test commands::chat --no-run && cargo test --lib --no-run
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/commands/chat.rs src-tauri/src/llm/messages.rs src-tauri/src/commands/conversation.rs src-tauri/src/ai/memory.rs
git commit -m "feat: persist canonical tool identity metadata"
```

---

## Task 3: 补齐实时 chat-turn-tool 事件身份与权限字段

**Files:**
- Modify: `src-tauri/src/commands/chat.rs:533-689`
- Modify: `src-tauri/src/actions/executor.rs:22-28`
- Modify: `src/lib/kokoro-bridge.ts:242-262`
- Modify: `src/ui/widgets/ChatPanel.tsx:94-118`
- Test: `src-tauri/src/commands/chat.rs`

**Step 1: 新增事件 payload 失败测试**

在 `src-tauri/src/commands/chat.rs` 增加测试，验证成功、失败、pending approval、approved/rejected payload 都包含：

- `tool`
- `tool_id`
- `source`
- `server_name`
- `needs_feedback`
- `permission_level`
- `risk_tags`
- 已有的 `deny_kind` / `approval_request_id` / `approval_status`

**Step 2: 运行 Rust 编译级测试，确认先失败**

Run:

```bash
cd src-tauri && cargo test commands::chat --no-run
```

Expected:
- FAIL 或编译不通过

**Step 3: 扩展 ToolExecutionOutcome 与 chat trace payload**

修改 `src-tauri/src/actions/executor.rs`：

- 为 `ToolExecutionOutcome` 增加统一 helper：
  - `tool_name()`
  - `tool_id()`
  - `tool_source()`
  - `tool_server_name()`
  - `tool_needs_feedback()`
  - `tool_permission_level()`
  - `tool_risk_tags()`

修改 `src-tauri/src/commands/chat.rs`：

- `tool_success_payload(...)`
- `tool_error_payload(...)`
- `pending_tool_trace_payload(...)`
- `approved_tool_trace_payload(...)`
- `approved_tool_error_payload(...)`
- `rejected_tool_trace_payload(...)`

统一补齐上述字段。

**Step 4: 更新 ChatPanel 事件映射**

确保 `buildToolTraceItem(...)` 将后端新增字段完整映射到 `ToolTraceItem`。

**Step 5: 运行验证**

Run:

```bash
cd src-tauri && cargo check && cargo test commands::chat --no-run
npx vitest run src/ui/widgets/chat-history.test.ts
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/actions/executor.rs src-tauri/src/commands/chat.rs src/lib/kokoro-bridge.ts src/ui/widgets/ChatPanel.tsx
git commit -m "feat: enrich chat tool trace payloads with identity"
```

---

## Task 4: 重构 MCP 设置页分组展示与工具卡片信息

**Files:**
- Modify: `src/ui/widgets/settings/McpTab.tsx:83-360`
- Modify: `src/ui/widgets/settings/mcpToolDisplay.ts:1-24`
- Modify: `src/ui/widgets/settings/mcpToolDisplay.test.ts`
- Modify: `src/ui/i18n.ts`（如需补最小文案键）

**Step 1: 为分组与展示逻辑写失败测试**

在 `src/ui/widgets/settings/mcpToolDisplay.test.ts` 增加测试，覆盖：

1. builtin 工具与 MCP 工具分组
2. MCP 工具按 `server_name` 二级分组
3. 工具来源标签、风险标签、权限级别展示文案
4. server_name 缺失时的回退行为

**Step 2: 运行测试并确认先失败**

Run:

```bash
npx vitest run src/ui/widgets/settings/mcpToolDisplay.test.ts
```

Expected:
- FAIL

**Step 3: 抽取分组 helper，避免把分组逻辑塞进组件 JSX**

在 `src/ui/widgets/settings/mcpToolDisplay.ts` 增加纯函数，例如：

```ts
export interface ToolGroup {
  key: string
  title: string
  tools: ActionInfo[]
}

export function groupToolsForDisplay(tools: ActionInfo[]): ToolGroup[]
export function getToolRiskTagsLabel(...)
export function getToolPermissionLevelLabel(...)
```

要求：
- Built-in 单独一组
- MCP 先归大组，再按 `server_name` 分子组
- 未命名 MCP server 回退到固定文案

**Step 4: 修改 McpTab，接入现有 MCP 页分组展示**

更新 `src/ui/widgets/settings/McpTab.tsx`：

- 不再直接 `tools.map(...)`
- 改为渲染分组后的结构
- 每个工具项展示：
  - 名称
  - canonical `tool.id`
  - 来源标签
  - risk tags
  - permission level
  - 开关
- 继续使用 `enabled_tools[tool.id]` 保存状态

**Step 5: 运行前端验证**

Run:

```bash
npx vitest run src/ui/widgets/settings/mcpToolDisplay.test.ts
npx tsc --noEmit
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src/ui/widgets/settings/McpTab.tsx src/ui/widgets/settings/mcpToolDisplay.ts src/ui/widgets/settings/mcpToolDisplay.test.ts src/ui/i18n.ts
git commit -m "feat: group tool directory in MCP settings"
```

---

## Task 5: 收口 direct action / chat / Telegram 输入适配到 canonical id

**Files:**
- Modify: `src-tauri/src/commands/actions.rs:1-340`
- Modify: `src-tauri/src/commands/chat.rs:554-583`
- Modify: `src-tauri/src/telegram/bot.rs:380-450,760-820`
- Modify: `src-tauri/src/actions/registry.rs`（若需要新增统一解析 helper）
- Test: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/telegram/bot.rs`

**Step 1: 写失败测试，锁定“边界兼容、核心只吃 canonical id”**

新增或扩展测试覆盖：

1. direct execute 传裸 `name` 时能解析到 `tool_id`
2. 传 canonical `tool_id` 时直接命中
3. builtin 与 MCP 同名时优先规则明确（建议：canonical id 精确匹配优先，裸名仅在无歧义时允许）
4. 歧义裸名输入返回明确错误，不静默猜测

**Step 2: 运行 Rust 编译级测试，确认先失败**

Run:

```bash
cd src-tauri && cargo test commands::actions --no-run && cargo test telegram::bot --no-run
```

Expected:
- FAIL 或编译不通过

**Step 3: 在 registry 或命令边界新增统一解析 helper**

推荐在 `src-tauri/src/actions/registry.rs` 增加统一 helper，例如：

```rust
fn resolve_action_id_for_input(input: &str) -> Result<String, ActionLookupError>
```

要求：
- canonical id 直接命中
- 裸 name 仅在唯一匹配时允许
- builtin 与 MCP 冲突时返回歧义错误
- 执行核心统一拿 `ActionInfo` / canonical `tool_id`

**Step 4: 更新各入口使用统一解析 helper**

需要至少覆盖：
- `commands/actions.rs`
- `commands/chat.rs`
- `telegram/bot.rs`

目标：
- 兼容层停留在边界
- 内部执行与日志统一使用 canonical id

**Step 5: 运行验证**

Run:

```bash
cd src-tauri && cargo check && cargo test commands::actions --no-run && cargo test telegram::bot --no-run
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/commands/actions.rs src-tauri/src/commands/chat.rs src-tauri/src/telegram/bot.rs src-tauri/src/actions/registry.rs
git commit -m "refactor: resolve tool inputs to canonical ids at boundaries"
```

---

## Task 6: 抽离统一权限决策模型

**Files:**
- Create: `src-tauri/src/actions/permission.rs`
- Modify: `src-tauri/src/actions/mod.rs`
- Modify: `src-tauri/src/actions/executor.rs:96-194`
- Modify: `src-tauri/src/commands/actions.rs:223-295`
- Modify: `src-tauri/src/commands/chat.rs:613-689,1490-1525`
- Test: `src-tauri/src/actions/permission.rs`
- Test: `src-tauri/src/actions/executor.rs`

**Step 1: 先写 permission 模块失败测试**

在新文件 `src-tauri/src/actions/permission.rs` 中先写测试，覆盖：

1. `Allow`
2. `DenyPolicy`（permission ceiling / blocked read / blocked external）
3. `DenyPendingApproval`（elevated 非 sensitive、blocked write、safe sensitive）
4. `DenyFailClosed`（elevated + sensitive、blocked sensitive）
5. 多条件同时命中时的优先级顺序

建议定义枚举：

```rust
pub enum PermissionDecision {
    Allow,
    DenyPolicy { reason: String },
    DenyPendingApproval { reason: String },
    DenyFailClosed { reason: String },
}
```

**Step 2: 运行 Rust 编译级测试，确认先失败**

Run:

```bash
cd src-tauri && cargo test actions::permission --no-run
```

Expected:
- FAIL，因为模块尚不存在或逻辑未实现

**Step 3: 实现统一权限决策模块**

在 `src-tauri/src/actions/permission.rs`：

- 抽出 `risk_tag_label(...)`
- 抽出 `exceeds_safe_permission_ceiling(...)`
- 抽出 `evaluate_permission_decision(action, settings)`
- 保留稳定 reason 文案，但只在该模块统一生成

要求：
- 执行器不再自己组合三套独立判断
- 统一由 `PermissionDecision` 驱动后续逻辑

**Step 4: 将 executor 与命令层切换到 PermissionDecision**

修改 `src-tauri/src/actions/executor.rs`：

- 删除/下沉 `policy_denial_reason`、`approval_pending_reason`、`high_risk_fail_closed_reason` 的核心判定职责
- 保留必要兼容 helper 时，也应转调 permission 模块
- `execute_tool_calls(...)` 按 `PermissionDecision` 分发

修改 `src-tauri/src/commands/actions.rs` / `chat.rs`：

- 统一消费 `PermissionDecision`
- 不再依赖字符串前缀做业务分支

**Step 5: 运行验证**

Run:

```bash
cd src-tauri && cargo check && cargo test actions::permission --no-run && cargo test actions::executor --no-run && cargo test commands::actions --no-run
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/actions/permission.rs src-tauri/src/actions/mod.rs src-tauri/src/actions/executor.rs src-tauri/src/commands/actions.rs src-tauri/src/commands/chat.rs
git commit -m "refactor: centralize tool permission decisions"
```

---

## Task 7: 统一审批触发与 trace 决策语义

**Files:**
- Modify: `src-tauri/src/commands/chat.rs:613-689,1490-1525`
- Modify: `src/lib/kokoro-bridge.ts:242-262`
- Modify: `src/ui/widgets/ChatPanel.tsx:170-317`
- Modify: `src/ui/widgets/ChatMessage.tsx:37-133`
- Test: `src-tauri/src/commands/chat.rs`
- Test: `src/ui/widgets/chat-history.test.ts`

**Step 1: 写失败测试，确认审批 flow 基于统一决策而不是前缀文本**

测试要求：

1. pending approval payload 由统一决策结果触发
2. approved/rejected 后 trace 仍保留原工具身份字段
3. 前端收到 resolved payload 后不会丢失 `toolId/source/serverName`

**Step 2: 运行测试，确认先失败**

Run:

```bash
cd src-tauri && cargo test commands::chat --no-run
npx vitest run src/ui/widgets/chat-history.test.ts
```

Expected:
- FAIL

**Step 3: 更新 chat 审批流只消费结构化决策**

修改 `src-tauri/src/commands/chat.rs`：

- pending approval 分支改为只看 `PermissionDecision::DenyPendingApproval`
- fail-closed / policy deny 分支改为只看对应决策枚举
- `deny_kind` 继续保留给前端，但来源于结构化决策，而不是字符串前缀推断

**Step 4: 更新前端 trace 展示逻辑**

确保 `ChatPanel.tsx` / `ChatMessage.tsx`：

- 不丢失工具身份字段
- pending → approved/rejected 替换时保留 `toolId/source/serverName`
- 继续显示 denyKind / approvalStatus，但不再假设只有文本语义可用

**Step 5: 运行验证**

Run:

```bash
cd src-tauri && cargo check && cargo test commands::chat --no-run
npx tsc --noEmit
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/commands/chat.rs src/lib/kokoro-bridge.ts src/ui/widgets/ChatPanel.tsx src/ui/widgets/ChatMessage.tsx src/ui/widgets/chat-history.test.ts
git commit -m "refactor: drive approval flow from permission decisions"
```

---

## Task 8: 引入第一版审计事件结构并接入最小可视化

**Files:**
- Create: `src-tauri/src/actions/audit.rs`
- Modify: `src-tauri/src/actions/mod.rs`
- Modify: `src-tauri/src/actions/executor.rs`
- Modify: `src-tauri/src/commands/chat.rs`
- Modify: `src/ui/widgets/settings/McpTab.tsx`
- Test: `src-tauri/src/actions/audit.rs`

**Step 1: 先写审计结构测试**

在 `src-tauri/src/actions/audit.rs` 先写测试，锁定最小字段：

```rust
ToolAuditEvent {
    tool_id,
    tool_name,
    source,
    server_name,
    invocation_source,
    risk_tags,
    permission_level,
    decision,
    reason,
    approved_by_user,
    conversation_id,
    character_id,
}
```

测试至少覆盖：
- allow
- policy deny
- pending approval
- fail-closed
- approved after pending approval

**Step 2: 运行测试并确认先失败**

Run:

```bash
cd src-tauri && cargo test actions::audit --no-run
```

Expected:
- FAIL

**Step 3: 实现最小审计结构并接到执行链**

在 `src-tauri/src/actions/audit.rs` 定义：

- `ToolAuditDecision`
- `ToolAuditEvent`
- 必要的 builder/helper

在 executor / chat 中：
- 每次权限判定后生成统一审计事件
- 本轮可以只先日志输出 / 事件透传，不强制入库
- 保持实现最小，不引入复杂持久化

**Step 4: 在现有 MCP 设置页补权限最小可视化**

更新 `src/ui/widgets/settings/McpTab.tsx`：

- 每个工具卡片显示 risk tags
- 显示 permission level
- 样式保持现有页面风格，不新做复杂控制区

**Step 5: 运行全量验证**

Run:

```bash
npx tsc --noEmit
cd src-tauri && cargo check && cargo check --tests && cargo test --lib --no-run
```

Expected:
- PASS

**Step 6: 提交**

```bash
git add src-tauri/src/actions/audit.rs src-tauri/src/actions/mod.rs src-tauri/src/actions/executor.rs src-tauri/src/commands/chat.rs src/ui/widgets/settings/McpTab.tsx
git commit -m "feat: add tool audit model and permission visibility"
```

---

## Task 9: 最终回归验证与计划收口

**Files:**
- Modify: `docs/_wip_p0_unified_tool_contract_progress.md`
- Modify: `docs/_wip_p2_hook_runtime_progress.md`（若需要补充衔接说明）
- Modify: `docs/plans/2026-04-05-p0-unified-tool-contract-completion-and-permission-phase-1-design.md`（仅在必要时同步结论）

**Step 1: 更新进度文档，记录完成项与验证结果**

更新 `docs/_wip_p0_unified_tool_contract_progress.md`：

- 标记 metadata 已补齐
- 标记设置页分组已接入现有 MCP 页
- 标记 pseudo/native 输入适配已收口
- 标记权限主线 Phase 1 已完成哪些内容
- 记录实际验证命令与已知限制

若 P2 文档需要增加“后续权限主线接管 deny/approval 语义”的说明，可做最小补充。

**Step 2: 运行最终验证**

Run:

```bash
npx tsc --noEmit
cd src-tauri && cargo check && cargo check --tests && cargo test --lib --no-run && cargo clippy -- -D warnings
```

Expected:
- PASS（若运行级测试因 Windows 环境限制不可稳定执行，需在文档中明确说明）

**Step 3: 检查改动范围与状态**

Run:

```bash
git status
git diff -- docs/_wip_p0_unified_tool_contract_progress.md docs/plans/2026-04-05-p0-unified-tool-contract-completion-and-permission-phase-1-design.md
```

Expected:
- 仅包含本计划涉及文件
- 文档与代码状态一致

**Step 4: 提交最终收口**

```bash
git add docs/_wip_p0_unified_tool_contract_progress.md docs/_wip_p2_hook_runtime_progress.md docs/plans/2026-04-05-p0-unified-tool-contract-completion-and-permission-phase-1-design.md
git commit -m "docs: update tool contract and permission phase progress"
```

---

## 执行顺序建议

严格按以下顺序执行：

1. Task 1 — 前端 trace/history 类型铺底
2. Task 2 — Rust 持久化 metadata 补齐
3. Task 3 — 实时 chat-turn-tool 事件补齐身份与权限字段
4. Task 4 — MCP 设置页分组与最小展示
5. Task 5 — 输入适配收口到 canonical id
6. Task 6 — 统一权限决策模型
7. Task 7 — 审批 flow 切换到结构化决策
8. Task 8 — 审计结构与最小权限可视化
9. Task 9 — 文档与最终验证收口

不要调换 5/6/7 的顺序；权限主线必须建立在稳定工具身份模型上。

---

## 最终验收标准

完成后必须满足：

- 所有工具身份主键统一为 canonical `tool_id`
- assistant/tool 消息 metadata 完整携带 `tool_id/source/server_name/needs_feedback`
- 实时 trace、历史回放、MCP 设置页消费同一套身份模型
- 裸 `name` 兼容只停留在边界层
- policy deny / pending approval / fail-closed 基于统一权限决策模型
- 现有 MCP 设置页可看到来源、风险标签、权限级别
- 后续审批 UI、审计持久化、远程控制安全约束可以在此基础上继续演进
