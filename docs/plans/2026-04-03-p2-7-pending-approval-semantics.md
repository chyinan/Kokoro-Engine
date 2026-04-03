# P2.7 Pending approval semantics Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 增加第一阶段的后端审批占位语义，让高风险 action 在命中更严格策略时能够稳定进入“待审批/未批准”拒绝路径，并继续保持前端零审批交互改动。

**Architecture:** 本阶段不做前端审批 UI，也不改 HookRuntime 的全局语义。实现方式是在 P2.6 的 fail-closed 基础上，把“高风险但可审批”的情况从普通 deny / fail-closed 中区分出来，形成统一的后端审批占位结果与稳定文案；执行链仍然只在 action 层处理该语义，并继续保留 `AfterActionInvoke` 审计与现有 hook/modify 顺序。

**Tech Stack:** Rust、Tauri v2、serde、现有 `actions/` 模块、`hooks/` 模块、`commands/actions.rs`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/actions/executor.rs` 已新增 `approval_pending_reason(...)` helper，用于区分审批占位拒绝
- `src-tauri/src/actions/executor.rs` 已将三类后端拒绝语义收敛为互斥路径：
  - 普通 policy deny：低风险 `read/external`
  - pending approval deny：高风险但可审批的 `elevated/write/safe-sensitive`
  - fail-closed deny：更强高风险的 `elevated-sensitive`
- `src-tauri/src/actions/executor.rs` 已在共享 executor 中接入顺序：`hook deny -> resolve -> enabled -> fail-closed -> approval-pending -> policy -> args modify -> execute`
- `src-tauri/src/commands/actions.rs` 已在 direct execute 路径接入同一 `approval_pending_reason(...)` helper
- direct execute 与 shared executor 继续保留 `AfterActionInvoke` 失败态审计路径
- 当前稳定拒绝前缀已可区分：
  - `Denied by hook: ...`
  - `Denied by policy: ...`
  - `Denied by fail-closed policy: ...`
  - `Denied pending approval: ...`
- 前端仍保持零审批交互改动；当前只沿用既有 trace / after hook 展示边界

**未完成：**
- 当前只做后端审批占位语义，不做 pending queue / approve-reject IPC / approval token
- 不做前端审批 UI / trace 面板新增状态展示
- 不改 HookRuntime 全局 fail-open / fail-closed 语义
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`

### 新增
- `docs/plans/2026-04-03-p2-7-pending-approval-semantics.md`

### 参考但未改动
- `src-tauri/src/actions/tool_settings.rs`
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/tests.rs`
- `src/lib/kokoro-bridge.ts`
- `src/ui/widgets/ChatPanel.tsx`

---

## 验证记录

本机已完成：
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test actions::executor --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

结果：
- 全部通过
- 运行级 `cargo test actions::executor -- --nocapture` 仍因本机已知 Windows 测试二进制问题触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- 因此本轮仍以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只做高风险 action 的后端审批占位语义
- 不改 `emit_action_gate(...)` 的 hook deny 语义
- 不改 `emit_before_action_args_modify(...)` 的 modify 语义
- 不做前端审批 UI / pending queue / approve-reject 流程
- 不改 HookRuntime 全局 fail-open 语义
- 不扩 MCP 协议，不做 manifest hook

---

## 后续顺序

P2.7 收口后，P2 的后续顺序应保持：
1. 再考虑前端审批 UI / trace 展示
2. 最后才讨论 manifest hook 与更通用的策略系统
