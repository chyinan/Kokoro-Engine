# P2.4 Action risk tags / permission levels metadata Progress

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Kokoro Engine 的 action/tool 元数据增加第一阶段的风险标签与权限级别字段，让后端、前端与后续 hook / 审批 / fail-closed 演进都有统一、稳定的元数据基础，但本阶段不改变执行行为。

**Architecture:** 本阶段只扩展 `ActionInfo` 及其构造链，不把风险/权限语义接进 gate、deny、modify 或设置存储。实现方式是为 `ActionInfo` 增加最小的 `risk_tags` 与 `permission_level` 字段，在 `ActionRegistry::make_action_info(...)` 统一构造，builtin 通过 `ActionHandler` 默认方法提供元数据，MCP tool 先使用保守默认值，再同步前端 `kokoro-bridge.ts` 类型与最小单测，保持 chat / direct execute / executor 行为完全不变。

**Tech Stack:** Rust、Tauri v2、serde、现有 `actions/` 模块、`mcp/bridge.rs`、`kokoro-bridge.ts`

---

## 当前实现状态（2026-04-03）

**已完成：**
- `src-tauri/src/actions/registry.rs` 已新增 `ActionRiskTag` 与 `ActionPermissionLevel` 枚举
- `src-tauri/src/actions/registry.rs` 已为 `ActionInfo` 增加 `risk_tags` 与 `permission_level` 字段
- `src-tauri/src/actions/registry.rs` 已为 `ActionHandler` 增加默认 metadata 方法，并在 `make_action_info(...)` 统一构造 metadata
- builtin 与 MCP action 现在都会稳定携带 metadata；MCP 仍使用保守默认值，不扩 MCP 协议
- `src-tauri/src/actions/builtin.rs` 已为最关键的代表性 builtin 补最小分类：
  - `search_memory` 仍走默认 `read/safe`
  - `play_cue` 与 `set_background` 已标为 `write/elevated`
- `src/lib/kokoro-bridge.ts` 已同步前端 `ActionInfo` 类型，暴露 `risk_tags` 与 `permission_level`
- `src-tauri/src/actions/executor.rs` 与 `src-tauri/src/commands/actions.rs` 仅更新测试构造数据；执行链行为保持不变

**未完成：**
- 当前只加 metadata，不做 executor/direct execute 的权限门禁
- `ToolSettings` 仍不承载权限策略配置
- fail-closed、审批 UI、manifest hook、前端展示面板仍未开始
- builtin 目前只做最小代表性分类，不代表全量 action 已完成精细分级
- 尚未在本机完成真实运行级 Rust 测试；本机运行测试二进制仍可能触发 `STATUS_ENTRYPOINT_NOT_FOUND`

---

## 本阶段涉及文件

### 修改
- `src-tauri/src/actions/registry.rs`
- `src-tauri/src/actions/builtin.rs`
- `src-tauri/src/actions/executor.rs`
- `src-tauri/src/commands/actions.rs`
- `src/lib/kokoro-bridge.ts`

### 参考但未改动
- `src-tauri/src/mcp/bridge.rs`
- `src-tauri/src/mcp/client.rs`
- `src-tauri/src/actions/tool_settings.rs`
- `src-tauri/src/commands/tool_settings.rs`

---

## 验证记录

本机已完成：
- `cd . && npx tsc --noEmit`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo clippy -- -D warnings`
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
- 只增加 action metadata，不改变执行行为
- 不改 `emit_action_gate(...)` 的 deny 语义
- 不改 `emit_before_action_args_modify(...)` 的 modify 语义
- 不改 `ToolSettings` 结构
- 不扩 MCP 协议，不要求 server 直接声明风险/权限字段
- 不做审批 UI、fail-closed 或权限门禁

---

## 后续顺序

P2.4 收口后，P2 的后续顺序应保持：
1. 先稳定 metadata 词表与 builtin 基线分类
2. 再考虑真正的策略执行（risk tags / permission levels 如何影响 gate）
3. 再考虑 fail-closed 与更细粒度审批
4. 最后才讨论前端审批 UI 与 manifest hook
