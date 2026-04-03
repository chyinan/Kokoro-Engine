# P2.8 Approval trace semantic display Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 增加第一阶段的前端审批 trace 展示，让聊天面板能够稳定区分并展示 hook deny、policy deny、fail-closed deny、pending approval 四类工具结果语义。

**Architecture:** 本阶段优先做“可观察、可读”的 trace 展示，而不做真正的 approve/reject 交互。实现方式是在现有 `chat-turn-tool` 事件上新增最小机器可读字段 `deny_kind`，并将前端当前的纯文本 tool trace 升级为结构化 trace item，在 `ChatPanel`、`ChatMessage`、`chat-history` 三处统一消费；执行链和 HookRuntime 全局语义保持不变。

**Tech Stack:** React、TypeScript、Tauri v2 event bridge、Rust、现有 `chat-turn-tool` 事件链路

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/commands/chat.rs` 已新增 `deny_kind_for_tool_error(...)` helper
- `src-tauri/src/commands/chat.rs` 的 `chat-turn-tool` 失败事件已附带稳定 `deny_kind`
- `src/lib/kokoro-bridge.ts` 已新增 `ToolTraceItem`，并扩展 `ChatTurnToolEvent`
- `src/ui/widgets/ChatPanel.tsx` 已将实时 tool trace 从纯文本升级为结构化 `ToolTraceItem`
- `src/ui/widgets/ChatMessage.tsx` 已按 `denyKind` 展示语义标签：Allowed / Hook denied / Policy denied / Fail-closed / Pending approval / Error
- `src/ui/widgets/chat-history.ts` 已与实时 trace 对齐，历史记录可回退解析 deny 前缀为相同语义
- 当前实时与历史两条链都能区分：
  - `Denied by hook: ...`
  - `Denied by policy: ...`
  - `Denied by fail-closed policy: ...`
  - `Denied pending approval: ...`

**未完成：**
- 当前只做前端语义展示，不做 approve/reject 真交互
- 不做 pending queue、approval token、approve-reject IPC
- 不改 settings UI，不做 per-tool 审批配置控件
- 运行级 Rust 测试在本机仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/commands/chat.rs`
- `src/lib/kokoro-bridge.ts`
- `src/ui/widgets/ChatPanel.tsx`
- `src/ui/widgets/ChatMessage.tsx`
- `src/ui/widgets/chat-history.ts`

### 新增
- `docs/plans/2026-04-03-p2-8-approval-trace-semantic-display.md`

### 参考但未改动
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`
- `src-tauri/src/hooks/tests.rs`
- `src/ui/widgets/settings/McpTab.tsx`

---

## 验证记录

本机已完成：
- `cd . && npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test commands::chat --no-run`

待本阶段收尾执行：
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test commands::actions --no-run`
- `cd src-tauri && cargo test hooks::tests --no-run`

说明：
- 运行级 `cargo test ... -- --nocapture` 在本机仍可能因已知 Windows 测试二进制问题触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- 因此本轮仍以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只做前端审批 trace 语义展示
- 不做 approve/reject 真交互
- 不做 pending queue / approval token / IPC 流程
- 不改 action 执行核心语义
- 不改 HookRuntime 全局 fail-open / fail-closed 语义
- 不做 manifest hook / 通用审批状态机 / 通用策略系统

---

## 后续顺序

P2.8 收口后，P2 的后续顺序应保持：
1. 再考虑真正的前端审批交互（approve/reject）
2. 再考虑更完整的 trace 面板或审批上下文展示
3. 最后才讨论 manifest hook 与更通用的策略系统
