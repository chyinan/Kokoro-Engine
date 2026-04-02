# P2.1 HookRuntime deny on action Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 的 HookRuntime 增加第一阶段的 action deny 语义，让 `BeforeActionInvoke` 可以用结构化 reason 拒绝工具执行，同时保持 chat / mod 其他生命周期仍然是 fail-open。

**Architecture:** 本阶段只给 `BeforeActionInvoke` 开启“可中断”语义，其他事件继续走现有 `emit_best_effort(...)`。后端统一在 HookRuntime 中识别 `HookOutcome::Deny { reason }`，并由共享 tool executor 把 deny 转成一致的错误结果与 `AfterActionInvoke` 审计事件；不引入 modify payload，不启用全局 fail-closed，不开放前端注册。

**Tech Stack:** Rust、Tauri v2、tokio、serde、现有 `hooks/` 模块、`actions/executor.rs`、`commands/actions.rs`

---

## Reality Check（已核实现状）

### 已确认的当前实现
- `src-tauri/src/hooks/types.rs:4-60`：`HookOutcome` 目前只有 `Continue`
- `src-tauri/src/hooks/runtime.rs:21-64`：运行时只有 `emit(...)` 与 `emit_best_effort(...)`，还没有“只对 deny 生效”的专用分发入口
- `src-tauri/src/actions/executor.rs:141-232`：`execute_tool_calls(...)` 已在 `BeforeActionInvoke` / `AfterActionInvoke` 发 hook，但 deny 还不会真正阻断执行
- `src-tauri/src/commands/actions.rs:25-56`：`execute_action(...)` 仍是直调路径，目前没有接 HookRuntime deny
- `docs/plans/2026-04-02-p2-hook-runtime-centralized.md:502-513`：P2 原计划已明确 P2.1 要“先从 action deny 开始，不要先动 chat”

### 已确认的边界
- 本阶段只让 `BeforeActionInvoke` 可返回 `Deny { reason }`
- `AfterActionInvoke` 仍必须发，用于审计 denied / disabled / resolve failed / success 四种结果
- chat 生命周期、mod lifecycle 仍保持 fail-open；不能因为本阶段引入 deny 而改变它们的行为
- 不在本阶段接 `modify payload`、`fail-closed`、前端审批 UI、manifest 声明式 hook 注册

---

## First Release Scope（P2.1 本阶段必须完成）

### 要完成
1. 给 `HookOutcome` 增加 `Deny { reason: String }`
2. 给 `HookRuntime` 增加“允许 deny 冒泡”的 action gate 分发入口
3. 让 `execute_tool_calls(...)` 在 `BeforeActionInvoke` 收到 deny 时停止实际工具执行
4. deny 后仍发 `AfterActionInvoke`，并带 `success=false` 与结构化 reason 映射出的结果文本
5. 为 deny 路径补最小单元测试 / 结构测试

### 明确不做
- chat deny
- mod deny
- modify payload
- 全局 fail-closed
- 前端消费 deny UI
- manifest / capability schema 扩展
- 数据库存储 hook trace

---

## File Plan

### Create
- `docs/plans/2026-04-03-p2-1-hook-deny-on-action.md`

### Modify
- `src-tauri/src/hooks/types.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/tests.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`（如决定覆盖直调路径）

### Verify Against Existing Files
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/commands/chat.rs`
- `src-tauri/src/mods/manager.rs`
- `docs/plans/2026-04-02-p2-hook-runtime-centralized.md`

---

## Design Notes（本阶段固定下来）

### HookOutcome
本阶段把类型扩展为：
- `Continue`
- `Deny { reason: String }`

要求：
- `reason` 必须是结构化 deny 的最小载体
- 本阶段只在 `BeforeActionInvoke` 真正消费 `Deny`
- 其他事件就算某 handler 返回 `Deny`，也不要改变现有 fail-open 行为

### HookRuntime API
建议新增一个明确语义的方法，例如：
- `emit_action_gate(event, payload)`
- 或 `emit_enforced(event, payload)`

但本阶段要求它只在 action deny 路径使用。

建议语义：
- 遍历订阅当前事件的 handler
- `Continue` → 继续下一个 handler
- `Deny { reason }` → 立即返回 deny
- `Err(error)` → 记录日志并继续（仍保持 fail-open）

### Deny 映射到执行器的行为
当 `BeforeActionInvoke` 返回 deny：
- 不调用真实 action handler
- 生成统一的失败结果文本，例如：`Denied by hook: <reason>`
- 仍然构造 `ToolExecutionOutcome`
- 仍然发 `AfterActionInvoke`
- `AfterActionInvoke` payload 中：
  - `success = Some(false)`
  - `result_message = Some("Denied by hook: ...")`

### 直调路径策略
本阶段建议一开始就明确二选一：

**推荐：最小覆盖 `execute_action(...)` 直调路径**
- 原因：一旦 deny 只拦 chat 工具循环、不拦 `execute_action`，行为会不一致
- 做法：在 `commands/actions.rs` 中复用同一套 action gate 逻辑或薄包装 helper

如果实现时发现这会显著扩 scope，则退一步：
- 只覆盖 `execute_tool_calls(...)`
- 但必须在计划文档和最终实现说明中明确 `execute_action` 仍暂时绕过 deny

---

## Task Breakdown

### Task 1: 扩展 HookOutcome 与 action gate 运行时入口

**Files:**
- Modify: `src-tauri/src/hooks/types.rs:4-60`
- Modify: `src-tauri/src/hooks/runtime.rs:21-64`
- Test: `src-tauri/src/hooks/tests.rs:1-147`

**Step 1: 先写失败测试，固定 deny 行为**

在 `src-tauri/src/hooks/tests.rs` 中新增测试：
- 某个 handler 返回 `Deny { reason }` 时，新的 action gate emit 会立即返回 deny
- deny 后不再继续执行后续 handler
- handler 返回普通错误时，action gate 仍保持 fail-open，继续后续 handler

测试示例（按项目风格写 Rust）：

```rust
#[tokio::test]
async fn action_gate_returns_deny_and_stops_later_handlers() {
    let runtime = HookRuntime::new();
    // 第一个 handler deny，第二个 handler 不应被调用
}
```

**Step 2: 跑测试确认失败**

Run:
- `cd src-tauri && cargo test hooks::tests -- --nocapture`

Expected:
- FAIL，提示 `HookOutcome::Deny` 或新的 runtime gate API 尚不存在

**Step 3: 写最小实现**

在 `src-tauri/src/hooks/types.rs`：

```rust
pub enum HookOutcome {
    Continue,
    Deny { reason: String },
}
```

在 `src-tauri/src/hooks/runtime.rs` 新增最小 gate API，例如：

```rust
pub async fn emit_action_gate(
    &self,
    event: &HookEvent,
    payload: &HookPayload,
) -> HookOutcome {
    // Continue / Deny / Err(日志后继续)
}
```

要求：
- 不改 `emit_best_effort(...)` 的现有语义
- gate API 只为 deny 路径服务，不提前支持 replace payload

**Step 4: 跑测试确认通过**

Run:
- `cd src-tauri && cargo test hooks::tests -- --nocapture`

Expected:
- PASS

**Step 5: Commit**

```bash
git add src-tauri/src/hooks/types.rs src-tauri/src/hooks/runtime.rs src-tauri/src/hooks/tests.rs
git commit -m "feat: add deny outcome for action hook gate"
```

---

### Task 2: 在共享 tool executor 消费 deny 结果

**Files:**
- Modify: `src-tauri/src/actions/executor.rs:24-232`
- Verify: `src-tauri/src/actions/registry.rs:69-72,313-320`
- Test: `src-tauri/src/actions/executor.rs:48-116`

**Step 1: 先写失败测试**

在 `src-tauri/src/actions/executor.rs` 的现有测试模块中补测试：
- deny 时不会调用真实 action handler
- deny 时 `ToolExecutionOutcome.result` 为 `Err("Denied by hook: ...")`
- deny 时仍会构造 `AfterActionInvoke` payload，且 `success=false`
- disabled / resolve failed 路径不应被这次改动破坏

测试方向示例：

```rust
#[test]
fn build_action_hook_payload_carries_denied_result() {
    let payload = build_action_hook_payload(
        None,
        "char-1",
        Some("chat".to_string()),
        &sample_invocation(),
        Some(&sample_action()),
        Some(false),
        Some("Denied by hook: blocked".to_string()),
    );
    // assert success/result_message
}
```

如果要验证“真实 handler 未执行”，就新增最小 fake handler + 调用计数。

**Step 2: 跑测试确认失败**

Run:
- `cd src-tauri && cargo test actions:: -- --nocapture`

Expected:
- FAIL，deny 断路逻辑尚不存在

**Step 3: 写最小实现**

在 `execute_tool_calls(...)` 中：
1. `BeforeActionInvoke` 不再走 `emit_best_effort(...)`
2. 改为调用新的 runtime gate API
3. 若结果是 deny：
   - 直接生成 `Err(format!("Denied by hook: {}", reason))`
   - 跳过 resolve / enabled / handler.execute
4. 无论 deny 与否，`AfterActionInvoke` 仍照常发

最小逻辑参考：

```rust
let gate = hooks.emit_action_gate(...).await;
let result = match gate {
    HookOutcome::Deny { reason } => Err(format!("Denied by hook: {}", reason)),
    HookOutcome::Continue => {
        // 保持原有 resolve / enabled / execute 流程
    }
};
```

要求：
- 不重构整个 executor
- 不修改 payload 结构
- deny 的文本前缀保持统一，方便后续前端/日志消费

**Step 4: 跑测试确认通过**

Run:
- `cd src-tauri && cargo test actions:: -- --nocapture`
- `cd src-tauri && cargo check`

Expected:
- PASS

**Step 5: Commit**

```bash
git add src-tauri/src/actions/executor.rs src-tauri/src/hooks
git commit -m "feat: enforce action deny in shared tool executor"
```

---

### Task 3: 评估并最小覆盖 execute_action 直调路径

**Files:**
- Modify: `src-tauri/src/commands/actions.rs:25-56`
- Verify: `src-tauri/src/actions/registry.rs:313-320`
- Test: `src-tauri/src/commands/actions.rs` 或 `src-tauri/src/actions/executor.rs`

**Step 1: 先写失败测试或结构约束测试**

至少固定一个约束：
- 直调路径若接 deny，也必须返回带 deny reason 的错误文本
- 若本任务决定不覆盖，则测试应明确“当前只覆盖共享 executor，直调路径不变”并在文档记录

**Step 2: 选实现方案（推荐其一）**

#### 推荐方案：最小覆盖
在 `commands/actions.rs` 中复用与 executor 同一套 gate helper：
- 构造 `BeforeActionInvoke` payload
- 收到 deny 时直接返回 `KokoroError::Validation(format!("Denied by hook: {}", reason))`
- 成功/失败后照常执行并返回现有结果

#### 备选方案：显式不覆盖
如果此处接 deny 会显著扩 scope：
- 保持代码不动
- 但必须在计划文档或实现总结中明确 `execute_action` 暂时绕过 deny

**Step 3: 跑验证**

Run:
- `cd src-tauri && cargo check`

Expected:
- PASS

**Step 4: Commit**

```bash
git add src-tauri/src/commands/actions.rs
git commit -m "feat: align direct action execution with hook deny"
```

如果本任务最终决定“暂不覆盖”，则这一步改成更新计划/说明文档的小提交。

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/hooks/types.rs` 已为 `HookOutcome` 增加 `Deny { reason: String }`
- `src-tauri/src/hooks/runtime.rs` 已增加仅供 action 使用的 `emit_action_gate(...)`，并保持 handler error 为 fail-open
- `src-tauri/src/actions/executor.rs` 已在 `BeforeActionInvoke` 消费 deny；deny 时统一映射为 `Denied by hook: <reason>`，并继续发送 `AfterActionInvoke`
- `src-tauri/src/commands/actions.rs` 直调 `execute_action(...)` 已做最小对齐：同样消费 deny，并继续发送 `AfterActionInvoke`
- `src-tauri/src/hooks/tests.rs`、`src-tauri/src/actions/executor.rs`、`src-tauri/src/commands/actions.rs` 已补最小测试，覆盖 deny 文本、短路语义与非 action fail-open 边界

**未完成：**
- chat 生命周期仍未支持 deny，继续保持 fail-open
- mod lifecycle 仍未支持 deny，继续保持 fail-open
- `modify payload`、全局 `fail-closed`、前端审批 UI、manifest 声明式 hook 注册、deny trace 持久化仍未实现
- 本机 Windows 运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`；因此本地验证以 `cargo check`、`cargo clippy -- -D warnings` 与 `cargo test --no-run` 为主

### Task 4: 收尾验证与文档对齐

**Files:**
- Modify: `docs/plans/2026-04-03-p2-1-hook-deny-on-action.md`
- Verify: `docs/plans/2026-04-02-p2-hook-runtime-centralized.md`

**Step 1: 补一段“本阶段已完成 / 未完成”说明**

文档里明确：
- 本阶段只让 `BeforeActionInvoke` 支持 deny
- chat / mod 仍未支持 deny
- `modify payload` / `fail-closed` / manifest / 前端审批都仍是后续工作

**Step 2: 跑整体验证**

Run:
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test hooks::tests -- --nocapture`
- `cd src-tauri && cargo test actions:: -- --nocapture`

如本机仍遇到已知的 Windows 测试二进制启动问题，则至少保证：
- `cargo check` PASS
- `cargo clippy -- -D warnings` PASS
- CI 中对应 targeted tests 通过

**Step 3: Commit**

```bash
git add docs/plans/2026-04-03-p2-1-hook-deny-on-action.md
git commit -m "docs: finalize action deny rollout plan"
```

---

## Guardrails

### DRY
- deny 语义只能在 `hooks/types.rs` 的 `HookOutcome` 中定义一次
- executor 与直调路径不要各自发明不同的 deny 文本格式
- 若需要 helper，优先放在 `actions/executor.rs` 内部最小复用，不新建泛化工具模块

### YAGNI
- 不要在本阶段实现 `ReplacePayload`
- 不要给 chat / mod 也顺手开 deny
- 不要新增数据库持久化 deny trace
- 不要做前端审批弹窗

### TDD
- 先让 hooks deny 测试失败
- 再让 executor deny 测试失败
- 再写最小实现
- 没有失败信号不要直接改生产逻辑

### Frequent Commits
- HookOutcome / runtime gate 一次 commit
- executor deny 一次 commit
- 直调路径对齐一次 commit
- 文档收尾一次 commit

---

## Verification

### 必跑命令
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test hooks::tests -- --nocapture`
- `cd src-tauri && cargo test actions:: -- --nocapture`

### 成功标准
- `HookOutcome` 支持 `Deny { reason }`
- deny 只对 `BeforeActionInvoke` 生效
- deny 能真正阻止 action 执行
- deny 后仍会发 `AfterActionInvoke`
- chat / mod 行为不因本阶段改变
- 文档明确写清未完成范围

---

## Suggested Execution Order

1. Task 1：扩展 HookOutcome 与 action gate 运行时入口
2. Task 2：共享 tool executor 消费 deny
3. Task 3：评估并最小覆盖 execute_action 直调路径
4. Task 4：收尾验证与文档对齐

---

## Notes For Future Claude

P2.1 做完后，下一个顺序应保持：
1. 先把 deny 的消费面稳定在 action
2. 再做 `BeforeLlmRequest` 的 modify payload
3. 再引入风险标签 / 权限级别
4. 最后才讨论 fail-closed 与前端审批 UI

不要在 action deny 还没稳定前，把 deny 扩到 chat 或 mod。