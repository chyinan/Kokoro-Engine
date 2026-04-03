# P2.3 Restricted tool args modify Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 增加第一阶段的受限 tool args modify 能力，让 hook 可以在 action 真正执行前，对已解析的字符串参数做最小、可审计、可测试的修改。

**Architecture:** 本阶段不复用 `HookOutcome`，也不改变 P2.1 的 `BeforeActionInvoke` deny 语义。实现方式是为 action 执行链路新增专用的可变 args payload 与 runtime modify 分发入口，并在 `emit_action_gate(...)`、action resolve、enabled 检查通过之后、`handler.execute(...)` / `registry.execute(...)` 之前执行串行 best-effort 修改，再继续沿用现有 `AfterActionInvoke` 审计与结果回传流程。

**Tech Stack:** Rust、Tauri v2、tokio、serde、现有 `hooks/` 模块、`actions/executor.rs`、`commands/actions.rs`、`actions/registry.rs`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/hooks/types.rs` 已新增 `BeforeActionArgsPayload`
- `src-tauri/src/hooks/runtime.rs` 已为 `HookHandler` 增加默认 `modify_before_action_args(...)`，并新增 `emit_before_action_args_modify(...)`，保持 handler error 为 fail-open
- `src-tauri/src/hooks/mod.rs` 已导出新的 `BeforeActionArgsPayload`
- `src-tauri/src/hooks/tests.rs` 已补最小测试，覆盖 payload 承载、modify 顺序、handler error 不阻断后续修改，以及 gate deny 语义不回归
- `src-tauri/src/actions/executor.rs` 已新增 `build_before_action_args_payload(...)` 与 `apply_before_action_args_payload(...)`
- `src-tauri/src/actions/executor.rs` 已将共享 `execute_tool_calls(...)` 接到 args modify 通道，且 modify 发生在 gate / resolve / enabled 之后、execute 之前
- `src-tauri/src/commands/actions.rs` 已将 `execute_action(...)` 对齐到同一 args modify 语义
- chat/native tool-calling/Telegram 因复用 `execute_tool_calls(...)` 自动覆盖；direct execute 已单独对齐
- P2.1 的 `emit_action_gate(...)`、deny 错误文案与短路路径未被扩 scope

**未完成：**
- 当前只支持字符串 `HashMap<String, String>` args modify，不支持 typed args / schema-level patch
- 未扩展到通用 `ReplacePayload`、前端审批 UI、manifest hook、fail-closed 或更高阶策略系统
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- P2 后续阶段（风险标签 / 权限级别、mod lifecycle 后续扩展、fail-closed、前端审批 UI）仍未开始

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/hooks/types.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/mod.rs`
- `src-tauri/src/hooks/tests.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`

### 参考但未改动
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/actions/builtin.rs`
- `src-tauri/src/commands/chat.rs`
- `src-tauri/src/llm/provider.rs`
- `src-tauri/src/telegram/bot.rs`

---

## 验证记录

本机已完成：
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test hooks::tests --no-run`
- `cd src-tauri && cargo test actions::executor --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

结果：
- 全部通过
- 运行级 `cargo test ... -- --nocapture` 仍可能因本机已知 Windows 测试二进制问题触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- 因此本轮仍以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只修改 action execute 前的字符串 args payload
- 不修改 `HookOutcome`
- 不改 `emit_action_gate(...)` 的 deny 语义
- 不改 `ActionContext` 结构
- 不改原始 tool tag / native tool call 解析协议
- 不改 Telegram 专属入口逻辑，只复用共享 executor 覆盖

---

## 后续顺序

P2.3 收口后，P2 的后续顺序应保持：
1. 稳定受限的 tool args modify
2. 再引入风险标签 / 权限级别
3. 再考虑更细粒度审批与 fail-closed
4. 最后才讨论前端审批 UI 与更通用的 payload patch 系统
