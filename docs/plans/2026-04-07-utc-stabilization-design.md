# UTC 稳定性收口设计（不含审计数据库）

## 1. 背景与目标

当前 P0（统一工具契约）与 P2（HookRuntime / 审批语义）主链路已落地，但仍存在过渡层语义与实现不完全收口的问题。为了在进入下一阶段前建立稳定基线，本次收口聚焦“后端鲁棒性与语义一致性”，不引入企业化审计体系。

本次目标：

1. 权限与审批判定彻底结构化，执行链不再依赖字符串前缀语义。
2. 审批状态机在 chat / telegram / direct action 三入口保持一致。
3. 保持最小可观测（日志/trace）即可，不新增审计数据库持久化。
4. 前端维持现有权限提醒，仅做必要字段兼容，不增加复杂审计 UI。
5. 补齐回归测试矩阵，确保收口后行为稳定可验证。

---

## 2. 范围与非目标

### 2.1 范围

- Rust 后端：`actions/permission.rs`、`actions/executor.rs`、`commands/chat.rs`、`commands/actions.rs`、`telegram/bot.rs`。
- 运行时语义：权限决策、pending approval、审批结果处理、执行结果回写。
- 前端最小范围：bridge 类型与 trace 展示的兼容对齐（不做新页面）。
- 测试：权限与审批全路径回归。

### 2.2 非目标

- 不新增审计数据库表与 migration。
- 不新增审计管理页、筛选器、统计面板。
- 不在本轮扩展批量审批、审批超时策略等 P3 议题。

---

## 3. 核心设计决策

### 决策 A：`PermissionDecision` 作为唯一业务语义真源

- 执行链所有分支只消费结构化 `PermissionDecision`。
- 文本前缀（如 `pending_approval:` / `denied:` 风格）仅保留为“历史展示兼容”，不得再驱动执行分支。

### 决策 B：统一审批状态机

统一状态流：

- `requested`
- `approved` 或 `rejected`
- `executed`（批准后执行成功）或 `aborted`（拒绝/异常中止）

三入口（chat / telegram / direct action）使用同一转移规则，不允许各自定义隐式语义。

### 决策 C：最小可观测而非持久化审计

- 工具调用生命周期继续记录在运行期日志与 trace 中。
- 保障调试与回放所需字段可见即可。
- 不引入额外存储复杂度与维护成本。

### 决策 D：前端最小改动策略

- 保留现有权限提醒体验。
- 仅做结构化字段消费优先级调整与 fallback 兼容。
- 不引入企业化审计交互。

---

## 4. 分模块改造方案

### 4.1 `actions/permission.rs`

- 固化 `PermissionDecision` 枚举语义（allow / deny / requires_approval）。
- 对未知或无效决策统一映射为拒绝执行（fail-closed）。
- 明确输出字段，避免下游解析自由发挥。

### 4.2 `actions/executor.rs`

- 删除/隔离对字符串前缀的执行语义依赖。
- 统一执行入口处理：
  1) 接收结构化决策
  2) 若需审批则挂起
  3) 审批通过后执行
  4) 审批拒绝或异常则中止
- 执行结果与审批结果都以结构化事件写入 trace/log。

### 4.3 `commands/chat.rs` / `commands/actions.rs` / `telegram/bot.rs`

- 统一 canonical `tool_id` 解析与传递。
- 对 pending approval 的触发、确认、拒绝流程采用同一语义。
- 保证同一 request 在重复审批时具备幂等行为（第一次有效，后续忽略并返回一致响应）。

### 4.4 前端 bridge 与 trace

- `kokoro-bridge.ts` 优先消费结构化字段。
- 历史消息仍允许旧字段 fallback 展示，避免回放断裂。
- 仅修正语义一致性，不新增审计 UI。

---

## 5. 数据流（收口后）

1. 用户触发工具调用（chat / telegram / direct）。
2. 解析 canonical `tool_id`。
3. 计算 `PermissionDecision`。
4. 若为 `requires_approval`：写入 pending 状态并等待审批。
5. 审批结果：
   - approved -> 执行 -> executed / failed
   - rejected -> aborted
6. 运行期 trace/log 记录关键节点，前端按结构化字段展示。

---

## 6. 错误处理与鲁棒性策略

1. **Fail-closed**：
   - 决策缺失、状态非法、入口语义不一致时统一拒绝执行。

2. **审批幂等**：
   - 同一 request 重复 approve/reject 不得造成重复执行。

3. **兼容层隔离**：
   - 旧前缀文案仅读取、不写入、不参与业务判定。

4. **主链路稳定优先**：
   - 非关键展示异常不影响执行链。

---

## 7. 测试策略

### 7.1 单元测试

- `PermissionDecision` 映射与边界值。
- 审批状态机合法/非法转移。
- 幂等审批行为。

### 7.2 集成测试

覆盖三入口全路径：

- allow
- deny
- pending -> approve -> execute
- pending -> reject -> abort

并校验 trace 语义字段完整性与一致性。

### 7.3 前端测试（最小）

- 结构化字段优先展示。
- 旧字段 fallback 不破坏历史回放。
- 现有权限提醒交互不回归。

---

## 8. 完成定义（DoD）

满足以下条件视为收口完成：

1. 执行链不再依赖字符串前缀做权限/审批判定。
2. 三入口审批语义一致且幂等。
3. 前端权限提醒保持现状可用，trace 展示兼容新旧数据。
4. 回归测试通过，编译/类型检查通过。
5. 文档与实际实现一致。

---

## 9. 风险与应对

### 风险

- 历史回放依赖旧字段导致展示异常。
- 收口后暴露此前被前缀兼容掩盖的边缘分支。

### 应对

- 保留只读 fallback 展示路径。
- 增加针对历史样本的回放测试。
- 收口阶段优先修复语义分歧，再做功能扩展。

---

## 10. 后续衔接

本设计完成后，下一步进入实现计划编写（细化到可执行步骤与文件级变更清单），并按“后端鲁棒性优先、前端最小改动”原则实施。

---

## 11. 实现完成状态

- [x] chat deny kind 结构化优先
- [x] pending approval 幂等
- [x] direct action 权限拒绝语义一致
- [x] 前端 deny kind 最小兼容