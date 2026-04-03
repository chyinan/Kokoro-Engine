# P2.5 Metadata-driven action policy execution Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 增加第一阶段的 metadata 驱动 action 策略执行能力，让后端可以基于 `ActionInfo` 的 `permission_level` 与 `risk_tags` 拒绝不符合当前策略的工具调用，同时保持现有 hook、deny、modify、前端审批 UI 语义不扩 scope。

**Architecture:** 本阶段不引入审批 UI，也不把策略判断塞进 `HookOutcome`。实现方式是为 `ToolSettings` 增加最小的策略字段，在共享 executor 与 direct execute 路径中于 action resolve 之后、individual enabled 检查之后、args modify 之前执行统一的 policy check；默认策略保持“允许 elevated、无 blocked tags”，确保现有行为默认不变，但配置一旦收紧就能稳定拒绝高权限或高风险 action。

**Tech Stack:** Rust、Tauri v2、serde、现有 `actions/` 模块、`commands/tool_settings.rs`、`kokoro-bridge.ts`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/actions/tool_settings.rs` 已新增 `max_permission_level` 与 `blocked_risk_tags`，并保持默认策略为 `elevated + no blocked tags`
- `src-tauri/src/actions/tool_settings.rs` 已补最小测试，覆盖默认值与 sanitize 去重行为
- `src/lib/kokoro-bridge.ts` 已同步前端 `ToolSettings` 类型
- `src/ui/widgets/settings/McpTab.tsx` 已补齐 `ToolSettings` 默认对象，避免前端本地默认值丢字段
- `src-tauri/src/actions/executor.rs` 已新增统一 `policy_denial_reason(...)` helper，并在共享 executor 中接入 policy check
- `src-tauri/src/commands/actions.rs` 已在 direct execute 路径接入同一 policy helper
- 当前执行顺序已保持为：`hook deny -> resolve -> enabled -> policy -> args modify -> execute`
- policy 拒绝文案已统一为 `Denied by policy: ...`，与 hook deny 区分
- `AfterActionInvoke` 审计路径仍保留，direct execute 的 policy denial 会继续走失败态 after hook

**未完成：**
- 当前只做后端最小策略执行，不做审批 UI / fail-closed / manifest hook
- 前端虽然已同步 `ToolSettings` 类型，但未增加任何策略配置控件
- 当前默认策略保持“允许 elevated、无 blocked tags”，因此默认行为不变；若需要真正控制策略，还需后续补 UI 或配置写入路径
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/actions/tool_settings.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/actions/builtin.rs`
- `src/lib/kokoro-bridge.ts`
- `src/ui/widgets/settings/McpTab.tsx`

### 参考但未改动
- `src-tauri/src/commands/tool_settings.rs`
- `src-tauri/src/hooks/runtime.rs`
- `src-tauri/src/hooks/tests.rs`

---

## 验证记录

本机已完成：
- `cd . && npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
- `cd src-tauri && cargo test actions::tool_settings --no-run`
- `cd src-tauri && cargo test actions::registry --no-run`
- `cd src-tauri && cargo test actions::executor --no-run`
- `cd src-tauri && cargo test commands::actions --no-run`

结果：
- 全部通过
- 运行级 `cargo test ... -- --nocapture` 仍可能因本机已知 Windows 测试二进制问题触发 `STATUS_ENTRYPOINT_NOT_FOUND`
- 因此本轮仍以编译级验证为主，而不是运行级验证

---

## 范围边界

本阶段固定边界如下：
- 只做 metadata 驱动的最小后端策略执行
- 不改 `emit_action_gate(...)` 的 hook deny 语义
- 不改 `emit_before_action_args_modify(...)` 的 modify 语义
- 不做审批 UI / fail-closed / manifest hook
- 不扩 MCP 协议
- 不做 per-tool policy editor UI

---

## 后续顺序

P2.5 收口后，P2 的后续顺序应保持：
1. 先稳定 policy 字段与默认行为
2. 再考虑 fail-closed 与更强审批语义
3. 再考虑前端审批 UI / trace 展示
4. 最后才讨论 manifest hook 与更通用的策略系统
