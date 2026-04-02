# P2 HookRuntime 中心化分发 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 落地一个后端统一的 HookRuntime，把 chat、action/tool、mod lifecycle 的 before/after 生命周期事件集中到稳定中间层中，先完成第一版观测与副作用分发能力，并为后续 deny / modify / fail-closed / manifest 声明 / 前端消费预留清晰扩展位。

**Architecture:** 本方案采用单一后端中心化运行时：新增 `hooks/` 模块定义统一事件枚举、结构化 payload、处理器 trait、运行时注册表与分发入口。第一版只做 `Continue` 语义，不改变现有主链路行为；chat、共享 tool executor、mod manager 仅在关键阶段调用 HookRuntime。计划同时把“后续接线”明确写入同一文档，避免 P2 第一版落地后失去演进上下文。

**Tech Stack:** Rust、Tauri v2、tokio、serde、现有 `ActionRegistry` / `execute_tool_calls(...)` / `AIOrchestrator` / `ModManager`

---

## Reality Check（已核实现状）

### 已确认的关键挂点
- `src-tauri/src/commands/chat.rs:447`：`stream_chat(...)` 是 chat 主链路总入口
- `src-tauri/src/commands/chat.rs:547-556`：`compose_prompt(...)` 在 LLM 请求前统一组装上下文
- `src-tauri/src/commands/chat.rs:564-570`：`chat-turn-start` 在回合开始时发射
- `src-tauri/src/commands/chat.rs:819-826`：工具调用统一经 `execute_tool_calls(...)`
- `src-tauri/src/commands/chat.rs:1347-1354`：`chat-turn-finish` 在回合收尾发射
- `src-tauri/src/actions/executor.rs:45-93`：`execute_tool_calls(...)` 是 Chat 工具调用共享执行器
- `src-tauri/src/actions/registry.rs:313-321`：`ActionRegistry::execute(...)` 仍存在直调入口（`execute_action` 等路径可用）
- `src-tauri/src/mods/manager.rs:233-394`：`load_mod(...)` 负责 theme/layout/components/scripts/init 整体加载生命周期
- `src-tauri/src/mods/manager.rs:423-428`：`unload_mod(...)` 是卸载收尾点
- `src-tauri/src/lib.rs:48-320`：Tauri 应用统一初始化入口，可注册 `HookRuntime` 到 `app.manage(...)`

### 已确认的约束
- 现有代码没有统一 Hook 中间层，只有零散 Tauri event emit
- `ModManifest` 当前只有 `permissions`，没有显式 lifecycle / hook 声明字段：`src-tauri/src/mods/manifest.rs:4-38`
- tool 共享执行器已经抽到 `actions/executor.rs`，因此 **before/after action invoke** 应优先接这里，而不是只接 `chat.rs`
- 第一版若直接引入 `deny` / `modify`，会立刻波及 chat 多轮 tool loop、mod load 顺序与错误策略，风险过大

---

## First Release Scope（P2 第一版必须完成）

### 目标边界
第一版只做：
1. 后端统一 HookRuntime
2. 统一事件类型与 payload 模型
3. chat / action/tool / mod lifecycle 三条主链路接入
4. 支持内建监听器注册与顺序分发
5. 支持 side-effect / audit / logging 型 hook
6. 默认 fail-open（hook 失败只记录，不中断主链路）

### 明确不做
第一版**不做**：
- deny
- modify payload
- 用户确认 UI
- 前端注册 hook API
- manifest 中声明式 hook 注册
- 按权限/风险标签拦截 tool
- 跨进程或远程 hook

---

## File Plan

### Create
- `src-tauri/src/hooks/mod.rs`
- `src-tauri/src/hooks/types.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/handlers.rs`
- `src-tauri/src/hooks/tests.rs`
- `docs/plans/2026-04-02-p2-hook-runtime-centralized.md`

### Modify
- `src-tauri/src/lib.rs`
- `src-tauri/src/commands/chat.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/mods/manager.rs`
- `src-tauri/src/mods/mod.rs`（如需导出 hooks 相关辅助类型时）

### Verify Against Existing Files
- `src-tauri/src/mods/manifest.rs`
- `src-tauri/src/commands/mods.rs`
- `src-tauri/src/ai/context.rs`
- `src-tauri/src/commands/actions.rs`

---

## Event Model（第一版固定下来）

### HookEvent
第一版统一事件枚举：
- `BeforeUserMessage`
- `AfterUserMessagePersisted`
- `BeforeLlmRequest`
- `AfterLlmResponse`
- `BeforeActionInvoke`
- `AfterActionInvoke`
- `OnModLoaded`
- `OnModUnloaded`

### HookPayload
要求使用**结构化 payload**，不要复用裸 `serde_json::Value` 作为顶层接口。建议拆为：
- `HookPayload::Chat(ChatHookPayload)`
- `HookPayload::Action(ActionHookPayload)`
- `HookPayload::Mod(ModHookPayload)`

其中建议字段：

#### ChatHookPayload
- `conversation_id: Option<String>`
- `character_id: String`
- `turn_id: Option<String>`
- `message: Option<String>`
- `response: Option<String>`
- `tool_round: Option<usize>`
- `hidden: bool`

#### ActionHookPayload
- `conversation_id: Option<String>`
- `character_id: String`
- `tool_call_id: Option<String>`
- `action_id: Option<String>`
- `action_name: String`
- `args: HashMap<String, String>`
- `success: Option<bool>`
- `result_message: Option<String>`
- `source: Option<String>`（如 `chat`, `telegram`, `direct_execute`）

#### ModHookPayload
- `mod_id: String`
- `stage: String`（第一版可保留，例如 `loaded` / `unloaded`）
- `has_theme: bool`
- `has_layout: bool`
- `component_count: usize`
- `script_count: usize`

### HookOutcome
第一版只定义：
- `Continue`

但类型设计要预留后续枚举位：
- `Deny { reason }`
- `ReplacePayload(...)`
- `FailClosed { reason }`

这些先不要在实现里启用，只在注释与类型位置预留演进空间。

---

## Runtime Design

### HookHandler trait
建议形态：
- `fn id(&self) -> &str`
- `fn events(&self) -> &'static [HookEvent]`
- `async fn handle(&self, event: &HookEvent, payload: &HookPayload) -> Result<HookOutcome, String>`

### HookRuntime
负责：
1. 注册 handler
2. 为单个事件查找订阅者
3. 顺序执行 handler
4. 聚合结果
5. 记录 hook 错误日志
6. 第一版统一返回 `HookOutcome::Continue`

建议内部结构：
- `handlers: Vec<Arc<dyn HookHandler>>`
- 或 `HashMap<HookEvent, Vec<Arc<dyn HookHandler>>>`

建议对外 API：
- `register(handler)`
- `emit(event, payload)`
- `emit_best_effort(event, payload)`

第一版推荐所有业务方调用 `emit_best_effort(...)`，内部 swallow handler error，仅日志输出。

### Built-in handlers（第一版最小内建监听器）
只做一个最小内建 handler：
- `AuditLogHookHandler`

职责：
- 统一打印 `[Hook] event=...` 日志
- 让第一版 hook 先具备可观测性

不要在第一版过早加入复杂 handler（如审批、策略路由、遥测上报）。

---

## Task Breakdown

### Task 1: 建立 hooks 模块骨架

**Files:**
- Create: `src-tauri/src/hooks/mod.rs`
- Create: `src-tauri/src/hooks/types.rs`
- Create: `src-tauri/src/hooks/runtime.rs`
- Create: `src-tauri/src/hooks/handlers.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/hooks/tests.rs`

**Step 1: 写失败测试，固定 HookRuntime 的最小行为**

在 `src-tauri/src/hooks/tests.rs` 中先写这些测试：
- 注册一个 handler 后，`emit(...)` 会调用它一次
- 同一事件多个 handler 按注册顺序执行
- handler 返回错误时，`emit_best_effort(...)` 不报错并继续后续 handler
- 不订阅某事件的 handler 不会被调用

测试应覆盖：
- `HookEvent`
- `HookPayload`
- `HookOutcome::Continue`
- `HookRuntime::register`
- `HookRuntime::emit_best_effort`

**Step 2: 跑测试确认失败**

Run:
- `cd src-tauri && cargo test hooks::tests -- --nocapture`

Expected:
- FAIL，提示 `hooks` 模块或相关类型不存在

**Step 3: 写最小实现**

实现：
- `HookEvent`
- `HookPayload`
- `HookOutcome`
- `HookHandler`
- `HookRuntime`
- `AuditLogHookHandler`

要求：
- 只实现第一版所需字段
- 不提前实现 deny / modify 逻辑
- 日志统一前缀 `[Hook]`

**Step 4: 跑测试确认通过**

Run:
- `cd src-tauri && cargo test hooks::tests -- --nocapture`

Expected:
- PASS

**Step 5: 小提交**

```bash
git add src-tauri/src/hooks src-tauri/src/lib.rs
git commit -m "feat: add centralized hook runtime skeleton"
```

---

### Task 2: 在 Tauri app 初始化时注册 HookRuntime

**Files:**
- Modify: `src-tauri/src/lib.rs:1-320`
- Test: `src-tauri/src/hooks/tests.rs`

**Step 1: 写失败测试或编译约束**

增加测试验证：
- `HookRuntime::new()` 可实例化
- 可注册默认 `AuditLogHookHandler`

**Step 2: 最小实现**

在 `src-tauri/src/lib.rs` 中：
- `pub mod hooks;`
- 在 `.setup(...)` 初始化阶段创建 `HookRuntime`
- 注册 `AuditLogHookHandler`
- 用 `app.manage(...)` 挂到 Tauri state

不要把 HookRuntime 塞进 AIOrchestrator，保持独立宿主层。

**Step 3: 验证编译**

Run:
- `cd src-tauri && cargo check`

Expected:
- PASS

**Step 4: 小提交**

```bash
git add src-tauri/src/lib.rs src-tauri/src/hooks
git commit -m "feat: register hook runtime in tauri app state"
```

---

### Task 3: 接入 chat 生命周期 hook

**Files:**
- Modify: `src-tauri/src/commands/chat.rs:447-1357`
- Verify: `src-tauri/src/ai/context.rs`
- Test: `src-tauri/src/hooks/tests.rs` 或 `src-tauri/src/commands/chat.rs` 新增轻量纯函数测试

**Step 1: 先固定挂点，不改行为**

必须只在以下阶段发 hook：
1. `BeforeUserMessage`
   - 在用户消息进入本轮处理、上下文组装前触发
2. `AfterUserMessagePersisted`
   - 在用户消息成功持久化后触发
3. `BeforeLlmRequest`
   - 在 `compose_prompt(...)` 完成、真正开始 `chat_stream(_with_tools)` 前触发
4. `AfterLlmResponse`
   - 在最终 `full_response` 形成后、`chat-turn-finish` 前触发

**Step 2: 写失败测试 / 结构约束测试**

如果 `chat.rs` 难以直接做集成测试，就至少写 payload builder 纯函数测试，例如：
- chat payload 会带 `character_id`
- hidden 请求会带 `hidden=true`
- `AfterLlmResponse` 会带最终响应文本，不带原始流式碎片

**Step 3: 最小实现**

在 `stream_chat(...)` 中读取 `State<'_, HookRuntime>` 或 `app_handle.state::<HookRuntime>()`，并在上述 4 个阶段调用 `emit_best_effort(...)`。

注意：
- hook 失败不能影响 chat 主链路
- 不要在流式 delta 内频繁发 hook，避免事件风暴
- `AfterLlmResponse` 只发一次最终文本

**Step 4: 验证**

Run:
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test commands::chat -- --nocapture`

Expected:
- 编译通过
- 现有 tag 解析测试不回归

**Step 5: 小提交**

```bash
git add src-tauri/src/commands/chat.rs src-tauri/src/hooks
git commit -m "feat: emit centralized hooks for chat lifecycle"
```

---

### Task 4: 接入共享 action/tool 执行生命周期 hook

**Files:**
- Modify: `src-tauri/src/actions/executor.rs:45-93`
- Modify: `src-tauri/src/lib.rs`（如需导出状态访问辅助）
- Verify: `src-tauri/src/actions/registry.rs:313-321`
- Verify: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/hooks/tests.rs` 或 `src-tauri/src/actions/executor.rs` 新增测试

**Step 1: 定位唯一执行边界**

以 `execute_tool_calls(...)` 作为 chat 路径的统一 hook 注入点：
- `BeforeActionInvoke`
- `AfterActionInvoke`

并评估 `ActionRegistry::execute(...)` 是否也要最小接入，以覆盖 `execute_action` 直调路径。

**Step 2: 写失败测试**

建议写轻量单元测试：
- 工具 disabled 时，仍会发 `AfterActionInvoke`，并标记 `success=false`
- resolve 失败时，仍会发 `AfterActionInvoke`
- success 时，payload 包含 `action_id` 与 `result_message`

**Step 3: 最小实现**

在 `execute_tool_calls(...)` 内：
- 解析 action 前发 `BeforeActionInvoke`
- 结果生成后发 `AfterActionInvoke`

推荐给 `ActionContext` 增加**可选**字段而不是大改：
- `conversation_id: Option<String>`（如果当前路径拿不到，可先为 `None`）
- `source: Option<String>`

若当前改动太大，第一版允许：
- chat 路径 source 固定为 `chat`
- conversation_id 先缺省

**Step 4: 验证**

Run:
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test actions:: -- --nocapture`

Expected:
- PASS

**Step 5: 小提交**

```bash
git add src-tauri/src/actions/executor.rs src-tauri/src/actions/registry.rs src-tauri/src/hooks
git commit -m "feat: emit centralized hooks for tool execution"
```

---

### Task 5: 接入 mod lifecycle hook

**Files:**
- Modify: `src-tauri/src/mods/manager.rs:233-428`
- Verify: `src-tauri/src/commands/mods.rs:34-45`
- Verify: `src-tauri/src/mods/manifest.rs:4-38`
- Test: `src-tauri/src/mods/manager.rs` tests 或 `src-tauri/src/hooks/tests.rs`

**Step 1: 先固定第一版事件边界**

只发两个事件：
- `OnModLoaded`
  - 触发时机：theme/layout/components/scripts/init 全部走完后，`load_mod(...)` 成功返回前
- `OnModUnloaded`
  - 触发时机：`unload_mod(...)` 完成 active state 清理并 emit `mod:unload` 后

**Step 2: 写失败测试**

建议补轻量测试：
- `ModHookPayload` 会正确反映 theme/layout/components/scripts 数量
- `load_mod(...)` 成功路径会构造 `OnModLoaded` payload
- `unload_mod(...)` 会构造 `OnModUnloaded` payload

**Step 3: 最小实现**

在 `ModManager::load_mod(...)` 成功收尾处调用 HookRuntime。不要把 hook 插到 theme/layout/components/scripts 每个阶段里，避免第一版粒度过细。

在 `unload_mod(...)` 收尾处调用 HookRuntime。

**Step 4: 验证**

Run:
- `cd src-tauri && cargo test mods::manager -- --nocapture`
- `cd src-tauri && cargo check`

Expected:
- PASS

**Step 5: 小提交**

```bash
git add src-tauri/src/mods/manager.rs src-tauri/src/hooks
git commit -m "feat: emit centralized hooks for mod lifecycle"
```

---

### Task 6: 收尾验证与最小文档对齐

**Files:**
- Modify: `docs/plans/2026-04-02-p2-hook-runtime-centralized.md`
- Verify: `docs/claude-code-architecture-inspirations.md`

**Step 1: 补一段“已完成 / 未完成”边界说明**

文档里明确：
- 第一版只做 runtime + centralized events + audit side effects
- deny / modify / fail-closed / manifest 扩展全部列为后续接线，不要伪装成已完成能力

**Step 2: 跑整体验证**

Run:
- `npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test hooks::tests -- --nocapture`
- `cd src-tauri && cargo test actions:: -- --nocapture`
- `cd src-tauri && cargo test mods::manager -- --nocapture`
- `cd src-tauri && cargo test commands::chat -- --nocapture`

**Step 3: 若通过，再准备进入执行阶段**

不要在这里直接扩 scope。

---

## Follow-up Wiring Checklist（必须写清，避免后续断档）

下面这些**不是第一版范围**，但必须作为后续衔接清单保存在计划里。

### P2.1：接 `deny` 语义
目标：让 Hook 可以拒绝某次 action / mod / chat 阶段继续执行。

建议接入点：
- chat：`BeforeUserMessage`、`BeforeLlmRequest`
- action：`BeforeActionInvoke`
- mod：未来若新增 `BeforeModLoad`，则在真正 load 前拦截

要求：
- 先从 action deny 开始，不要先动 chat
- 所有 deny 必须返回结构化 reason
- 前端消费前，先日志化和内部可观测

### P2.2：接 `modify payload`
目标：允许 Hook 修改输入上下文，例如 tool args 或 prompt augment。

建议顺序：
1. 先支持 `BeforeLlmRequest` 添加 system note / metadata
2. 再支持 `BeforeActionInvoke` 改参数
3. 最后才考虑改 user message 原文

要求：
- 必须保留原始 payload 供审计
- 修改后 payload 与原始 payload 都要可记录

### P2.3：接 fail-closed 策略
目标：把部分高风险事件从 fail-open 升级成 fail-closed。

建议只对以下场景启用：
- 高风险 action
- 外部 MCP 调用
- manifest 声明了必须通过审批的 mod capability

要求：
- fail-closed 不能全局默认打开
- 需引入风险级别 / 权限标签后再接

### P2.4：接 manifest 生命周期 / 能力声明
目标：让 mod 明确声明自己订阅哪些 hook、请求哪些能力。

建议给 `ModManifest` 未来增加：
- `hooks: []`
- `capabilities: []` 或扩展现有 `permissions`
- `lifecycle: { on_load, on_unload }`（如仍需显式声明）

要求：
- 先 schema 扩展，再接运行时
- 缺失声明时默认不给额外能力
- 这是后续 fail-closed 的前置条件

### P2.5：接前端消费层
目标：前端先**消费** hook 结果，而不是自己注册 hook。

建议顺序：
1. 先在调试面板 / 日志面板展示 hook trace
2. 再考虑 UI 状态联动（如 tool invoke / mod load 时间线）
3. 最后才考虑开放前端 hook API

要求：
- 第一阶段仅展示，不参与决策
- 避免让 React 层反向侵入后端调度

### P2.6：接 Telegram / 其他入口统一覆盖
当前第一版重点覆盖 chat + shared executor + mod manager。
后续应继续核对：
- `src-tauri/src/telegram/bot.rs`
- `src-tauri/src/commands/actions.rs`
- 未来 vision / tts / stt 主链路

目标：所有“before/after 做点什么”的需求最终都汇入 HookRuntime，而不是再散落新回调。

---

## Guardrails

### DRY
- 不要在 `chat.rs` 和 `executor.rs` 各自发明一套 hook 类型
- 所有事件名、payload、结果模型都只能从 `hooks/types.rs` 导出

### YAGNI
- 第一版不要做策略 DSL
- 第一版不要做数据库持久化 hook trace
- 第一版不要做前端 hook 注册
- 第一版不要把每个 delta token 都变成 hook

### TDD
- 先给 `HookRuntime` 骨架写失败测试
- 再给 chat / tool / mod 三条接线补最小测试或结构约束测试
- 没有失败测试或至少失败编译信号，不要直接写实现

### Frequent Commits
- 每个 Task 一次 commit
- 不要把 skeleton、chat、tool、mod 全挤在一个大提交里

---

## Verification

### 必跑命令
- `npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test hooks::tests -- --nocapture`
- `cd src-tauri && cargo test actions:: -- --nocapture`
- `cd src-tauri && cargo test mods::manager -- --nocapture`
- `cd src-tauri && cargo test commands::chat -- --nocapture`

### 成功标准
- 新增 `hooks` 模块后 Rust 编译通过
- chat、tool、mod 三条主链路都能调用统一 HookRuntime
- 第一版不改变既有业务行为
- hook 错误不会打断主链路
- 计划文档明确写清后续接线清单

---

## Suggested Execution Order

1. Task 1：hooks 模块骨架 + 测试
2. Task 2：Tauri app 初始化注册 HookRuntime
3. Task 3：chat 生命周期接线
4. Task 4：共享 tool executor 接线
5. Task 5：mod lifecycle 接线
6. Task 6：全量验证 + 文档收尾

---

## Notes For Future Claude

如果后续继续做 P2 演进，优先顺序必须是：
1. `deny` on action
2. `modify` on llm request
3. 风险标签 / 权限级别
4. fail-closed
5. manifest 声明扩展
6. 前端 hook trace 消费

不要跳过前 3 项直接做“前端可注册 hook”，否则边界会重新变散。
