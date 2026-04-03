# P2.2 BeforeLlmRequest modify payload Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 的 `BeforeLlmRequest` 增加第一阶段的 modify payload 能力，让 hook 可以在请求发给 LLM 之前，对本轮文本消息载荷做最小、可审计、可测试的修改。

**Architecture:** 本阶段不复用 `HookOutcome`，也不把 modify 语义扩散到 action deny 或 mod lifecycle。实现方式是为 `BeforeLlmRequest` 新增专用 payload 与 runtime modify 分发入口，并在 chat 主链路中于 `compose_prompt(...)` 之后、provider 消息转换之前执行 best-effort 修改，再继续沿用现有消息转换与流式流程。

**Tech Stack:** Rust、Tauri v2、tokio、serde、现有 `hooks/` 模块、`commands/chat.rs`、`ai/context.rs`、`llm/messages.rs`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/hooks/types.rs` 已新增 `BeforeLlmRequestMessage` 与 `BeforeLlmRequestPayload`
- `src-tauri/src/hooks/runtime.rs` 已为 `HookHandler` 增加默认 `modify_before_llm_request(...)`，并新增 `emit_before_llm_request_modify(...)`，保持 handler error 为 fail-open
- `src-tauri/src/hooks/mod.rs` 已导出新的 BeforeLlmRequest payload 类型
- `src-tauri/src/hooks/tests.rs` 已补最小测试，覆盖 modify 顺序、request_message/messages 承载与 handler error 不阻断后续修改
- `src-tauri/src/commands/chat.rs` 已将 `BeforeLlmRequest` modify 接到 `compose_prompt(...)` 之后、provider 消息转换之前
- `src-tauri/src/commands/chat.rs` 在 hidden 请求路径下，已改为使用修改后的 `request_message` 追加 `user_text_message(...)`
- 现有 `BeforeLlmRequest` 观测型 hook 仍保留，并继续发送修改后的 message 文本用于审计/观测
- P2.1 的 `emit_action_gate(...)`、action deny 语义与 actions 路径未被扩 scope

**未完成：**
- `BeforeLlmRequest` 目前只支持文本消息 modify，不支持 vision/image payload 修改
- 未扩展到 chat deny、mod deny、tool args modify、Telegram、前端审批 UI、manifest hook 或全局 fail-closed
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/hooks/types.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/mod.rs`
- `src-tauri/src/hooks/tests.rs`
- `src-tauri/src/commands/chat.rs`

### 参考但未改动
- `src-tauri/src/ai/context.rs`
- `src-tauri/src/llm/messages.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`
- `src-tauri/src/mods/manager.rs`

---

## 验证记录

本机已完成：
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test hooks::tests --no-run`
- `cd src-tauri && cargo test commands::chat --no-run`
- `cd src-tauri && cargo test actions:: --no-run`

结果：
- 全部通过
- 由于本机已知 Windows 测试二进制启动问题，本轮以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只修改 `BeforeLlmRequest` 的文本消息 payload
- 不修改 `HookOutcome`
- 不改 `emit_action_gate(...)` 的 deny 语义
- 不改 action/mod 生命周期行为
- 不改 vision/image 注入流程，只保证它们继续在 modify 之后按原逻辑执行

---

## 后续顺序

P2.2 收口后，下一个顺序应保持：
1. 稳定 `BeforeLlmRequest` modify payload
2. 再考虑受限的 tool args modify
3. 再引入风险标签 / 权限级别
4. 最后才讨论 fail-closed 与前端审批 UI
