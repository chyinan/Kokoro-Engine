# UTC 稳定性优先最小收口 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在不扩展审计数据库和复杂前端 UI 的前提下，完成 UTC/P1/P2 的语义一致性收口，先降回归风险，再补最小能力模型闭环。

**Architecture:** 后端以 `PermissionDecision` 作为拒绝语义唯一真源，chat/actions/hook 路径统一消费结构化判定；审批状态机强化幂等，避免二次 resolve 产生竞态漂移。前端只做最小兼容消费（`metadata.deny_kind` 优先），保留历史前缀 fallback。MOD 能力模型仅做最小声明式扩展并保持旧 `permissions` 兼容。

**Tech Stack:** Rust (Tauri v2, tokio, serde), TypeScript (React, Vitest), Cargo, npm

---

### Task 1: 权限拒绝语义统一映射（PermissionDecision → deny_kind）

**Files:**
- Modify: `src-tauri/src/actions/permission.rs`
- Test: `src-tauri/src/actions/permission.rs`（`mod tests`）

**Step 1: Write the failing test**

在 `permission.rs` 测试模块新增：

```rust
#[test]
fn decision_deny_kind_maps_each_denial_variant() {
    use crate::actions::PermissionDecision;

    assert_eq!(deny_kind(&PermissionDecision::DenyPolicy { reason: "x".into() }), Some("policy_denied"));
    assert_eq!(deny_kind(&PermissionDecision::DenyPendingApproval { reason: "x".into() }), Some("pending_approval"));
    assert_eq!(deny_kind(&PermissionDecision::DenyFailClosed { reason: "x".into() }), Some("fail_closed"));
    assert_eq!(deny_kind(&PermissionDecision::Allow), None);
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test actions::permission::tests::decision_deny_kind_maps_each_denial_variant -- --nocapture`
Expected: FAIL，提示 `deny_kind` 不存在或返回不匹配。

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
Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/actions/permission.rs
git commit -m "refactor(permission): add structured deny kind mapping"
```

---

### Task 2: chat trace 语义改为结构化优先

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Test: `src-tauri/src/commands/chat.rs`（现有 tests 模块）

**Step 1: Write the failing test**

新增测试：

```rust
#[test]
fn tool_error_payload_prefers_permission_decision_over_error_prefix() {
    let outcome = sample_tool_outcome_with_decision(
        crate::actions::PermissionDecision::DenyFailClosed { reason: "blocked".into() },
        Err("custom message without prefix".to_string()),
    );

    let payload = tool_error_payload(&outcome, "turn-1", "custom message without prefix");
    assert_eq!(payload.get("deny_kind").and_then(|v| v.as_str()), Some("fail_closed"));
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::chat::tests::tool_error_payload_prefers_permission_decision_over_error_prefix -- --nocapture`
Expected: FAIL（当前可能回退到 `execution_error`）。

**Step 3: Write minimal implementation**

在 `chat.rs` 新增 helper 并接入：

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

将 `tool_error_payload(...)` 中 deny kind 计算替换为 `deny_kind_for_outcome(...)`。
保留 `deny_kind_for_tool_error(...)` 仅作历史 fallback。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::tool_error_payload_prefers_permission_decision_over_error_prefix -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_approval_trace_payload_includes_request_id_and_requested_status -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "refactor(chat): prefer structured permission decisions in deny trace"
```

---

### Task 3: pending approval 状态机幂等保护

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Test: `src-tauri/src/commands/chat.rs`（approval state tests）

**Step 1: Write the failing test**

新增测试：

```rust
#[tokio::test]
async fn pending_tool_approval_state_rejects_second_resolution() {
    let state = PendingToolApprovalState::new();
    let request_id = state
        .register("turn-1".into(), "tool-1".into(), "tool".into(), std::collections::HashMap::new())
        .await;

    let first = approve_tool_approval_inner(&state, request_id.clone()).await;
    assert!(first.is_ok());

    let second = reject_tool_approval_inner(&state, request_id, Some("late reject".into())).await;
    assert!(second.is_err());
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::chat::tests::pending_tool_approval_state_rejects_second_resolution -- --nocapture`
Expected: FAIL（若当前允许重复 resolve）。

**Step 3: Write minimal implementation**

在 resolve 路径加入“已解决即拒绝”保护：
- 已 resolve 的 request 返回统一错误（如 `approval request already resolved`）
- receiver 只消费一次，状态不可逆

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::pending_tool_approval_state_rejects_second_resolution -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_tool_approval_state_generates_request_id_and_resolves_approve -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::test_pending_tool_approval_state_resolves_reject_and_unknown_id_errors -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "fix(chat): enforce idempotent pending approval resolution"
```

---

### Task 4: direct action 拒绝语义与 PermissionDecision 对齐

**Files:**
- Modify: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/commands/actions.rs`

**Step 1: Write the failing test**

新增测试：

```rust
#[tokio::test]
async fn execute_action_denial_uses_permission_decision_reason() {
    // 构造一个会触发 Deny* 的动作与设置
    // 调用 execute_action
    // 断言返回的 Validation 错误消息来自 PermissionDecision 的 reason
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::actions::tests::execute_action_denial_uses_permission_decision_reason -- --nocapture`
Expected: FAIL（若当前仍有字符串耦合路径）。

**Step 3: Write minimal implementation**

在 `execute_action(...)` 拒绝分支：
- 仅从 `PermissionDecision` 提取 reason
- 不新增基于错误前缀的业务分支
- 保持 hook 与现有错误返回模型不变

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::actions::tests::execute_action_denial_uses_permission_decision_reason -- --nocapture`
- `cd src-tauri && cargo test commands::actions -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/actions.rs
git commit -m "refactor(actions): align denial reason with permission decisions"
```

---

### Task 5: 前端最小兼容（deny_kind 优先结构化字段）

**Files:**
- Modify: `src/ui/widgets/chat-history.ts`
- Test: `src/ui/widgets/chat-history.test.ts`

**Step 1: Write the failing test**

新增两个测试：

```ts
it("优先使用 metadata.deny_kind", () => {
  // metadata.deny_kind = "fail_closed"; error = "random"
  // 断言 parsed denyKind === "fail_closed"
});

it("metadata 缺失时回退 error 前缀推断", () => {
  // 无 metadata.deny_kind，仅有旧前缀错误字符串
  // 断言仍可推断 denyKind
});
```

**Step 2: Run test to verify it fails**

Run: `npx vitest run src/ui/widgets/chat-history.test.ts`
Expected: FAIL（若当前始终依赖前缀）。

**Step 3: Write minimal implementation**

在 `chat-history.ts` deny kind 解析逻辑改为：
1. `metadata.deny_kind`
2. error 前缀 fallback

不改 UI 结构，不新增组件。

**Step 4: Run test to verify it passes**

Run: `npx vitest run src/ui/widgets/chat-history.test.ts`
Expected: PASS。

**Step 5: Commit**

```bash
git add src/ui/widgets/chat-history.ts src/ui/widgets/chat-history.test.ts
git commit -m "fix(chat-history): prioritize structured deny_kind with legacy fallback"
```

---

### Task 6: Hook modify strict/permissive 最小策略接线

**Files:**
- Modify: `src-tauri/src/hooks/types.rs`
- Modify: `src-tauri/src/hooks/runtime.rs`
- Modify: `src-tauri/src/commands/chat.rs`
- Modify: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/hooks/tests.rs`

**Step 1: Write the failing test**

新增测试：

```rust
#[tokio::test]
async fn before_llm_modify_strict_mode_returns_err_when_handler_fails() {
    // 注册会返回 Err 的 modify handler
    // strict 模式调用应返回 Err
}

#[tokio::test]
async fn before_llm_modify_permissive_mode_keeps_best_effort_behavior() {
    // 同样 handler
    // permissive 模式调用应 Ok 并继续
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test hooks::tests::before_llm_modify_strict_mode_returns_err_when_handler_fails -- --nocapture`
Expected: FAIL（当前无 strict 策略）。

**Step 3: Write minimal implementation**

在 `hooks/types.rs` 增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookModifyPolicy {
    Permissive,
    Strict,
}
```

更新 runtime modify API：

```rust
pub async fn emit_before_llm_request_modify(
    &self,
    payload: &mut BeforeLlmRequestPayload,
    policy: HookModifyPolicy,
) -> Result<(), String>
```

```rust
pub async fn emit_before_action_args_modify(
    &self,
    payload: &mut BeforeActionArgsPayload,
    policy: HookModifyPolicy,
) -> Result<(), String>
```

并在命令层接线：
- chat before-llm-request 使用 `Strict`
- direct action args modify 使用 `Strict`

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test hooks::tests::before_llm_modify_strict_mode_returns_err_when_handler_fails -- --nocapture`
- `cd src-tauri && cargo test hooks::tests::before_llm_modify_permissive_mode_keeps_best_effort_behavior -- --nocapture`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/hooks/types.rs src-tauri/src/hooks/runtime.rs src-tauri/src/commands/chat.rs src-tauri/src/commands/actions.rs src-tauri/src/hooks/tests.rs
git commit -m "refactor(hooks): add strict modify policy for core execution paths"
```

---

### Task 7: MOD 声明式 capabilities 最小模型

**Files:**
- Modify: `src-tauri/src/mods/manifest.rs`
- Modify: `src-tauri/src/mods/manager.rs`
- Test: `src-tauri/src/mods/manifest.rs`（tests）

**Step 1: Write the failing test**

新增测试：

```rust
#[test]
fn parse_capabilities_manifest_and_keep_permissions_backward_compatible() {
    let json = r#"{
      \"id\":\"demo\",
      \"name\":\"Demo\",
      \"version\":\"0.1.0\",
      \"description\":\"demo\",
      \"capabilities\":[
        {\"name\":\"tts.speak\",\"risk\":\"write\",\"requires_confirmation\":false},
        {\"name\":\"system.info\",\"risk\":\"read\",\"requires_confirmation\":false}
      ],
      \"permissions\":[\"tts\"]
    }"#;

    let manifest: ModManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.capabilities.len(), 2);
    assert_eq!(manifest.permissions, vec!["tts"]);
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test mods::manifest::tests::parse_capabilities_manifest_and_keep_permissions_backward_compatible -- --nocapture`
Expected: FAIL（capabilities 尚未定义）。

**Step 3: Write minimal implementation**

在 `manifest.rs` 增加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModCapability {
    pub name: String,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub requires_confirmation: bool,
}
```

并在 `ModManifest` 增加：

```rust
#[serde(default)]
pub capabilities: Vec<ModCapability>,
```

在 `manager.rs` 加载时做轻量校验：
- capability `name.trim().is_empty()` 时报错
- 其余字段透传，不做策略扩展

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test mods::manifest::tests::parse_capabilities_manifest_and_keep_permissions_backward_compatible -- --nocapture`
- `cd src-tauri && cargo test mods::manifest -- --nocapture`
- `cd src-tauri && cargo check`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/mods/manifest.rs src-tauri/src/mods/manager.rs
git commit -m "feat(mods): add minimal declarative capabilities support"
```

---

### Task 8: 全量验证与文档收口

**Files:**
- Modify: `docs/_wip_p0_unified_tool_contract_progress.md`
- Modify: `docs/plans/2026-04-07-utc-stability-minimal-closure-design.md`

**Step 1: Write the checklist in docs**

在文档中补收口清单：

```md
- [ ] deny kind 结构化优先已落地
- [ ] pending approval 幂等已落地
- [ ] direct action 语义与 PermissionDecision 一致
- [ ] 前端 deny_kind 最小兼容已落地
- [ ] hook strict/permissive 已接线
- [ ] mod capabilities 最小模型已落地
```

**Step 2: Run verification batch**

Run:
- `npx tsc --noEmit`
- `npx vitest run src/ui/widgets/chat-history.test.ts`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`
- `cd src-tauri && cargo test --lib --no-run`

Expected: 全部通过。

**Step 3: Fix minimally if needed**

仅修复导致上述验证失败的最小代码，不做额外重构。

**Step 4: Re-run verification**

重复 Step 2，直到全部 PASS。

**Step 5: Commit**

```bash
git add docs/_wip_p0_unified_tool_contract_progress.md docs/plans/2026-04-07-utc-stability-minimal-closure-design.md
git commit -m "docs: close utc stability-first minimal closure checklist"
```

---

## 执行注意事项

- 使用 @superpowers:test-driven-development 执行每个任务的先红后绿。
- 使用 @superpowers:verification-before-completion 在每个任务与总收口前验证。
- 保持 YAGNI：不新增审计 DB、不新增审批 UI、不做企业化扩面。
- 本轮按用户要求直接在当前工作区执行（不使用 worktree）。
- Windows 若运行期测试不稳定，优先 `cargo check --tests` 和 `--no-run` 保证编译链收口。
