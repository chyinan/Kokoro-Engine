# LLM 服务工厂化最小收口 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在不改外部接口的前提下，把 LlmService 收口为统一 provider map + 一致选路模型，降低后续维护成本与语义漂移风险。

**Architecture:** 仅重构 `src-tauri/src/llm/service.rs` 内部状态与构建流程，建立统一 provider map 与 active id。`provider()` 和 `system_provider()` 走同一来源，`update_config()` 采用“先构建、后替换”的原子更新策略。保持命令层和前端 bridge 全量兼容，不引入健康探测或策略路由。

**Tech Stack:** Rust (Tauri v2, tokio, Arc/RwLock), Cargo test/check

---

### Task 1: 为 LlmService 增加 provider map 状态模型

**Files:**
- Modify: `src-tauri/src/llm/service.rs`
- Test: `src-tauri/src/llm/service.rs`（新增 `#[cfg(test)] mod tests` 或复用现有 llm tests）

**Step 1: Write the failing test**

新增测试，验证服务初始化后可从 map 取到 active provider：

```rust
#[tokio::test]
async fn from_config_builds_provider_map_and_returns_active_provider() {
    let (config, path) = test_llm_config_with_two_enabled_providers();
    let service = LlmService::from_config(config, path);

    let provider = service.provider().await;
    assert!(!provider.id().is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test llm::service::tests::from_config_builds_provider_map_and_returns_active_provider -- --nocapture`
Expected: FAIL（当前内部尚未显式 provider map 模型或测试辅助未满足）。

**Step 3: Write minimal implementation**

在 `LlmService` 内部增加：
- `providers: Arc<RwLock<HashMap<String, Arc<dyn LlmProvider>>>>`
- `active_provider_id: Arc<RwLock<String>>`

并在 `from_config(...)` 中：
- 按配置构建 provider map
- 计算 active id（`active_provider` -> first enabled -> first provider）
- 初始化服务状态

**Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test llm::service::tests::from_config_builds_provider_map_and_returns_active_provider -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/llm/service.rs
git commit -m "refactor(llm): store providers map in llm service"
```

---

### Task 2: 统一 provider 选路（provider/system_provider）

**Files:**
- Modify: `src-tauri/src/llm/service.rs`
- Test: `src-tauri/src/llm/service.rs` tests

**Step 1: Write the failing test**

新增两条测试：

```rust
#[tokio::test]
async fn system_provider_prefers_system_provider_id_when_present() {
    let service = make_service_with_active_and_system_provider();
    let provider = service.system_provider().await;
    assert_eq!(provider.id(), "system-provider");
}

#[tokio::test]
async fn system_provider_falls_back_to_active_when_system_missing() {
    let service = make_service_with_missing_system_provider();
    let provider = service.system_provider().await;
    assert_eq!(provider.id(), "active-provider");
}
```

**Step 2: Run test to verify it fails**

Run:
- `cd src-tauri && cargo test llm::service::tests::system_provider_prefers_system_provider_id_when_present -- --nocapture`
- `cd src-tauri && cargo test llm::service::tests::system_provider_falls_back_to_active_when_system_missing -- --nocapture`

Expected: FAIL（当前 `system_provider()` 可能走按需重建路径，行为与 map 模式不一致）。

**Step 3: Write minimal implementation**

在 `system_provider()` 中：
- 优先从 `providers` map 取 `system_provider`
- 若找不到则回退 `active_provider_id`
- `system_model` 覆盖仅做最小临时实例，不写回全局 map

在 `provider()` 中：
- 统一从 map + active id 获取，不再依赖单实例字段

**Step 4: Run test to verify it passes**

Run:
- `cd src-tauri && cargo test llm::service::tests::system_provider_prefers_system_provider_id_when_present -- --nocapture`
- `cd src-tauri && cargo test llm::service::tests::system_provider_falls_back_to_active_when_system_missing -- --nocapture`

Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/llm/service.rs
git commit -m "refactor(llm): unify provider selection via provider map"
```

---

### Task 3: update_config 改为原子更新（先构建后替换）

**Files:**
- Modify: `src-tauri/src/llm/service.rs`
- Test: `src-tauri/src/llm/service.rs` tests

**Step 1: Write the failing test**

新增测试：

```rust
#[tokio::test]
async fn update_config_rebuilds_provider_map_and_switches_active_consistently() {
    let service = make_service_with_initial_config();
    let new_config = make_config_with_different_active_provider();

    service.update_config(new_config).await.unwrap();

    let provider = service.provider().await;
    assert_eq!(provider.id(), "new-active-provider");
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test llm::service::tests::update_config_rebuilds_provider_map_and_switches_active_consistently -- --nocapture`
Expected: FAIL（当前可能仅替换单 provider 状态）。

**Step 3: Write minimal implementation**

在 `update_config(...)` 中：
1. 先保存配置（保持现有行为）
2. 基于新配置构建新的 provider map 与 active id
3. 构建成功后再替换：`providers`、`active_provider_id`、`config`
4. 若构建失败，返回错误并保持旧内存态不变

**Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test llm::service::tests::update_config_rebuilds_provider_map_and_switches_active_consistently -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/llm/service.rs
git commit -m "fix(llm): make config updates rebuild providers atomically"
```

---

### Task 4: 空 provider/无效配置路径的最小错误语义收口

**Files:**
- Modify: `src-tauri/src/llm/service.rs`
- Test: `src-tauri/src/llm/service.rs` tests

**Step 1: Write the failing test**

新增测试：

```rust
#[tokio::test]
async fn returns_explicit_error_when_no_available_provider() {
    let service = make_service_with_no_enabled_provider();
    let result = service.try_provider().await; // 或等价错误路径
    assert!(result.is_err());
}
```

**Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test llm::service::tests::returns_explicit_error_when_no_available_provider -- --nocapture`
Expected: FAIL（当前可能走隐式硬编码 fallback）。

**Step 3: Write minimal implementation**

统一错误策略：
- 当 map 为空或 target id 无法解析到可用 provider 时，返回明确错误消息
- 避免静默 fallback 到与配置无关的硬编码 provider

> 注：若现有调用签名不便直接返回 Result，可保持对外签名不变，内部通过最小兼容路径抛出可观测错误并在测试中验证一致行为。

**Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test llm::service::tests::returns_explicit_error_when_no_available_provider -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src-tauri/src/llm/service.rs
git commit -m "fix(llm): make missing-provider path explicit"
```

---

### Task 5: 全量验证与文档收口

**Files:**
- Modify: `docs/plans/2026-04-07-llm-service-factory-minimal-closure-design.md`（补“实现状态”）
- Optional Modify: `docs/_wip_p0_unified_tool_contract_progress.md`（若你希望同步进度）

**Step 1: Write checklist in design doc**

补充清单：

```md
- [ ] LlmService provider map 模型落地
- [ ] provider/system_provider 统一选路
- [ ] update_config 原子重建与替换
- [ ] 无可用 provider 错误语义明确
```

**Step 2: Run verification**

Run:
- `cd src-tauri && cargo test llm::service -- --nocapture`
- `cd src-tauri && cargo test llm::messages --no-run`
- `cd src-tauri && cargo check`
- `cd src-tauri && cargo check --tests`

Expected: PASS。

**Step 3: Write minimal fixes if needed**

仅修复导致验证失败的最小代码，不扩面。

**Step 4: Re-run verification**

重复 Step 2 至全部通过。

**Step 5: Commit**

```bash
git add docs/plans/2026-04-07-llm-service-factory-minimal-closure-design.md docs/_wip_p0_unified_tool_contract_progress.md src-tauri/src/llm/service.rs
git commit -m "test(llm): close minimal llm service factory stabilization"
```

---

## 执行注意事项

- 严格使用 @superpowers:test-driven-development：每个任务先红后绿。
- 严格 YAGNI：不引入健康探测/策略路由/外部接口变更。
- 本轮按用户要求：不使用 worktree，直接在当前工作区执行。

## 完成判据

- LlmService 内部完成 provider map 化。
- 选路语义统一且可预测。
- 配置更新无中间漂移状态。
- 关键测试与编译验证通过。
