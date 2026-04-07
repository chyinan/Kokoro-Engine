# Turn 级可中止执行流与 Hook 生命周期补齐 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 chat 主执行链增加 turn 级可中止能力，并补齐 `BeforeTtsPlay` / `AfterTtsPlay` / `OnEmotionChanged` 三个关键 Hook 生命周期事件。

**Architecture:** 在后端命令层引入最小 `TurnCancellationState`（内存态、按 turn_id 幂等管理），在 chat 主链路关键检查点提前中止后续执行。Hook 体系仅做增量补齐：扩展事件枚举、复用现有 runtime 分发，不引入额外平台化策略或前端复杂 UI。

**Tech Stack:** Rust (Tauri v2, tokio, serde), TypeScript (现有前端最小兼容), Vitest, cargo test/check

---

### Task 1: 建立 TurnCancellationState 最小状态模型（幂等取消）

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Test: `src-tauri/src/commands/chat.rs` (existing `mod tests`)

**Step 1: Write the failing test**

在 `chat.rs` 测试模块新增：

```rust
#[tokio::test]
async fn turn_cancellation_state_register_cancel_and_idempotent() {
    let state = TurnCancellationState::new();

    state.register_turn("turn-1").await;
    assert!(!state.is_cancelled("turn-1").await);

    assert!(state.cancel_turn("turn-1", Some("user".into())).await.is_ok());
    assert!(state.is_cancelled("turn-1").await);

    // 幂等：重复取消不应报错
    assert!(state.cancel_turn("turn-1", Some("again".into())).await.is_ok());
    assert!(state.is_cancelled("turn-1").await);
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test commands::chat::tests::turn_cancellation_state_register_cancel_and_idempotent -- --nocapture`

Expected: FAIL with `cannot find type 'TurnCancellationState'`.

**Step 3: Write minimal implementation**

在 `chat.rs` 增加最小状态结构（仅内存态）：

```rust
#[derive(Default)]
struct TurnCancellationState {
    cancelled: tokio::sync::RwLock<std::collections::HashMap<String, Option<String>>>,
}

impl TurnCancellationState {
    fn new() -> Self { Self::default() }

    async fn register_turn(&self, turn_id: &str) {
        let mut map = self.cancelled.write().await;
        map.entry(turn_id.to_string()).or_insert(None);
    }

    async fn cancel_turn(&self, turn_id: &str, reason: Option<String>) -> Result<(), String> {
        let mut map = self.cancelled.write().await;
        if let Some(entry) = map.get_mut(turn_id) {
            if entry.is_none() {
                *entry = reason;
            }
            return Ok(());
        }
        Err(format!("unknown turn_id: {}", turn_id))
    }

    async fn is_cancelled(&self, turn_id: &str) -> bool {
        self.cancelled
            .read()
            .await
            .get(turn_id)
            .map(|v| v.is_some())
            .unwrap_or(false)
    }

    async fn clear_turn(&self, turn_id: &str) {
        self.cancelled.write().await.remove(turn_id);
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test commands::chat::tests::turn_cancellation_state_register_cancel_and_idempotent -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "feat(chat): add turn cancellation state with idempotent cancel"
```

---

### Task 2: chat 主链路接入取消检查点（LLM 前 / tool 前后 / delta 前）

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Test: `src-tauri/src/commands/chat.rs` (existing tests module)

**Step 1: Write the failing test**

新增测试，验证取消后不再继续工具调用：

```rust
#[tokio::test]
async fn cancelled_turn_stops_before_tool_execution() {
    // 构造最小 chat 执行上下文（复用现有测试 helper）
    // register turn -> cancel turn
    // 触发 tool 执行入口
    // 断言：返回取消语义，且 tool 执行计数仍为 0
}
```

新增测试，验证取消后不再发 delta：

```rust
#[test]
fn cancelled_turn_skips_delta_emit_payload_generation() {
    // 调用用于构建/发射 delta 的 helper 前先标记 cancelled
    // 断言 helper 返回 cancelled 分支（或 None）
}
```

**Step 2: Run test to verify it fails**

Run:
- `cd src-tauri && cargo test commands::chat::tests::cancelled_turn_stops_before_tool_execution -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::cancelled_turn_skips_delta_emit_payload_generation -- --nocapture`

Expected: FAIL（当前链路未统一检查 cancelled）。

**Step 3: Write minimal implementation**

在 `chat.rs` 主执行路径加入统一 helper：

```rust
async fn ensure_turn_not_cancelled(
    state: &TurnCancellationState,
    turn_id: &str,
) -> Result<(), String> {
    if state.is_cancelled(turn_id).await {
        return Err("turn cancelled by user".to_string());
    }
    Ok(())
}
```

并在以下节点调用：
- LLM 请求前
- 每次 tool 执行前
- tool 执行后结果写回前
- delta 事件发射前

在 turn 完成/错误收尾路径调用 `clear_turn(turn_id)`。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::cancelled_turn_stops_before_tool_execution -- --nocapture`
- `cd src-tauri && cargo test commands::chat::tests::cancelled_turn_skips_delta_emit_payload_generation -- --nocapture`
- `cd src-tauri && cargo test commands::chat --no-run`

Expected: PASS.

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs
git commit -m "feat(chat): gate core pipeline by turn cancellation checkpoints"
```

---

### Task 3: 增加取消命令入口与未知 turn 语义

**Files:**
- Modify: `src-tauri/src/commands/chat.rs`
- Modify: `src/lib/kokoro-bridge.ts`
- Test: `src-tauri/src/commands/chat.rs` (tests)

**Step 1: Write the failing test**

在 `chat.rs` tests 增加：

```rust
#[tokio::test]
async fn cancel_turn_returns_error_for_unknown_turn_id() {
    let state = TurnCancellationState::new();
    let result = state.cancel_turn("not-exists", Some("user".into())).await;
    assert!(result.is_err());
    assert!(result.err().unwrap().contains("unknown turn_id"));
}
```

在前端 bridge 的类型测试/调用点增加（若已有测试模块则放入对应测试）：

```ts
// 调用 cancelChatTurn('turn-1') 应映射到后端 invoke('cancel_chat_turn', ...)
```

**Step 2: Run test to verify it fails**

Run:
- `cd src-tauri && cargo test commands::chat::tests::cancel_turn_returns_error_for_unknown_turn_id -- --nocapture`
- `npx tsc --noEmit`

Expected: FAIL（命令入口与 bridge 尚未对齐）。

**Step 3: Write minimal implementation**

后端 `chat.rs` 增加 tauri command（最小签名）：

```rust
#[tauri::command]
pub async fn cancel_chat_turn(
    turn_id: String,
    reason: Option<String>,
    cancel_state: tauri::State<'_, Arc<TurnCancellationState>>,
) -> Result<(), String> {
    cancel_state.cancel_turn(&turn_id, reason).await
}
```

并在 `src-tauri/src/lib.rs` 注册 command + `app.manage(Arc::new(TurnCancellationState::new()))`。

前端 `kokoro-bridge.ts` 增加：

```ts
export async function cancelChatTurn(turnId: string, reason?: string): Promise<void> {
  await invoke("cancel_chat_turn", { turnId, reason: reason ?? null });
}
```

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test commands::chat::tests::cancel_turn_returns_error_for_unknown_turn_id -- --nocapture`
- `cd src-tauri && cargo check`
- `npx tsc --noEmit`

Expected: PASS.

**Step 5: Commit**

```bash
git add src-tauri/src/commands/chat.rs src-tauri/src/lib.rs src/lib/kokoro-bridge.ts
git commit -m "feat(chat): expose cancel turn command and bridge binding"
```

---

### Task 4: Hook 生命周期补齐（Before/After TTS + EmotionChanged）

**Files:**
- Modify: `src-tauri/src/hooks/types.rs`
- Modify: `src-tauri/src/hooks/handlers.rs`
- Modify: `src-tauri/src/hooks/runtime.rs` (if event match logic needs tests update)
- Modify: `src-tauri/src/hooks/tests.rs`
- Modify: `src-tauri/src/tts/` relevant invocation file(s)
- Modify: `src-tauri/src/ai/emotion.rs` (or actual emotion update file)

**Step 1: Write the failing test**

在 `hooks/tests.rs` 增加：

```rust
#[tokio::test]
async fn audit_handler_subscribes_new_tts_and_emotion_events() {
    let handler = AuditLogHookHandler;
    let events = handler.events();
    assert!(events.contains(&HookEvent::BeforeTtsPlay));
    assert!(events.contains(&HookEvent::AfterTtsPlay));
    assert!(events.contains(&HookEvent::OnEmotionChanged));
}
```

再加一条 runtime 分发测试：

```rust
#[tokio::test]
async fn runtime_dispatches_before_tts_play_event_to_registered_handler() {
    // 注册测试 handler，仅监听 BeforeTtsPlay
    // emit 对应 payload
    // 断言 handler 被调用
}
```

**Step 2: Run test to verify it fails**

Run:
- `cd src-tauri && cargo test hooks::tests::audit_handler_subscribes_new_tts_and_emotion_events -- --nocapture`
- `cd src-tauri && cargo test hooks::tests::runtime_dispatches_before_tts_play_event_to_registered_handler -- --nocapture`

Expected: FAIL（事件枚举未定义或 handler 未订阅）。

**Step 3: Write minimal implementation**

在 `hooks/types.rs` 扩展枚举：

```rust
pub enum HookEvent {
    // existing...
    BeforeTtsPlay,
    AfterTtsPlay,
    OnEmotionChanged,
}
```

复用现有 payload 模型策略（最小改动）：
- TTS 事件可先复用 `HookPayload::Action` 或新增最小 `TtsHookPayload`（二选一，优先更小改动）
- Emotion 事件新增最小 payload（`from`, `to`, `source`）

在 `hooks/handlers.rs` 的 `AuditLogHookHandler::events()` 增加三个事件。

在 TTS 播放入口前后接线 `emit_best_effort`：
- 播放前发 `BeforeTtsPlay`
- 播放后发 `AfterTtsPlay`

在 emotion 状态变更处接线 `emit_best_effort(HookEvent::OnEmotionChanged, ...)`。

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test hooks::tests::audit_handler_subscribes_new_tts_and_emotion_events -- --nocapture`
- `cd src-tauri && cargo test hooks::tests::runtime_dispatches_before_tts_play_event_to_registered_handler -- --nocapture`
- `cd src-tauri && cargo test hooks --no-run`

Expected: PASS.

**Step 5: Commit**

```bash
git add src-tauri/src/hooks/types.rs src-tauri/src/hooks/handlers.rs src-tauri/src/hooks/runtime.rs src-tauri/src/hooks/tests.rs src-tauri/src/tts src-tauri/src/ai
git commit -m "feat(hooks): add tts and emotion lifecycle events"
```

---

### Task 5: 全量验证与文档收口

**Files:**
- Modify: `docs/plans/2026-04-07-turn-cancellation-and-hook-lifecycle-implementation-plan.md`
- Modify: `docs/claude-code-architecture-inspirations.md` (仅在需补“已落地状态”附注时)

**Step 1: Write the failing checklist item**

在计划文档末尾加收口清单：

```md
- [x] turn cancellation state + command 完成
- [x] chat 关键检查点接入取消语义
- [x] hook 新增 2 个 TTS 生命周期事件并接线
- [x] 全量验证通过
```

**Step 2: Run verification to identify failures**

Run:
- `npx tsc --noEmit`
- `npx vitest run src/ui/widgets/chat-history.test.ts`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test hooks --no-run`
- `cd src-tauri && cargo test --lib --no-run`

Expected: 全部通过。

**Step 3: Write minimal implementation/fixes**

只修复导致失败的最小代码，不做额外重构。

**Step 4: Run verification again**

重复 Step 2，直到全部 PASS。

**Step 5: Commit**

```bash
git add docs/plans/2026-04-07-turn-cancellation-and-hook-lifecycle-implementation-plan.md docs/claude-code-architecture-inspirations.md
git commit -m "docs: close cancellation and hook lifecycle plan checklist"
```

---

## 执行注意事项

- 全程遵守 DRY / YAGNI / TDD，不引入企业化审批 UI 或审计数据库。
- commit message 使用英文 Conventional Commits。
- 如遇 Windows 运行时测试异常，优先 `cargo check --tests` 与 `--no-run` 保证编译链稳定。

## 完成判据

- chat turn 可被幂等取消，取消后主链路在关键点稳定中止。
- 取消语义可从前端 bridge 调用后端 command。
- Hook 生命周期新增 `BeforeTtsPlay` / `AfterTtsPlay` 并完成最小接线（情绪系统已移除）。
- 全量验证通过，且未引入额外复杂系统。