# P2.6 High-risk action fail-closed Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 增加第一阶段的高风险 action fail-closed 语义，让后端在检测到高权限或高风险工具调用且不满足当前策略时，稳定拒绝执行并保留现有审计链路，同时继续不做前端审批 UI。

**Architecture:** 本阶段不引入前端审批交互，也不把 fail-closed 泛化到所有 hook。实现方式是在 P2.5 已有 metadata-driven policy gate 基础上，明确区分“普通 policy deny”和“高风险 fail-closed deny”的后端语义，只在 action 执行链路内对高风险 action 走更强的拒绝路径与稳定错误文案，保持 hook deny、args modify、AfterActionInvoke 审计与默认行为边界不变。

**Tech Stack:** Rust、Tauri v2、serde、现有 `actions/` 模块、`hooks/` 模块、`commands/actions.rs`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/actions/executor.rs` 已新增 `high_risk_fail_closed_reason(...)` helper，用于区分高风险 fail-closed 拒绝
- `src-tauri/src/actions/executor.rs` 已为高风险 fail-closed 补最小测试，覆盖 elevated ceiling 拒绝、高风险 tag 拒绝、默认 safe action 允许与稳定前缀
- `src-tauri/src/actions/executor.rs` 已在共享 executor 中接入顺序：`hook deny -> resolve -> enabled -> fail-closed -> policy -> args modify -> execute`
- `src-tauri/src/commands/actions.rs` 已在 direct execute 路径接入同一 fail-closed helper
- direct execute 的 fail-closed 仍会继续走 `AfterActionInvoke` 失败态审计路径
- hook deny、普通 policy deny、fail-closed deny 的文案前缀现在已可区分：
  - `Denied by hook: ...`
  - `Denied by policy: ...`
  - `Denied by fail-closed policy: ...`
- 前端仍保持零审批交互改动；当前只沿用既有 trace 展示边界

**未完成：**
- 当前只做高风险动作的后端强语义，不做 pending approval / approve-reject UI
- 不做 HookRuntime 全局 fail-closed，不把 hook handler error 提升成全局中断
- 不做 per-tool 审批界面、manifest hook 或通用策略 DSL
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`

### 参考但未改动
- `src-tauri/src/actions/tool_settings.rs`
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/tests.rs`
- `src/lib/kokoro-bridge.ts`
- `src/ui/widgets/settings/McpTab.tsx`

---

## 验证记录

本机已完成：
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test actions::executor --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`
- `cd src-tauri && cargo test hooks::tests --no-run`

结果：
- 全部通过
- 运行级 `cargo test ... -- --nocapture` 仍可能因本机已知 Windows 测试二进制问题触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- 因此本轮仍以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只做高风险 action 的后端 fail-closed 语义
- 不改 `emit_action_gate(...)` 的 hook deny 语义
- 不改 `emit_before_action_args_modify(...)` 的 modify 语义
- 不做前端审批 UI / pending queue / trace 面板
- 不改 HookRuntime 全局 fail-open 语义
- 不扩 MCP 协议，不做 manifest hook

---

## 后续顺序

P2.6 收口后，P2 的后续顺序应保持：
1. 再考虑 pending approval / 更强审批语义
2. 再考虑前端审批 UI / trace 展示
3. 最后才讨论 manifest hook 与通用策略系统
