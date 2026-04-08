# UTC 稳定性收口（无审计数据库）Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在不引入审计数据库和复杂前端审计 UI 的前提下，完成权限/审批语义收口，移除执行链对字符串前缀的业务依赖，并补齐回归测试。

**Architecture:** 以 `PermissionDecision` 为唯一判定真源，在后端统一 deny 分类与审批状态流；chat/telegram/direct action 共用结构化语义，前端仅做最小兼容消费。通过 TDD 分步收口，每步单测先红后绿并小步提交。

**Tech Stack:** Rust (Tauri v2, tokio, serde_json), TypeScript (React + Vitest)

---

### Task 1: 为权限拒绝结果建立结构化 deny kind（后端真源）

**Files:**
- Modify: `src-tauri/src/actions/permission.rs`
- Test: `src-tauri/src/actions/permission.rs` (existing `mod tests`)

**Step 1: Write the failing test**

在 `permission.rs` 的测试模块新增：

```rust
#[test]
fn decision_deny_kind_maps_each_denial_variant() {
    assert_eq!(deny_kind(&PermissionDecision::DenyPolicy { reason: "x".into() }), Some("policy_denied"));
    assert_eq!(deny_kind(&PermissionDecision::DenyPendingApproval { reason: "x".into() }), Some("pending_approval"));
    assert_eq!(deny_kind(&PermissionDecision::DenyFailClosed { reason: "x".into() }), Some("fail_closed"));
    assert_eq!(deny_kind(&PermissionDecision::Allow), None);
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test actions::permission::tests::decision_deny_kind_maps_each_denial_variant -- --nocapture`

Expected: FAIL with `cannot find function 'deny_kind'`.

**Step 3: Write minimal implementation**

在 `permission.rs` 新增：

```rust
pub fn deny_kind(decision: &PermissionDecision) -> Option<&'static str> {
    match decision {
        PermissionDecision::Allow => None,
        PermissionDecision::DenyPolicy { .. } => Some("policy_denied"),
        PermissionDecision::DenyPendingApproval { .. } => Some("pending_approval"),
        PermissionDecision::DenyFailClosed { .. } => Some("fail_closed"),
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test actions::permission::tests::decision_deny_kind_maps_each_denial_variant -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src-tauri/src/actions/permission.rs
git commit -m "refactor(permission): add structured deny kind mapping"
```

---

### Task 2: chat trace 改为优先结构化决策，去掉前缀驱动判定

**Files:**
- Modify: `src-tauri/src/commands/chat.rs` (deny kind 计算、tool trace payload)
- Test: `src-tauri/src/commands/chat.rs` (existing tests around `deny_kind_for_tool_error`/trace payload)

**Step 1: Write the failing test**

新增测试，验证 deny kind 不依赖错误字符串前缀：

```rust
#[test]
fn tool_error_payload_prefers_permission_decision_over_error_prefix() {
    let outcome = sample_tool_outcome_with_decision(
        crate::actions::PermissionDecision::DenyFailClosed { reason: "boom".into() },
        Err("custom message without prefix".to_string()),
    );
    let payload = tool_error_payload(&outcome, "turn-1", "custom message without prefix");
    assert_eq!(payload.get("deny_kind").and_then(|v| v.as_str()), Some("fail_closed"));
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::chat::tests::tool_error_payload_prefers_permission_decision_over_error_prefix -- --nocapture`

Expected: FAIL (当前逻辑基于 `starts_with`，会得到 `execution_error`)。

**Step 3: Write minimal implementation**

在 `chat.rs`：

1) 新增 helper（优先读取 `outcome.permission_decision`）

```rust
fn deny_kind_for_outcome(
    outcome: &crate::actions::ToolExecutionOutcome,
    error: &str,
) -> &'static str {
    if let Some(decision) = outcome.permission_decision.as_ref() {
        if let Some(kind) = crate::actions::permission::deny_kind(decision) {
            return kind;
        }
    }
    deny_kind_for_tool_error(error)
}
```

2) `tool_error_payload(...)` 使用 `deny_kind_for_outcome(...)`。

3) 保留 `deny_kind_for_tool_error(...)` 仅做历史 fallback（兼容旧数据）。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::tool_error_payload_prefers_permission_decision_over_error_prefix -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_approval_trace_payload_includes_request_id_and_requested_status -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "refactor(chat): use structured permission decision for deny kind"
```

---

### Task 3: 强化审批状态机幂等与非法迁移保护

**Files:**
- Modify: `src-tauri/src/commands/chat.rs` (`PendingToolApprovalState` + resolve logic)
- Test: `src-tauri/src/commands/chat.rs` (approval state tests)

**Step 1: Write the failing test**

新增测试：同一个 request 第二次 approve/reject 不得二次生效。

```rust
#[tokio::test]
async fn pending_tool_approval_state_rejects_second_resolution() {
    let state = PendingToolApprovalState::new();
    let request_id = state.register("turn-1".into(), "tool-1".into(), "tool".into(), HashMap::new()).await;

    let first = approve_tool_approval_inner(&state, request_id.clone()).await;
    assert!(first.is_ok());

    let second = reject_tool_approval_inner(&state, request_id.clone(), Some("late reject".into())).await;
    assert!(second.is_err());
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::chat::tests::pending_tool_approval_state_rejects_second_resolution -- --nocapture`

Expected: FAIL（若当前允许重复 resolve 或返回不一致）。

**Step 3: Write minimal implementation**

在 `PendingToolApprovalState` 的 resolve 路径增加：
- 已处理请求直接返回一致错误（如 `approval request already resolved`）。
- 确保 receiver 只消费一次，状态不可逆。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::pending_tool_approval_state_rejects_second_resolution -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_tool_approval_state_generates_request_id_and_resolves_approve -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_tool_approval_state_resolves_reject_and_unknown_id_errors -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "fix(chat): make pending approval resolution idempotent"
```

---

### Task 4: direct action 与共享权限语义对齐（无前缀业务分支）

**Files:**
- Modify: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/commands/actions.rs` (existing tests around pending/fail-closed/policy)

**Step 1: Write the failing test**

新增测试：`execute_action` 在 pending approval 场景返回时，错误语义来自 `PermissionDecision`，而不是字符串前缀判断分支。

```rust
#[tokio::test]
async fn execute_action_denial_uses_permission_decision_reason() {
    // 构造 blocked write 或 elevated under safe ceiling 的 action/settings
    // 断言返回 KokoroError::Validation(reason) 与 evaluate_permission_decision 一致
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::actions::tests::execute_action_denial_uses_permission_decision_reason -- --nocapture`

Expected: FAIL（若当前分支存在文本耦合或 reason 漂移）。

**Step 3: Write minimal implementation**

在 `execute_action(...)` 权限拒绝分支中：
- 只从 `PermissionDecision` 提取 reason。
- 删除/避免新增任何基于错误文本前缀的判断。
- 保持 hook 与错误回传路径不变（最小改动）。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::actions::tests::execute_action_denial_uses_permission_decision_reason -- --nocapture`
- `cd src-tauri && cargo test commands::actions -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/actions.rs
git commit -m "refactor(actions): align direct execute denial with permission decisions"
```

---

### Task 5: 前端最小兼容收口（不新增审计 UI）

**Files:**
- Modify: `src/ui/widgets/chat-history.ts`
- Test: `src/ui/widgets/chat-history.test.ts`

**Step 1: Write the failing test**

新增测试：当 metadata 已有 `deny_kind` 时，解析优先使用它；仅在缺失时才 fallback 到错误前缀。

```ts
it("优先使用后端 deny_kind，仅在缺失时回退前缀推断", () => {
  // 构造 tool_result metadata: deny_kind="fail_closed" + error 无前缀
  // 断言 parse 后 tool.denyKind === "fail_closed"
});
```

**Step 2: Run test to verify it fails**

Run: `npx vitest run src/ui/widgets/chat-history.test.ts`

Expected: FAIL（当前若始终依赖 error 前缀推断）。

**Step 3: Write minimal implementation**

在 `chat-history.ts` 工具事件解析里：
- deny kind 读取顺序改为：`metadata.deny_kind` -> `error` 前缀推断。
- 保持现有 UI 结构不变，不新增字段面板。

**Step 4: Run test to verify it passes**

Run: `npx vitest run src/ui/widgets/chat-history.test.ts`

Expected: PASS。

**Step 5: Commit**

```bash
git add src/ui/widgets/chat-history.ts src/ui/widgets/chat-history.test.ts
git commit -m "fix(chat-history): prefer structured deny kind with fallback parsing"
```

---

### Task 6: 全量回归与收口验证

**Files:**
- Modify: `docs/plans/2026-04-07-utc-stabilization-design.md`（补充“实现完成状态”小节）
- Optional Modify: `docs/_wip_p0_unified_tool_contract_progress.md`（同步收口进度）

**Step 1: Write the failing test/checklist item**

先建立本轮验证清单（写入设计文档末尾）：

```md
- [ ] chat deny kind 结构化优先
- [ ] pending approval 幂等
- [ ] direct action 权限拒绝语义一致
- [ ] 前端 deny kind 最小兼容
```

**Step 2: Run verification to identify failures (if any)**

Run:
- `npx tsc --noEmit`
- `npx vitest run src/ui/widgets/chat-history.test.ts src/ui/widgets/chat-history.test.ts`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

Expected: 全部通过；若失败，定位后最小修复再重跑。

**Step 3: Write minimal implementation/fixes**

只修复导致上述检查失败的最小代码，不做额外重构。

**Step 4: Run verification again**

重复 Step 2，直到全部 PASS。

**Step 5: Commit**

```bash
git add docs/plans/2026-04-07-utc-stabilization-design.md docs/_wip_p0_unified_tool_contract_progress.md src-tauri/src/actions/permission.rs src-tauri/src/commands/chat.rs src-tauri/src/commands/actions.rs src/ui/widgets/chat-history.ts src/ui/widgets/chat-history.test.ts
git commit -m "test: close utc stabilization with structured permission semantics"
```

---

## 执行注意事项

- 严格遵守 DRY / YAGNI：本轮不引入审计数据库，不新增复杂前端页面。
- 每个 Task 完成后立即提交，避免大批量混合改动。
- 若出现 Windows 环境 `STATUS_ENTRYPOINT_NOT_FOUND`，优先使用 `cargo check --tests` 与 `--no-run` 保证编译链稳定。

## 完成判据

- 后端执行链业务语义不再依赖错误字符串前缀。
- 审批状态机具备幂等保护。
- 三入口（chat/telegram/direct）权限拒绝语义一致。
- 前端仅做最小兼容，权限提醒体验不回归。
- 全部验证命令通过。
