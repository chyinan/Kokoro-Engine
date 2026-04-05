# P0 统一工具契约收口与多层权限主线 Phase 1 设计

> **状态**：已确认设计
> **日期**：2026-04-05
> **范围**：先补完 P0 统一工具契约尾项，再立刻进入多层权限与安全边界主线的第一阶段实现

---

## 1. 背景

Kokoro Engine 已经完成以下关键基础工作：

- P0 已完成 canonical tool id、统一工具目录、共享执行器等主链路落地
- P1 已完成分层上下文压缩与长期记忆优先注入
- P2 已完成 HookRuntime、deny 语义、pending approval 与审批 trace 基础展示

当前最合理的下一步不是继续扩更多能力，而是继续把系统边界做稳：

1. 先把 P0 统一工具契约剩余尾项彻底收口
2. 再在统一身份模型之上进入多层权限与安全边界主线

---

## 2. 目标

本轮目标分成两段连续推进：

### 阶段 A：补完 P0 统一工具契约尾项

完成以下三件事：

1. 为 `assistant_tool_calls` / `tool_result` / trace / history 相关 metadata 补齐：
   - `tool_id`
   - `tool_name`
   - `source`
   - `server_name`
   - `needs_feedback`
2. 将工具目录与设置展示继续收口到现有 **MCP 设置页**，按 `source` / `server_name` 分组展示
3. 将 pseudo / native / 兼容入口继续收口到 canonical tool id 管线，确保执行核心不再依赖裸 `name`

### 阶段 B：正式进入多层权限主线 Phase 1

在已统一的工具身份模型上，引入第一版统一权限判定与最小可视化能力：

1. 标准化能力声明层
2. 抽离统一权限判定层
3. 统一审批触发语义
4. 补齐第一版审计事件结构
5. 在现有 MCP 设置页与 trace 展示中做最小权限可视化

---

## 3. 设计原则

### 3.1 工具身份必须唯一且稳定

真实工具身份一律使用 canonical `tool_id`。`tool_name` 仅用于展示或兼容旧数据，不作为核心判定键。

### 3.2 兼容层留在边界，不进入执行核心

允许外部入口在短期内继续接受裸 `name` 或旧格式，但进入执行器前必须统一解析为 canonical `tool_id`。

### 3.3 先统一“工具是谁”，再统一“工具能不能做”

权限主线必须建立在稳定的工具身份模型上，避免再次出现按裸名、按来源猜测、按 trace 文案前缀反推语义的分叉逻辑。

### 3.4 最小可视化优先于复杂控制台

本轮权限主线只做最小但稳定的 UI 接入：

- 继续接到现有 **MCP 设置页**
- 展示来源、风险标签、权限级别、判定语义
- 不新建独立权限中心
- 不做复杂审批队列 UI

---

## 4. P0 收口设计

### 4.1 统一 metadata 结构

为所有工具调用与工具结果相关消息补齐统一 metadata 结构：

- `tool_id`: canonical id，唯一主键
- `tool_name`: 展示名/兼容字段
- `source`: `builtin` / `mcp`
- `server_name`: MCP 服务名，builtin 为 `null`
- `needs_feedback`: 保留交互语义

### 4.2 贯通范围

以下链路都应读取或写入同一身份结构：

- assistant tool call 消息
- tool result 消息
- 聊天实时 trace
- 历史消息回放
- Telegram 等其他工具调用入口

### 4.3 设置页接入方式

不新增独立权限页，而是在现有 **MCP 设置页** 中继续收口。

分组方式：

- Built-in Tools
- MCP Tools
  - server A
  - server B
  - server C

每个工具项最少展示：

- `tool.name`
- `tool.id`
- 来源标签
- 描述
- 风险标签
- 权限级别
- 当前启用状态

### 4.4 输入适配收口

明确执行链规则：

- 边界入口允许旧输入
- 解析层负责 `name -> tool_id`
- 执行核心只消费 `tool_id` 与 `ActionInfo`

重点收口位置：

- `commands/actions.rs`
- `commands/chat.rs`
- `telegram/bot.rs`
- 其他仍使用裸名进入执行链的路径

---

## 5. 多层权限主线 Phase 1 设计

### 5.1 能力声明层

以现有 `ActionInfo` 为统一权限输入模型，后续权限判断只依赖下列字段：

- `id`
- `source`
- `server_name`
- `risk_tags`
- `permission_level`

### 5.2 统一权限判定层

将当前执行器中的 deny / pending / fail-closed 逻辑抽离为统一决策模型。内部语义应至少支持：

- `Allow`
- `DenyPolicy { reason }`
- `DenyPendingApproval { reason }`
- `DenyFailClosed { reason }`

要求：

- 内部先产出结构化决策
- 文案字符串只在边界层生成
- 前端 trace 与历史展示消费稳定语义，而不是继续依赖文本前缀推断

### 5.3 审批触发语义统一

pending approval 的触发应基于统一权限决策层，而不是由各调用链自行拼接分支。

本轮不做：

- 批量审批
- 审批超时策略
- 审批队列中心

本轮要做的是让审批成为统一决策的一种结果。

### 5.4 审计事件结构

Phase 1 先统一结构，不强制要求立刻落数据库。

最小事件字段建议包含：

- `tool_id`
- `tool_name`
- `source`
- `server_name`
- `invocation_source`（chat / telegram / mod / system）
- `risk_tags`
- `permission_level`
- `decision`
- `reason`
- `approved_by_user`
- `conversation_id`
- `character_id`

这份结构将同时服务于：

- trace 面板
- 调试
- 后续安全审计
- 远程控制场景约束

---

## 6. 数据流设计

目标数据流如下：

1. 入口接收工具调用请求
2. 边界层解析为 canonical `tool_id`
3. 通过目录拿到完整 `ActionInfo`
4. 统一权限判定层给出决策：
   - allow
   - deny_policy
   - deny_pending_approval
   - deny_fail_closed
5. 如果允许，则进入共享执行器执行
6. 产出统一结果事件与审计事件
7. 前端 trace / history / MCP 设置页消费同一套身份与权限语义

这个收口顺序意味着：

- P0 解决“工具是谁”
- Phase 1 解决“工具能不能做”
- 后续审批 UI 再解决“谁来批、怎么批”

---

## 7. 里程碑拆分

### M1：补齐统一工具 metadata

**目标**：把 `tool_id / source / server_name` 从工具目录贯通到消息、trace、历史与结果链路。

**重点位置**：
- `src-tauri/src/llm/messages.rs`
- `src-tauri/src/commands/chat.rs`
- `src-tauri/src/telegram/bot.rs`
- `src/lib/kokoro-bridge.ts`
- `src/ui/widgets/ChatPanel.tsx`
- `src/ui/widgets/ChatMessage.tsx`
- `src/ui/widgets/chat-history.ts`

**完成标准**：
- assistant tool call metadata 完整
- tool result metadata 完整
- 实时与历史 trace 可共享同一身份字段
- 旧历史数据缺字段时可降级显示

### M2：收口工具目录与输入适配

**目标**：把工具展示与执行入口统一到 canonical id。

**重点位置**：
- `src/ui/widgets/settings/McpTab.tsx`
- `src/ui/widgets/settings/mcpToolDisplay.ts`
- `src-tauri/src/commands/actions.rs`
- `src-tauri/src/commands/chat.rs`
- `src-tauri/src/telegram/bot.rs`

**完成标准**：
- MCP 设置页按 `source/server_name` 分组
- 工具项展示 risk tags 与 permission level
- enabled 开关继续以 `tool.id` 为键
- 所有入口兼容旧输入但最终收敛到 canonical id

### M3：抽离统一权限判定层

**目标**：把 policy / pending / fail-closed 逻辑从执行器中的字符串分支提升为统一决策模型。

**重点位置**：
- `src-tauri/src/actions/executor.rs`
- 新增权限判定模块（命名待实现时确认）

**完成标准**：
- 返回统一决策模型
- 保持现有 deny 语义不回退
- trace 展示与内部判定一致
- 单元测试覆盖四类决策与优先级

### M4：接入 Phase 1 审计与最小可视化

**目标**：在不做复杂审批系统重构的前提下，完成权限主线第一版闭环。

**重点位置**：
- executor / chat / actions 命令链
- trace payload
- 现有 MCP 设置页

**完成标准**：
- 设置页能看到来源、风险、权限级别
- trace 能稳定展示拒绝原因与身份信息
- 后端具备统一审计事件结构
- 为后续审批 UI 留出稳定字段

---

## 8. 验证策略

### P0 收口验证

- `tool_id/source/server_name` 能贯通到 chat trace、history、tool result
- builtin / MCP 同名工具不会冲突
- 裸 `name` 兼容入口仍可用，但最终解析为 canonical id
- MCP 设置页分组与展示正确

### Phase 1 验证

- 同一工具在不同入口下可稳定识别
- policy deny / pending approval / fail-closed 由统一判定层给出
- trace 展示与内部判定一致
- 审计事件字段完整
- 不破坏当前 approve/reject 基础交互语义

建议验证命令沿用当前项目已有基线：

- `npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test --lib --no-run`
- 如条件允许，再补充与 actions/chat/hooks 相关的编译级测试

---

## 9. 非目标

本轮明确不做：

- 独立权限中心页面
- 完整审批中心 / pending queue UI
- per-tool 细粒度审批规则编辑器
- 审计数据库落表与检索面板
- MOD manifest 权限体系重构
- Telegram / MOD / MCP 各自独立的复杂策略系统

本轮只做：

- **统一工具身份收口**
- **统一权限主线第一阶段**
- **在现有 MCP 设置页中的最小可视化接入**

---

## 10. 成功标准

本轮完成后，应达到以下状态：

1. 工具身份模型统一
2. 执行入口统一
3. 权限判定模型统一
4. trace 语义统一
5. 前端设置页已能承接工具目录与权限最小展示
6. 后续审批 UI、审计持久化、远程控制安全约束都能在此基础上继续迭代
