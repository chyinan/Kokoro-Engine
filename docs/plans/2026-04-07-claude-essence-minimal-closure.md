# Claude Code 精髓最小收口 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 以最小改动完成三项高价值收口：Hook modify 可控策略、MOD 声明式能力最小模型、chat 结构化 deny 语义彻底优先，避免企业化扩展。

**Architecture:** 保持现有 UTC/P1/P2 主链路不扩面，只在现有中心点补齐关键边界。后端继续以结构化语义为真源，前端不新增复杂审批/审计界面。通过 TDD 小步推进，每个任务独立提交，确保可回滚。

**Tech Stack:** Rust (Tauri v2, tokio, serde), TypeScript (现有最小兼容), SQLite migration（仅在必要时）

---

### Task 1: Hook modify 失败策略最小化可控（strict / permissive）

**Files:**
- Modify: `src-tauri/src/hooks/runtime.rs`
- Modify: `src-tauri/src/hooks/types.rs`
- Modify: `src-tauri/src/commands/chat.rs`
- Modify: `src-tauri/src/commands/actions.rs`
- Test: `src-tauri/src/hooks/tests.rs`

**Step 1: Write the failing test**

在 `src-tauri/src/hooks/tests.rs` 增加两组测试：

```rust
#[tokio::test]
async fn before_llm_modify_strict_mode_returns_err_when_handler_fails() {
    // 注册一个会在 modify_before_llm_request 返回 Err 的测试 handler
    // 调用 strict 模式 API
    // 断言返回 Err("...")
}

#[tokio::test]
async fn before_llm_modify_permissive_mode_keeps_current_best_effort_behavior() {
    // 同样的失败 handler
    // 调用 permissive 模式 API
    // 断言返回 Ok(()) 且 payload 未崩溃
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test hooks::tests::before_llm_modify_strict_mode_returns_err_when_handler_fails -- --nocapture`
Expected: FAIL（当前 modify API 仅日志吞错，无 strict 分支）。

**Step 3: Write minimal implementation**

在 `src-tauri/src/hooks/types.rs` 增加策略枚举：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookModifyPolicy {
    Permissive,
    Strict,
}
```

在 `src-tauri/src/hooks/runtime.rs` 将 modify API 改为显式策略：

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

行为：
- `Permissive`：保持当前逻辑（记录错误后继续）
- `Strict`：首个 handler 返回 Err 时直接返回 Err

命令层最小接线：
- `chat.rs` 的 before-llm-request 使用 `Strict`
- `actions.rs` 的 direct action args modify 使用 `Strict`

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test hooks::tests::before_llm_modify_strict_mode_returns_err_when_handler_fails -- --nocapture`
- `cd src-tauri && cargo test hooks::tests::before_llm_modify_permissive_mode_keeps_current_best_effort_behavior -- --nocapture`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/hooks/types.rs src-tauri/src/hooks/runtime.rs src-tauri/src/commands/chat.rs src-tauri/src/commands/actions.rs src-tauri/src/hooks/tests.rs
git commit -m "refactor(hooks): add strict modify policy for core pipelines"
```

---

### Task 2: MOD manifest 升级为声明式能力（最小模型，不做平台化）

**Files:**
- Modify: `src-tauri/src/mods/manifest.rs`
- Modify: `src-tauri/src/mods/manager.rs`
- Test: `src-tauri/src/mods/manifest.rs` (existing tests module)

**Step 1: Write the failing test**

在 `manifest.rs` 测试模块增加：

```rust
#[test]
fn parse_capabilities_manifest_and_keep_permissions_backward_compatible() {
    let json = r#"{
      "id":"demo",
      "name":"Demo",
      "version":"0.1.0",
      "description":"demo",
      "capabilities":[
        {"name":"tts.speak","risk":"write","requires_confirmation":false},
        {"name":"system.info","risk":"read","requires_confirmation":false}
      ],
      "permissions":["tts"]
    }"#;

    let manifest: ModManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.capabilities.len(), 2);
    assert_eq!(manifest.permissions, vec!["tts"]);
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test mods::manifest::tests::parse_capabilities_manifest_and_keep_permissions_backward_compatible -- --nocapture`
Expected: FAIL（当前没有 capabilities 字段模型）。

**Step 3: Write minimal implementation**

在 `src-tauri/src/mods/manifest.rs` 增加最小结构：

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

在 `manager.rs` 加最小校验（仅在加载时做轻量合法性检查）：
- capability `name` 非空
- 其余字段仅保留原样，不引入复杂策略引擎

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test mods::manifest::tests::parse_capabilities_manifest_and_keep_permissions_backward_compatible -- --nocapture`
- `cd src-tauri && cargo test mods::manifest -- --nocapture`
- `cd src-tauri && cargo check`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/mods/manifest.rs src-tauri/src/mods/manager.rs
git commit -m "feat(mods): add minimal declarative capabilities in manifest"
```

---

### Task 3: chat deny kind 完全结构化优先，前缀仅历史回放 fallback

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Modify: `src/ui/widgets/chat-history.ts`
- Test: `src-tauri/src/commands/chat.rs` (existing tests module)
- Test: `src/ui/widgets/chat-history.test.ts`

**Step 1: Write the failing test**

后端新增测试（chat.rs）：

```rust
#[test]
fn tool_trace_error_deny_kind_prefers_outcome_decision_when_available() {
    // 构造 outcome.permission_decision = DenyPolicy
    // error 文本给无前缀字符串
    // 断言 payload deny_kind == "policy_denied"
}
```

前端新增测试（chat-history.test.ts）：

```ts
it("metadata.deny_kind 优先于 error 前缀推断", () => {
  // metadata.deny_kind = "fail_closed"; error = "random"
  // 断言解析结果 denyKind === "fail_closed"
});
```

**Step 2: Run test to verify it fails**

Run:
- `cd src-tauri && cargo test commands::chat::tests::tool_trace_error_deny_kind_prefers_outcome_decision_when_available -- --nocapture`
- `npx vitest run src/ui/widgets/chat-history.test.ts`

Expected: 至少一项 FAIL（表示仍有前缀耦合路径）。

**Step 3: Write minimal implementation**

后端 `chat.rs`：
- 所有新生成 tool trace payload 的 `deny_kind` 统一走 `deny_kind_for_outcome(...)`
- `deny_kind_for_tool_error(...)` 仅保留给历史消息回放/兼容解析路径

前端 `chat-history.ts`：
- deny kind 解析顺序固定为：`metadata.deny_kind` -> `error` 前缀 fallback
- 不新增 UI 元素，不改变现有展示结构

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat --no-run`
- `npx vitest run src/ui/widgets/chat-history.test.ts`
- `npx tsc --noEmit`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs src/ui/widgets/chat-history.ts src/ui/widgets/chat-history.test.ts
git commit -m "refactor(chat): prioritize structured deny semantics over string prefixes"
```

---

### Task 4: 总体验证与文档收口（仅更新计划/进度，不扩面）

**Files:**
- Modify: `docs/_wip_p0_unified_tool_contract_progress.md`
- Modify: `docs/plans/2026-04-07-utc-stabilization-design.md`

**Step 1: Write the failing checklist item**

在文档增加收口勾选项：

```md
- [ ] Hook modify strict/permissive 策略已落地
- [ ] MOD 声明式 capabilities 最小模型已落地
- [ ] chat deny kind 结构化优先已收口
```

**Step 2: Run verification to identify failures**

Run:
- `npx tsc --noEmit`
- `npx vitest run src/ui/widgets/chat-history.test.ts`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test --lib --no-run`

Expected: 全部通过。

**Step 3: Write minimal fixes if needed**

仅修复导致校验失败的最小代码，不做额外重构。

**Step 4: Run verification again**

重复 Step 2，直到全部 PASS。

**Step 5: Commit**

```bash
git add docs/_wip_p0_unified_tool_contract_progress.md docs/plans/2026-04-07-utc-stabilization-design.md
git commit -m "docs: close minimal claude-essence stabilization checklist"
```

---

## 执行注意事项

- 只做“精髓收口”，不新增审批前端、不新增审计数据库、不做企业化能力扩展。
- 坚持 DRY / YAGNI / TDD：每个任务先红后绿，任务完成即提交。
- 若 Windows 环境触发运行期测试问题，优先使用 `cargo check --tests` 与 `--no-run` 保证编译级稳定。

## 完成判据

- Hook modify 失败行为可配置，核心链路可 fail-closed。
- MOD 从纯权限字符串升级为最小声明式能力模型并保留兼容。
- chat/tool trace 语义以结构化字段为真源，字符串前缀仅做历史回放 fallback。
- 全量验证通过，且未引入新 UI 复杂度。