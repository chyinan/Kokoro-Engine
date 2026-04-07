# UTC 稳定性优先最小收口 Design

## 目标
在不引入审计数据库与复杂前端审计 UI 的前提下，先完成 UTC/P1/P2 的语义一致性收口，降低回归风险；随后补齐 hook policy 与 MOD capabilities 的最小闭环。

## 约束
- 不使用 worktree，直接在当前工作区改动。
- 不做企业化扩面：不新增审批/审计页面、不新增审计入库。
- 保持 DRY / YAGNI，优先最小改动。

## 范围
### In Scope
- 后端：
  - `src-tauri/src/actions/permission.rs`
  - `src-tauri/src/commands/chat.rs`
  - `src-tauri/src/commands/actions.rs`
  - `src-tauri/src/hooks/runtime.rs`
  - `src-tauri/src/hooks/types.rs`
  - `src-tauri/src/mods/manifest.rs`
  - `src-tauri/src/mods/manager.rs`
- 前端最小兼容：
  - `src/ui/widgets/chat-history.ts`
  - `src/ui/widgets/chat-history.test.ts`
- 文档同步：
  - `docs/_wip_p0_unified_tool_contract_progress.md`
  - 本设计文档与后续 implementation plan

### Out of Scope
- 审计数据库（`ToolAuditEvent` 落库等）
- 新审批/审计前端页面
- 大规模重构与新平台化能力系统

## 核心设计决策
1. **权限语义真源统一**：新执行链一律以 `PermissionDecision` 派生 deny kind；错误前缀仅作历史回放 fallback。
2. **审批状态机幂等**：同一 request 只允许 resolve 一次；二次 resolve 返回一致错误。
3. **Hook modify 策略可控**：引入 strict/permissive；核心链路走 strict，失败 fail-closed。
4. **MOD 最小声明式能力模型**：manifest 增加 `capabilities[]` 最小字段，保留 `permissions[]` 兼容。
5. **前端最小兼容**：`metadata.deny_kind` 优先，`error` 前缀仅 fallback；不新增 UI 结构。

## 实施顺序（风险优先）
1. 后端 deny 结构化真源收口（permission/chat/actions）
2. pending approval 幂等与非法迁移保护（chat）
3. 前端最小兼容与回归测试（chat-history）
4. hook strict/permissive 最小接线（hooks + chat/actions）
5. mod capabilities 最小模型（manifest/manager）
6. 全量验证与进度文档收口

## 验证标准
- 新执行链业务语义不再依赖错误字符串前缀。
- pending approval 二次 resolve 被拒绝且语义一致。
- 前端优先消费结构化 `deny_kind`，旧数据可回放。
- hook strict/permissive 在核心链路生效。
- MOD capabilities 可解析且保持 permissions 兼容。
- `npx tsc --noEmit`、关键 vitest、`cargo check`、`cargo check --tests`、关键 `--no-run` 通过。

## 收口检查清单
- [x] deny kind 结构化优先已落地
- [x] pending approval 幂等已落地
- [x] direct action 语义与 PermissionDecision 一致
- [x] 前端 deny_kind 最小兼容已落地
- [x] hook strict/permissive 已接线
- [x] mod capabilities 最小模型已落地
