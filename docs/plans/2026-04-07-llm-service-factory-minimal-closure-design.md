# LLM 服务工厂化最小收口 Design

## 目标
在不扩面、不改外部接口的前提下，将 `LlmService` 从“单 active provider + 按需重建”收口为“统一 provider map + 一致选路”，降低后续维护成本与语义漂移风险。

## 范围与约束
- 仅改动：`src-tauri/src/llm/service.rs`（必要时补充同目录测试）
- 不改 Tauri 命令层接口
- 不改前端 bridge / UI
- 不引入健康探测、策略路由、审计扩展

## 当前问题
`LlmService` 当前主状态仅持有一个 active provider，`system_provider()` 需要按需重建 provider，导致：
- provider 选路逻辑分散
- update 后行为一致性难保证
- 后续扩展多 provider 管理成本高

## 设计方案（方案 A）

### 1) 内部状态重构（外部接口保持不变）
将 `LlmService` 内部状态调整为：
- `providers: Arc<RwLock<HashMap<String, Arc<dyn LlmProvider>>>>`
- `active_provider_id: Arc<RwLock<String>>`
- `config: Arc<RwLock<LlmConfig>>`
- `config_path: PathBuf`

### 2) 统一 provider 工厂与选路
在 `service.rs` 内部新增统一构建逻辑：
- 从 `LlmConfig.providers` 构建 provider map（仅 enabled；空时按现有兜底策略处理）
- 计算 active id：`active_provider` -> 首个 enabled -> 首个 provider
- 复用同一套规则给 `from_config()` 与 `update_config()`

### 3) 调用路径语义
- `provider()`：按 `active_provider_id` 从 map 直接返回
- `system_provider()`：按 `system_provider`（缺省回 active）从 map 返回
- `system_model` 覆盖仅在必要时做最小临时构建，不污染全局 map

### 4) update_config 原子替换策略
- 先基于新配置完整构建新 map + 新 active id
- 构建成功后再替换内存状态并更新 config
- 构建失败时保持旧运行态可用

## 错误处理策略
1. active id 无效：回退首个可用 provider；若无可用 provider，返回明确错误
2. system id 无效：回退 active provider
3. update_config 构建失败：不替换旧状态
4. system_model 覆盖失败：记录 warning 并回退已有 provider 实例

## 实现状态
- [x] LlmService provider map 模型落地
- [x] provider/system_provider 统一选路
- [x] update_config 原子重建与替换
- [x] 无可用 provider 错误语义明确

## 测试与验收

### 最小测试集
- `from_config` 构建 provider map 且 active 可取
- `system_provider` 命中 system id；system 不存在时回退 active
- `update_config` 后 map 与 active 一致更新
- providers 不可用场景返回明确错误或一致兜底行为

### 完成判据
- `LlmService` 不再依赖“按需重建 provider”作为常规路径
- 外部接口与调用方保持不变
- 关键路径测试通过
