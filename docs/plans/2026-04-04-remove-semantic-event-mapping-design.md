# 移除 Semantic Event Mapping（设计文档）

## 背景
情感系统已删除，`ModelTab` 中的 **Semantic Event Mapping** 区块（非 `interaction:*` 语义映射）已成为冗余功能，需从前端彻底清理。

## 目标
1. 删除前端 Semantic Event Mapping 区块及关联逻辑。
2. 加载模型配置时自动清理 `semantic_cue_map` 中所有非 `interaction:` 键。
3. 清理中文文案中已无引用的 semantic 相关键。

## 非目标
- 不改后端数据结构。
- 不改 interaction 映射功能。

## 方案（已确认）
采用“彻底清理（前端 + 文案 key + 相关默认常量）”方案。

### A. 组件逻辑清理（`src/ui/widgets/settings/ModelTab.tsx`）
删除以下内容：
- 常量：`SEMANTIC_KEYS`
- 状态：`semanticKey`、`semanticCue`、`editingSemanticKey`
- 方法：
  - `resetSemanticDraft`
  - `handleSaveSemanticMapping`
  - `handleEditSemanticMapping`
  - `handleDeleteSemanticMapping`
- 派生数据：`generalSemanticEntries`
- 标签函数：`semanticKeyLabel`
- JSX：Semantic Event Mapping 整个区块

保留：
- `interaction:*` 映射的新增/编辑/删除
- `cue_map` 与 interaction 共享的 `persistSemanticMap`

### B. 数据清理策略（仅保留 interaction）
在模型 profile 载入后执行一次规范化：
- 过滤 `semantic_cue_map`，仅保留 `key.startsWith("interaction:")` 的条目。
- 若过滤前后数量不同，则调用 `saveLive2dModelProfile` 写回。
- 将写回后的 profile 更新到本地状态，确保 UI 与持久化一致。

### C. 文案清理（`src/ui/locales/zh.json`）
删除 semantic 专用 key（若无其他引用）：
- `settings.model.mapping.semantic.*`
- `settings.model.mapping.actions.save_semantic`
- `settings.model.mapping.semantic_keys.*`

保留 interaction/cues 仍被引用的 key。

## 验收标准
1. 设置页中不再出现 Semantic Event Mapping 区块。
2. 旧配置加载后，非 `interaction:` 的语义映射被自动清理。
3. interaction 映射功能可正常保存、编辑、删除。
4. 前端无缺失翻译引用导致的报错。

## 最小验证
- 运行针对 `ModelTab` 的测试（如存在），覆盖“渲染无 semantic 区块”与“interaction 正常”。
- 至少执行前端类型检查确保改动安全。